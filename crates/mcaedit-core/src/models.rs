//! Vanilla blockstate + block model baking (offline view).
//!
//! Architecture follows Arcus92/minecraft-web-exporter (MIT):
//! `blockstates/*.json` → variants/multipart → merged `models/block/*.json`
//! elements → cached quads with UV + cullface + variant x/y/uvlock rotation.
//!
//! Textures resolve `#var` maps; animated PNGs use first frame via [`crate::assets`].

use crate::assets::BlockTextureAtlas;
use crate::blockstate::BlockState;
use glam::{Vec2, Vec3};
use image::RgbaImage;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceDir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl FaceDir {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "down" => Some(Self::Down),
            "up" => Some(Self::Up),
            "north" => Some(Self::North),
            "south" => Some(Self::South),
            "west" => Some(Self::West),
            "east" => Some(Self::East),
            _ => None,
        }
    }

    pub fn offset(self) -> (i32, i32, i32) {
        match self {
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
        }
    }

    fn invert(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    fn normal(self) -> Vec3 {
        match self {
            Self::Down => Vec3::new(0.0, -1.0, 0.0),
            Self::Up => Vec3::new(0.0, 1.0, 0.0),
            Self::North => Vec3::new(0.0, 0.0, -1.0),
            Self::South => Vec3::new(0.0, 0.0, 1.0),
            Self::West => Vec3::new(-1.0, 0.0, 0.0),
            Self::East => Vec3::new(1.0, 0.0, 0.0),
        }
    }
}

/// One baked quad in block-local [0,1] space (web-exporter CachedBlockStateFace).
#[derive(Clone, Debug)]
pub struct BakedFace {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub d: Vec3,
    pub uv_a: Vec2,
    pub uv_b: Vec2,
    pub uv_c: Vec2,
    pub uv_d: Vec2,
    pub normal: Vec3,
    /// Texture path like `minecraft:block/oak_planks` (no `#`).
    pub texture: String,
    pub cull: Option<FaceDir>,
    pub tint: bool,
    /// Element `shade` (default true). Cross plants set false.
    pub shade: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BakedBlock {
    pub faces: Vec<BakedFace>,
    /// True when model is a full solid cube (fast opaque cull).
    pub full_cube: bool,
}

/// Jar-backed blockstate/model catalog + texture resolver.
pub struct ModelCatalog {
    path: PathBuf,
    blockstates: HashMap<String, JsonValue>,
    models: HashMap<String, JsonValue>,
    baked: HashMap<String, BakedBlock>,
    atlas: BlockTextureAtlas,
    tex_images: HashMap<String, Option<RgbaImage>>,
}

impl ModelCatalog {
    pub fn open(path: impl AsRef<Path>) -> crate::error::Result<Self> {
        let atlas = BlockTextureAtlas::open(path.as_ref())?;
        Ok(Self {
            path: atlas.path().to_path_buf(),
            blockstates: HashMap::new(),
            models: HashMap::new(),
            baked: HashMap::new(),
            atlas,
            tex_images: HashMap::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn atlas_mut(&mut self) -> &mut BlockTextureAtlas {
        &mut self.atlas
    }

    /// Load (cache) texture by resource id `minecraft:block/stone` or `block/stone`.
    pub fn texture_image(&mut self, tex: &str) -> Option<&RgbaImage> {
        if !self.tex_images.contains_key(tex) {
            let img = load_texture_from_jar(&self.path, tex);
            self.tex_images.insert(tex.to_string(), img);
        }
        self.tex_images.get(tex).and_then(|o| o.as_ref())
    }

    pub fn texture_image_owned(&mut self, tex: &str) -> Option<RgbaImage> {
        self.texture_image(tex).cloned()
    }

    /// Bake faces for a full block state compact id.
    pub fn bake_block(&mut self, state: &BlockState) -> BakedBlock {
        let key = state.to_compact();
        if let Some(b) = self.baked.get(&key) {
            return b.clone();
        }
        let baked = self.bake_block_uncached(state);
        self.baked.insert(key, baked.clone());
        baked
    }

    fn bake_block_uncached(&mut self, state: &BlockState) -> BakedBlock {
        let stem = block_stem(&state.name);
        let Some(bs) = self.load_blockstate(stem).cloned() else {
            return BakedBlock::default();
        };
        let mut faces = Vec::new();
        if let Some(variants) = bs.get("variants").and_then(|v| v.as_object()) {
            if let Some(entries) = match_variants(variants, &state.properties) {
                for entry in entries {
                    self.append_variant_faces(entry, &mut faces);
                }
            }
        }
        if let Some(multipart) = bs.get("multipart").and_then(|v| v.as_array()) {
            for part in multipart {
                if when_matches(part.get("when"), &state.properties) {
                    if let Some(apply) = part.get("apply") {
                        for entry in variant_entries(apply) {
                            self.append_variant_faces(entry, &mut faces);
                        }
                    }
                }
            }
        }
        let full_cube = is_full_cube_heuristic(&faces);
        BakedBlock { faces, full_cube }
    }

    fn append_variant_faces(&mut self, entry: &JsonValue, out: &mut Vec<BakedFace>) {
        let Some(obj) = entry.as_object() else {
            return;
        };
        let Some(model_name) = obj.get("model").and_then(|m| m.as_str()) else {
            return;
        };
        let x = obj.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let y = obj.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let uvlock = obj
            .get("uvlock")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let model_faces = self.bake_model(model_name);
        for face in model_faces {
            out.push(rotate_face(face, x, y, uvlock));
        }
    }

    fn bake_model(&mut self, model_name: &str) -> Vec<BakedFace> {
        let Some(merged) = self.merge_model(model_name) else {
            return Vec::new();
        };
        let textures = merged
            .get("textures")
            .and_then(|t| t.as_object())
            .cloned()
            .unwrap_or_default();
        let Some(elements) = merged.get("elements").and_then(|e| e.as_array()) else {
            return Vec::new();
        };
        let mut faces = Vec::new();
        for el in elements {
            let shade = el
                .get("shade")
                .and_then(|s| s.as_bool())
                .unwrap_or(true);
            let from = json_vec3(el.get("from")).unwrap_or(Vec3::ZERO);
            let to = json_vec3(el.get("to")).unwrap_or(Vec3::splat(16.0));
            let rotation = el.get("rotation");
            let Some(face_map) = el.get("faces").and_then(|f| f.as_object()) else {
                continue;
            };
            for (dir_name, face_json) in face_map {
                let Some(dir) = FaceDir::parse(dir_name) else {
                    continue;
                };
                if let Some(face) =
                    bake_element_face(from, to, rotation, dir, face_json, &textures, shade)
                {
                    faces.push(face);
                }
            }
        }
        faces
    }

    fn merge_model(&mut self, name: &str) -> Option<JsonValue> {
        let raw = self.load_model(name)?.clone();
        let parent = raw
            .get("parent")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let Some(parent_name) = parent else {
            return Some(raw);
        };
        let parent_merged = self.merge_model(&parent_name)?;
        Some(merge_model_json(raw, parent_merged))
    }

    fn load_blockstate(&mut self, stem: &str) -> Option<&JsonValue> {
        if !self.blockstates.contains_key(stem) {
            let path = format!("assets/minecraft/blockstates/{stem}.json");
            let v = read_json_from_jar(&self.path, &path);
            self.blockstates.insert(stem.to_string(), v.unwrap_or(JsonValue::Null));
        }
        self.blockstates.get(stem).filter(|v| !v.is_null())
    }

    fn load_model(&mut self, name: &str) -> Option<&JsonValue> {
        let key = normalize_model_name(name);
        if !self.models.contains_key(&key) {
            let path = format!("assets/minecraft/models/{key}.json");
            let v = read_json_from_jar(&self.path, &path);
            self.models.insert(key.clone(), v.unwrap_or(JsonValue::Null));
        }
        self.models.get(&key).filter(|v| !v.is_null())
    }
}

fn block_stem(name: &str) -> &str {
    name.rsplit_once(':').map(|(_, n)| n).unwrap_or(name)
}

fn normalize_model_name(name: &str) -> String {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if n.starts_with("block/") || n.starts_with("item/") {
        n.to_string()
    } else {
        format!("block/{n}")
    }
}

fn read_json_from_jar(jar: &Path, entry: &str) -> Option<JsonValue> {
    let file = File::open(jar).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut zf = zip.by_name(entry).ok()?;
    let mut buf = String::new();
    zf.read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

fn load_texture_from_jar(jar: &Path, tex: &str) -> Option<RgbaImage> {
    let path = texture_jar_path(tex)?;
    let file = File::open(jar).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut zf = zip.by_name(&path).ok()?;
    let mut buf = Vec::with_capacity(zf.size() as usize);
    zf.read_to_end(&mut buf).ok()?;
    let img = image::load_from_memory(&buf).ok()?.to_rgba8();
    Some(crate::assets::first_animation_frame_pub(img))
}

fn texture_jar_path(tex: &str) -> Option<String> {
    let t = tex.strip_prefix("minecraft:").unwrap_or(tex);
    let t = t.strip_prefix("#").unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    // Already `block/foo` or `item/foo`
    Some(format!("assets/minecraft/textures/{t}.png"))
}

fn merge_model_json(child: JsonValue, parent: JsonValue) -> JsonValue {
    let mut out = parent;
    let child_obj = match child.as_object() {
        Some(o) => o,
        None => return out,
    };
    let out_obj = out.as_object_mut().unwrap();
    if let Some(v) = child_obj.get("gui_light") {
        out_obj.insert("gui_light".into(), v.clone());
    }
    if let Some(v) = child_obj.get("display") {
        out_obj.insert("display".into(), v.clone());
    }
    if let Some(v) = child_obj.get("ambientocclusion") {
        out_obj.insert("ambientocclusion".into(), v.clone());
    }
    if let Some(v) = child_obj.get("elements") {
        out_obj.insert("elements".into(), v.clone());
    }
    // textures: parent then child override
    let mut tex = out_obj
        .get("textures")
        .and_then(|t| t.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(ct) = child_obj.get("textures").and_then(|t| t.as_object()) {
        for (k, v) in ct {
            tex.insert(k.clone(), v.clone());
        }
    }
    out_obj.insert("textures".into(), JsonValue::Object(tex));
    out
}

fn json_vec3(v: Option<&JsonValue>) -> Option<Vec3> {
    let a = v?.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some(Vec3::new(
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ))
}

fn resolve_texture(textures: &serde_json::Map<String, JsonValue>, key: &str) -> Option<String> {
    let mut cur = key.to_string();
    for _ in 0..32 {
        if let Some(rest) = cur.strip_prefix('#') {
            let next = textures.get(rest)?.as_str()?;
            cur = next.to_string();
            continue;
        }
        if cur.is_empty() {
            return None;
        }
        // Ensure namespace form for jar path helper
        if cur.contains(':') {
            return Some(cur);
        }
        return Some(format!("minecraft:{cur}"));
    }
    None
}

fn default_uv(from: Vec3, to: Vec3, dir: FaceDir) -> (f32, f32, f32, f32) {
    match dir {
        FaceDir::Down | FaceDir::Up => (from.x, from.z, to.x, to.z),
        FaceDir::North | FaceDir::South => (from.x, from.y, to.x, to.y),
        FaceDir::West | FaceDir::East => (from.z, from.y, to.z, to.y),
    }
}

fn bake_element_face(
    from: Vec3,
    to: Vec3,
    rotation: Option<&JsonValue>,
    dir: FaceDir,
    face_json: &JsonValue,
    textures: &serde_json::Map<String, JsonValue>,
    shade: bool,
) -> Option<BakedFace> {
    let tex_key = face_json.get("texture")?.as_str()?;
    let texture = resolve_texture(textures, tex_key)?;
    let uv = if let Some(arr) = face_json.get("uv").and_then(|u| u.as_array()) {
        if arr.len() >= 4 {
            (
                arr[0].as_f64()? as f32,
                arr[1].as_f64()? as f32,
                arr[2].as_f64()? as f32,
                arr[3].as_f64()? as f32,
            )
        } else {
            default_uv(from, to, dir)
        }
    } else {
        default_uv(from, to, dir)
    };
    let cull = face_json
        .get("cullface")
        .and_then(|c| c.as_str())
        .and_then(FaceDir::parse);
    let tint = face_json
        .get("tintindex")
        .and_then(|t| t.as_i64())
        .is_some();
    let face_rot = face_json
        .get("rotation")
        .and_then(|r| r.as_f64())
        .unwrap_or(0.0) as i32;

    let rotate_pos = |p: Vec3| -> Vec3 {
        let p = apply_element_rotation(p, rotation);
        p / 16.0
    };
    let rotate_n = |n: Vec3| -> Vec3 { apply_element_rotation_vec(n, rotation) };

    let (mut a, mut b, mut c, mut d, normal) = match dir {
        FaceDir::Down => (
            rotate_pos(Vec3::new(from.x, from.y, to.z)),
            rotate_pos(Vec3::new(from.x, from.y, from.z)),
            rotate_pos(Vec3::new(to.x, from.y, from.z)),
            rotate_pos(Vec3::new(to.x, from.y, to.z)),
            rotate_n(dir.normal()),
        ),
        FaceDir::Up => (
            rotate_pos(Vec3::new(from.x, to.y, from.z)),
            rotate_pos(Vec3::new(from.x, to.y, to.z)),
            rotate_pos(Vec3::new(to.x, to.y, to.z)),
            rotate_pos(Vec3::new(to.x, to.y, from.z)),
            rotate_n(dir.normal()),
        ),
        FaceDir::North => (
            rotate_pos(Vec3::new(to.x, to.y, from.z)),
            rotate_pos(Vec3::new(to.x, from.y, from.z)),
            rotate_pos(Vec3::new(from.x, from.y, from.z)),
            rotate_pos(Vec3::new(from.x, to.y, from.z)),
            rotate_n(dir.normal()),
        ),
        FaceDir::South => (
            rotate_pos(Vec3::new(from.x, to.y, to.z)),
            rotate_pos(Vec3::new(from.x, from.y, to.z)),
            rotate_pos(Vec3::new(to.x, from.y, to.z)),
            rotate_pos(Vec3::new(to.x, to.y, to.z)),
            rotate_n(dir.normal()),
        ),
        FaceDir::West => (
            rotate_pos(Vec3::new(from.x, to.y, from.z)),
            rotate_pos(Vec3::new(from.x, from.y, from.z)),
            rotate_pos(Vec3::new(from.x, from.y, to.z)),
            rotate_pos(Vec3::new(from.x, to.y, to.z)),
            rotate_n(dir.normal()),
        ),
        FaceDir::East => (
            rotate_pos(Vec3::new(to.x, to.y, to.z)),
            rotate_pos(Vec3::new(to.x, from.y, to.z)),
            rotate_pos(Vec3::new(to.x, from.y, from.z)),
            rotate_pos(Vec3::new(to.x, to.y, from.z)),
            rotate_n(dir.normal()),
        ),
    };

    // Texture rotation via vertex cycle (web-exporter).
    match (face_rot / 90).rem_euclid(4) {
        1 => {
            let t = a;
            a = b;
            b = c;
            c = d;
            d = t;
        }
        2 => {
            std::mem::swap(&mut a, &mut c);
            std::mem::swap(&mut b, &mut d);
        }
        3 => {
            let t = a;
            a = d;
            d = c;
            c = b;
            b = t;
        }
        _ => {}
    }

    let (u0, v0, u1, v1) = uv;
    Some(BakedFace {
        a,
        b,
        c,
        d,
        uv_a: Vec2::new(u0 / 16.0, 1.0 - v0 / 16.0),
        uv_b: Vec2::new(u0 / 16.0, 1.0 - v1 / 16.0),
        uv_c: Vec2::new(u1 / 16.0, 1.0 - v1 / 16.0),
        uv_d: Vec2::new(u1 / 16.0, 1.0 - v0 / 16.0),
        normal,
        texture,
        cull,
        tint,
        shade,
    })
}

fn apply_element_rotation(p: Vec3, rotation: Option<&JsonValue>) -> Vec3 {
    let Some(rot) = rotation else {
        return p;
    };
    let origin = json_vec3(rot.get("origin")).unwrap_or(Vec3::splat(8.0));
    let angle = rot.get("angle").and_then(|a| a.as_f64()).unwrap_or(0.0) as f32;
    let axis = rot.get("axis").and_then(|a| a.as_str()).unwrap_or("y");
    let rescale = rot
        .get("rescale")
        .and_then(|r| r.as_bool())
        .unwrap_or(false);
    let rad = angle.to_radians();
    let mut sin = rad.sin();
    let mut cos = rad.cos();
    if rescale {
        let scale = 1.0 / cos;
        sin *= scale;
        cos *= scale;
    }
    let rx = p.x - origin.x;
    let ry = p.y - origin.y;
    let rz = p.z - origin.z;
    match axis {
        "x" => Vec3::new(p.x, origin.y + ry * cos - rz * sin, origin.z + rz * cos + ry * sin),
        "z" => Vec3::new(origin.x + rx * cos - ry * sin, origin.y + ry * cos + rx * sin, p.z),
        _ => Vec3::new(origin.x + rx * cos + rz * sin, p.y, origin.z + rz * cos - rx * sin),
    }
}

fn apply_element_rotation_vec(p: Vec3, rotation: Option<&JsonValue>) -> Vec3 {
    let Some(rot) = rotation else {
        return p;
    };
    let angle = rot.get("angle").and_then(|a| a.as_f64()).unwrap_or(0.0) as f32;
    let axis = rot.get("axis").and_then(|a| a.as_str()).unwrap_or("y");
    let rad = angle.to_radians();
    let sin = rad.sin();
    let cos = rad.cos();
    match axis {
        "x" => Vec3::new(p.x, p.y * cos - p.z * sin, p.z * cos + p.y * sin),
        "z" => Vec3::new(p.x * cos - p.y * sin, p.y * cos + p.x * sin, p.z),
        _ => Vec3::new(p.x * cos + p.z * sin, p.y, p.z * cos - p.x * sin),
    }
}

fn rotate_face(face: BakedFace, x_deg: f32, y_deg: f32, uvlock: bool) -> BakedFace {
    let rx = ((x_deg / 90.0).round() as i32).rem_euclid(4) as u8;
    let ry = ((y_deg / 90.0).round() as i32).rem_euclid(4) as u8;
    if rx == 0 && ry == 0 {
        return face;
    }
    let mut f = face;
    // Direction before rotations (web-exporter uses original Direction for some uvlock checks).
    let mut direction = face_dir_from_normal(f.normal).or(f.cull);
    let orig_direction = direction;

    // Rotate X
    match rx {
        1 => {
            f.a = rot_x90(f.a);
            f.b = rot_x90(f.b);
            f.c = rot_x90(f.c);
            f.d = rot_x90(f.d);
            f.normal = Vec3::new(f.normal.x, f.normal.z, -f.normal.y);
            f.cull = map_cull_x90(f.cull);
            direction = map_cull_x90(direction);
        }
        2 => {
            f.a = rot_x180(f.a);
            f.b = rot_x180(f.b);
            f.c = rot_x180(f.c);
            f.d = rot_x180(f.d);
            f.normal = Vec3::new(f.normal.x, -f.normal.y, -f.normal.z);
            if uvlock
                && matches!(
                    direction,
                    Some(FaceDir::East | FaceDir::West | FaceDir::North | FaceDir::South)
                )
            {
                f.uv_a = Vec2::new(1.0 - f.uv_a.x, 1.0 - f.uv_a.y);
                f.uv_b = Vec2::new(1.0 - f.uv_b.x, 1.0 - f.uv_b.y);
                f.uv_c = Vec2::new(1.0 - f.uv_c.x, 1.0 - f.uv_c.y);
                f.uv_d = Vec2::new(1.0 - f.uv_d.x, 1.0 - f.uv_d.y);
            }
            f.cull = map_cull_x180(f.cull);
            direction = map_cull_x180(direction);
        }
        3 => {
            f.a = rot_x270(f.a);
            f.b = rot_x270(f.b);
            f.c = rot_x270(f.c);
            f.d = rot_x270(f.d);
            f.normal = Vec3::new(f.normal.x, -f.normal.z, f.normal.y);
            f.cull = map_cull_x270(f.cull);
            direction = map_cull_x270(direction);
        }
        _ => {}
    }

    match ry {
        1 => {
            f.a = rot_y90(f.a);
            f.b = rot_y90(f.b);
            f.c = rot_y90(f.c);
            f.d = rot_y90(f.d);
            f.normal = Vec3::new(-f.normal.z, f.normal.y, f.normal.x);
            // web-exporter: uvlock when original Direction is Up/Down
            if uvlock && matches!(orig_direction, Some(FaceDir::Up | FaceDir::Down)) {
                if matches!(direction, Some(FaceDir::Down)) {
                    f.uv_a = Vec2::new(1.0 - f.uv_a.y, f.uv_a.x);
                    f.uv_b = Vec2::new(1.0 - f.uv_b.y, f.uv_b.x);
                    f.uv_c = Vec2::new(1.0 - f.uv_c.y, f.uv_c.x);
                    f.uv_d = Vec2::new(1.0 - f.uv_d.y, f.uv_d.x);
                } else {
                    f.uv_a = Vec2::new(f.uv_a.y, 1.0 - f.uv_a.x);
                    f.uv_b = Vec2::new(f.uv_b.y, 1.0 - f.uv_b.x);
                    f.uv_c = Vec2::new(f.uv_c.y, 1.0 - f.uv_c.x);
                    f.uv_d = Vec2::new(f.uv_d.y, 1.0 - f.uv_d.x);
                }
            }
            f.cull = map_cull_y90(f.cull);
            direction = map_cull_y90(direction);
        }
        2 => {
            f.a = rot_y180(f.a);
            f.b = rot_y180(f.b);
            f.c = rot_y180(f.c);
            f.d = rot_y180(f.d);
            f.normal = Vec3::new(-f.normal.x, f.normal.y, -f.normal.z);
            if uvlock && matches!(direction, Some(FaceDir::Up | FaceDir::Down)) {
                f.uv_a = Vec2::new(1.0 - f.uv_a.x, 1.0 - f.uv_a.y);
                f.uv_b = Vec2::new(1.0 - f.uv_b.x, 1.0 - f.uv_b.y);
                f.uv_c = Vec2::new(1.0 - f.uv_c.x, 1.0 - f.uv_c.y);
                f.uv_d = Vec2::new(1.0 - f.uv_d.x, 1.0 - f.uv_d.y);
            }
            f.cull = map_cull_y180(f.cull);
            direction = map_cull_y180(direction);
        }
        3 => {
            f.a = rot_y270(f.a);
            f.b = rot_y270(f.b);
            f.c = rot_y270(f.c);
            f.d = rot_y270(f.d);
            f.normal = Vec3::new(f.normal.z, f.normal.y, -f.normal.x);
            if uvlock && matches!(direction, Some(FaceDir::Up | FaceDir::Down)) {
                if matches!(direction, Some(FaceDir::Down)) {
                    f.uv_a = Vec2::new(f.uv_a.y, 1.0 - f.uv_a.x);
                    f.uv_b = Vec2::new(f.uv_b.y, 1.0 - f.uv_b.x);
                    f.uv_c = Vec2::new(f.uv_c.y, 1.0 - f.uv_c.x);
                    f.uv_d = Vec2::new(f.uv_d.y, 1.0 - f.uv_d.x);
                } else {
                    f.uv_a = Vec2::new(1.0 - f.uv_a.y, f.uv_a.x);
                    f.uv_b = Vec2::new(1.0 - f.uv_b.y, f.uv_b.x);
                    f.uv_c = Vec2::new(1.0 - f.uv_c.y, f.uv_c.x);
                    f.uv_d = Vec2::new(1.0 - f.uv_d.y, f.uv_d.x);
                }
            }
            f.cull = map_cull_y270(f.cull);
            direction = map_cull_y270(direction);
        }
        _ => {}
    }
    let _ = direction;
    f
}

fn face_dir_from_normal(n: Vec3) -> Option<FaceDir> {
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();
    if ay >= ax && ay >= az {
        if n.y >= 0.0 {
            Some(FaceDir::Up)
        } else {
            Some(FaceDir::Down)
        }
    } else if az >= ax {
        if n.z >= 0.0 {
            Some(FaceDir::South)
        } else {
            Some(FaceDir::North)
        }
    } else if n.x >= 0.0 {
        Some(FaceDir::East)
    } else {
        Some(FaceDir::West)
    }
}

fn rot_x90(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, 1.0 - v.y)
}
fn rot_x180(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 1.0 - v.y, 1.0 - v.z)
}
fn rot_x270(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 1.0 - v.z, v.y)
}
fn rot_y90(v: Vec3) -> Vec3 {
    Vec3::new(1.0 - v.z, v.y, v.x)
}
fn rot_y180(v: Vec3) -> Vec3 {
    Vec3::new(1.0 - v.x, v.y, 1.0 - v.z)
}
fn rot_y270(v: Vec3) -> Vec3 {
    Vec3::new(v.z, v.y, 1.0 - v.x)
}

fn map_cull_x90(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::Down,
        FaceDir::Up => FaceDir::North,
        FaceDir::South => FaceDir::Up,
        FaceDir::Down => FaceDir::South,
        o => o,
    })
}
fn map_cull_x180(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::South,
        FaceDir::Up => FaceDir::Down,
        FaceDir::South => FaceDir::North,
        FaceDir::Down => FaceDir::Up,
        o => o,
    })
}
fn map_cull_x270(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::Up,
        FaceDir::Up => FaceDir::South,
        FaceDir::South => FaceDir::Down,
        FaceDir::Down => FaceDir::North,
        o => o,
    })
}
fn map_cull_y90(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::East,
        FaceDir::East => FaceDir::South,
        FaceDir::South => FaceDir::West,
        FaceDir::West => FaceDir::North,
        o => o,
    })
}
fn map_cull_y180(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::South,
        FaceDir::East => FaceDir::West,
        FaceDir::South => FaceDir::North,
        FaceDir::West => FaceDir::East,
        o => o,
    })
}
fn map_cull_y270(c: Option<FaceDir>) -> Option<FaceDir> {
    c.map(|d| match d {
        FaceDir::North => FaceDir::West,
        FaceDir::East => FaceDir::North,
        FaceDir::South => FaceDir::East,
        FaceDir::West => FaceDir::South,
        o => o,
    })
}

fn is_full_cube_heuristic(faces: &[BakedFace]) -> bool {
    // Six axis-aligned cullfaces covering full sides → treat as opaque cube for cull.
    let mut mask = 0u8;
    for f in faces {
        let Some(c) = f.cull else {
            continue;
        };
        let bit = match c {
            FaceDir::Down => 1,
            FaceDir::Up => 2,
            FaceDir::North => 4,
            FaceDir::South => 8,
            FaceDir::West => 16,
            FaceDir::East => 32,
        };
        // Rough: vertices span ~full face
        let (min, max) = face_bounds_2d(f, c);
        if (max.x - min.x) > 0.99 && (max.y - min.y) > 0.99 {
            mask |= bit;
        }
    }
    mask == 0x3F
}

fn face_bounds_2d(f: &BakedFace, dir: FaceDir) -> (Vec2, Vec2) {
    let project = |v: Vec3| match dir {
        FaceDir::Down | FaceDir::Up => Vec2::new(v.x, v.z),
        FaceDir::North | FaceDir::South => Vec2::new(v.x, v.y),
        FaceDir::West | FaceDir::East => Vec2::new(v.z, v.y),
    };
    let pts = [project(f.a), project(f.b), project(f.c), project(f.d)];
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for p in pts {
        min = min.min(p);
        max = max.max(p);
    }
    (min, max)
}

/// Neighbor full-cube opaque → cull this face (fast path for palace solids).
pub fn should_cull_face(face: &BakedFace, neighbor: Option<&BakedBlock>) -> bool {
    let Some(cull) = face.cull else {
        return false;
    };
    let Some(n) = neighbor else {
        return false;
    };
    if n.faces.is_empty() {
        return false;
    }
    if n.full_cube {
        return true;
    }
    // Partial: any opposite cullface on neighbor that covers us (simplified).
    let inv = cull.invert();
    let (min_this, max_this) = face_bounds_2d(face, cull);
    for of in &n.faces {
        if of.cull != Some(inv) {
            continue;
        }
        let (min_o, max_o) = face_bounds_2d(of, cull);
        if min_this.x >= min_o.x - 1e-3
            && max_this.x <= max_o.x + 1e-3
            && min_this.y >= min_o.y - 1e-3
            && max_this.y <= max_o.y + 1e-3
        {
            return true;
        }
    }
    false
}

fn match_variants<'a>(
    variants: &'a serde_json::Map<String, JsonValue>,
    props: &std::collections::BTreeMap<String, String>,
) -> Option<Vec<&'a JsonValue>> {
    // Preserve JSON insertion order (serde_json::Map).
    for (key, value) in variants {
        if variant_key_matches(key, props) {
            return Some(variant_entries(value));
        }
    }
    None
}

fn variant_key_matches(key: &str, props: &std::collections::BTreeMap<String, String>) -> bool {
    if key.is_empty() {
        return true;
    }
    for part in key.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match props.get(k.trim()) {
            Some(pv) if pv == v.trim() => {}
            _ => return false,
        }
    }
    true
}

fn variant_entries(value: &JsonValue) -> Vec<&JsonValue> {
    match value {
        JsonValue::Array(a) => a.iter().collect(),
        JsonValue::Object(_) => vec![value],
        _ => Vec::new(),
    }
}

fn when_matches(when: Option<&JsonValue>, props: &std::collections::BTreeMap<String, String>) -> bool {
    let Some(when) = when else {
        return true;
    };
    // OR: { "OR": [ {...}, {...} ] }
    if let Some(arr) = when.get("OR").and_then(|o| o.as_array()) {
        return arr.iter().any(|clause| and_clause_matches(clause, props));
    }
    // AND: { "AND": [ {...}, {...} ] } → merge into one clause
    if let Some(arr) = when.get("AND").and_then(|a| a.as_array()) {
        return arr.iter().all(|clause| and_clause_matches(clause, props));
    }
    and_clause_matches(when, props)
}

fn and_clause_matches(clause: &JsonValue, props: &std::collections::BTreeMap<String, String>) -> bool {
    let Some(obj) = clause.as_object() else {
        return false;
    };
    for (k, v) in obj {
        if k == "OR" || k == "AND" {
            continue;
        }
        let Some(want) = v.as_str() else {
            return false;
        };
        let Some(have) = props.get(k) else {
            return false;
        };
        // pipe-separated alternatives
        if !want.split('|').any(|alt| alt == have) {
            return false;
        }
    }
    true
}

/// Cheap AO: darken if any of 3 neighbors toward the face corner are solid.
pub fn simple_ao(solid_at: impl Fn(i32, i32, i32) -> bool, x: i32, y: i32, z: i32, corner: Vec3, normal: Vec3) -> f32 {
    let nx = if normal.x.abs() > 0.5 {
        normal.x.signum() as i32
    } else {
        0
    };
    let ny = if normal.y.abs() > 0.5 {
        normal.y.signum() as i32
    } else {
        0
    };
    let nz = if normal.z.abs() > 0.5 {
        normal.z.signum() as i32
    } else {
        0
    };
    // Corner offsets in the face plane
    let sx = if (corner.x - 0.5).abs() > 0.1 {
        if corner.x > 0.5 {
            1
        } else {
            -1
        }
    } else {
        0
    };
    let sy = if (corner.y - 0.5).abs() > 0.1 {
        if corner.y > 0.5 {
            1
        } else {
            -1
        }
    } else {
        0
    };
    let sz = if (corner.z - 0.5).abs() > 0.1 {
        if corner.z > 0.5 {
            1
        } else {
            -1
        }
    } else {
        0
    };
    let mut occ = 0;
    // side1, side2, diagonal in the outward hemisphere
    let s1 = (x + sx + nx, y + if nx != 0 || nz != 0 { sy } else { 0 } + ny, z + if nx != 0 || ny != 0 { sz } else { 0 } + nz);
    // Simplified MC-style: check three neighbors of the vertex
    let c1 = (x + sx, y + sy, z + sz);
    let c2 = (x + sx + nx, y + sy + ny, z + sz + nz);
    let c3 = (x + nx, y + ny, z + nz);
    for (cx, cy, cz) in [c1, c2, c3] {
        if solid_at(cx, cy, cz) {
            occ += 1;
        }
    }
    let _ = s1;
    match occ {
        0 => 1.0,
        1 => 0.8,
        2 => 0.6,
        _ => 0.4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn write_png(path: &Path, rgba: [u8; 4]) {
        let mut img = RgbaImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = image::Rgba(rgba);
        }
        img.save(path).unwrap();
    }

    fn fixture_jar(dir: &Path) -> PathBuf {
        let jar = dir.join("models-26.2.jar");
        let oak = dir.join("oak_planks.png");
        write_png(&oak, [0xC0, 0x90, 0x50, 0xFF]);
        let file = File::create(&jar).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        // Use normal strings — `#` in JSON must not appear inside raw `br#"..."#`.
        let hash = "#";
        let files: Vec<(&str, Vec<u8>)> = vec![
            (
                "assets/minecraft/textures/block/oak_planks.png",
                std::fs::read(&oak).unwrap(),
            ),
            (
                "assets/minecraft/textures/block/stone.png",
                std::fs::read(&oak).unwrap(),
            ),
            (
                "assets/minecraft/blockstates/oak_slab.json",
                br#"{
  "variants": {
    "type=bottom": { "model": "minecraft:block/oak_slab" },
    "type=top": { "model": "minecraft:block/oak_slab_top" },
    "type=double": { "model": "minecraft:block/oak_planks" }
  }
}"#
                .to_vec(),
            ),
            (
                "assets/minecraft/blockstates/oak_planks.json",
                br#"{ "variants": { "": { "model": "minecraft:block/oak_planks" } } }"#.to_vec(),
            ),
            (
                "assets/minecraft/blockstates/oak_fence.json",
                br#"{
  "multipart": [
    { "apply": { "model": "minecraft:block/oak_fence_post" } },
    { "when": { "north": "true" }, "apply": { "model": "minecraft:block/oak_fence_side", "uvlock": true } }
  ]
}"#
                .to_vec(),
            ),
            (
                "assets/minecraft/models/block/oak_planks.json",
                br#"{ "parent": "minecraft:block/cube_all", "textures": { "all": "minecraft:block/oak_planks" } }"#
                    .to_vec(),
            ),
            (
                "assets/minecraft/models/block/cube_all.json",
                format!(
                    r#"{{
  "parent": "block/cube",
  "textures": {{
    "particle": "{h}all", "down": "{h}all", "up": "{h}all",
    "north": "{h}all", "south": "{h}all", "west": "{h}all", "east": "{h}all"
  }}
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/cube.json",
                format!(
                    r#"{{
  "elements": [{{
    "from": [0,0,0], "to": [16,16,16],
    "faces": {{
      "down":  {{ "texture": "{h}down", "cullface": "down" }},
      "up":    {{ "texture": "{h}up", "cullface": "up" }},
      "north": {{ "texture": "{h}north", "cullface": "north" }},
      "south": {{ "texture": "{h}south", "cullface": "south" }},
      "west":  {{ "texture": "{h}west", "cullface": "west" }},
      "east":  {{ "texture": "{h}east", "cullface": "east" }}
    }}
  }}]
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/oak_slab.json",
                format!(
                    r#"{{
  "parent": "minecraft:block/slab",
  "textures": {{
    "bottom": "minecraft:block/oak_planks",
    "top": "minecraft:block/oak_planks",
    "side": "minecraft:block/oak_planks"
  }}
}}"#
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/oak_slab_top.json",
                format!(
                    r#"{{
  "parent": "minecraft:block/slab_top",
  "textures": {{
    "bottom": "minecraft:block/oak_planks",
    "top": "minecraft:block/oak_planks",
    "side": "minecraft:block/oak_planks"
  }}
}}"#
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/slab.json",
                format!(
                    r#"{{
  "textures": {{ "particle": "{h}side" }},
  "elements": [{{
    "from": [0,0,0], "to": [16,8,16],
    "faces": {{
      "down":  {{ "texture": "{h}bottom", "cullface": "down" }},
      "up":    {{ "texture": "{h}top" }},
      "north": {{ "texture": "{h}side", "cullface": "north" }},
      "south": {{ "texture": "{h}side", "cullface": "south" }},
      "west":  {{ "texture": "{h}side", "cullface": "west" }},
      "east":  {{ "texture": "{h}side", "cullface": "east" }}
    }}
  }}]
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/slab_top.json",
                format!(
                    r#"{{
  "textures": {{ "particle": "{h}side" }},
  "elements": [{{
    "from": [0,8,0], "to": [16,16,16],
    "faces": {{
      "down":  {{ "texture": "{h}bottom" }},
      "up":    {{ "texture": "{h}top", "cullface": "up" }},
      "north": {{ "texture": "{h}side", "cullface": "north" }},
      "south": {{ "texture": "{h}side", "cullface": "south" }},
      "west":  {{ "texture": "{h}side", "cullface": "west" }},
      "east":  {{ "texture": "{h}side", "cullface": "east" }}
    }}
  }}]
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/oak_fence_post.json",
                format!(
                    r#"{{
  "textures": {{ "particle": "minecraft:block/oak_planks", "texture": "minecraft:block/oak_planks" }},
  "elements": [{{
    "from": [6,0,6], "to": [10,16,10],
    "faces": {{
      "down":  {{ "texture": "{h}texture", "cullface": "down" }},
      "up":    {{ "texture": "{h}texture", "cullface": "up" }},
      "north": {{ "texture": "{h}texture" }},
      "south": {{ "texture": "{h}texture" }},
      "west":  {{ "texture": "{h}texture" }},
      "east":  {{ "texture": "{h}texture" }}
    }}
  }}]
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
            (
                "assets/minecraft/models/block/oak_fence_side.json",
                format!(
                    r#"{{
  "textures": {{ "particle": "minecraft:block/oak_planks", "texture": "minecraft:block/oak_planks" }},
  "elements": [{{
    "from": [7,0,0], "to": [9,16,9],
    "faces": {{
      "down":  {{ "texture": "{h}texture" }},
      "up":    {{ "texture": "{h}texture" }},
      "north": {{ "texture": "{h}texture", "cullface": "north" }},
      "south": {{ "texture": "{h}texture" }},
      "west":  {{ "texture": "{h}texture" }},
      "east":  {{ "texture": "{h}texture" }}
    }}
  }}]
}}"#,
                    h = hash
                )
                .into_bytes(),
            ),
        ];
        for (name, data) in files {
            zip.start_file(name, opts).unwrap();
            zip.write_all(&data).unwrap();
        }
        zip.finish().unwrap();
        jar
    }

    #[test]
    fn slab_bottom_has_half_height_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let jar = fixture_jar(dir.path());
        let mut cat = ModelCatalog::open(&jar).unwrap();
        let state = BlockState::parse("minecraft:oak_slab[type=bottom]").unwrap();
        let baked = cat.bake_block(&state);
        assert!(!baked.faces.is_empty());
        // Top of slab should be at y=0.5
        let max_y = baked
            .faces
            .iter()
            .flat_map(|f| [f.a.y, f.b.y, f.c.y, f.d.y])
            .fold(0.0f32, f32::max);
        assert!((max_y - 0.5).abs() < 1e-3, "max_y={max_y}");
        assert!(!baked.full_cube);
    }

    #[test]
    fn fence_multipart_adds_side_when_north() {
        let dir = tempfile::tempdir().unwrap();
        let jar = fixture_jar(dir.path());
        let mut cat = ModelCatalog::open(&jar).unwrap();
        let post_only = BlockState::parse("minecraft:oak_fence[north=false,south=false,east=false,west=false]")
            .unwrap();
        let with_n = BlockState::parse("minecraft:oak_fence[north=true,south=false,east=false,west=false]")
            .unwrap();
        let a = cat.bake_block(&post_only);
        let b = cat.bake_block(&with_n);
        assert!(b.faces.len() > a.faces.len());
    }

    #[test]
    fn cube_all_resolves_texture_var() {
        let dir = tempfile::tempdir().unwrap();
        let jar = fixture_jar(dir.path());
        let mut cat = ModelCatalog::open(&jar).unwrap();
        let state = BlockState::parse("minecraft:oak_planks").unwrap();
        let baked = cat.bake_block(&state);
        assert_eq!(baked.faces.len(), 6);
        assert!(baked.full_cube);
        assert!(baked.faces[0].texture.contains("oak_planks"));
        assert!(cat.texture_image(&baked.faces[0].texture).is_some());
    }

    #[test]
    fn variant_key_subset_match() {
        let mut props = std::collections::BTreeMap::new();
        props.insert("type".into(), "bottom".into());
        assert!(variant_key_matches("type=bottom", &props));
        assert!(!variant_key_matches("type=top", &props));
        assert!(variant_key_matches("", &props));
    }

    #[test]
    fn lightmap_dark_vs_bright() {
        use crate::view::mc_lightmap_brightness;
        assert!(mc_lightmap_brightness(15, 0) > mc_lightmap_brightness(0, 0));
        assert!(mc_lightmap_brightness(0, 14) > mc_lightmap_brightness(0, 0));
    }
}
