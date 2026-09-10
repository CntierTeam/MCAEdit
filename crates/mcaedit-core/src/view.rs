use crate::assets::{
    needs_biome_tint, BlockTextureAtlas, CubeFace, DEFAULT_MC_VERSION,
};
use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::region::{parse_region_name, RegionStore};
use crate::session::Session;
use crate::world::WorldView;
use glam::{Mat4, Vec2, Vec3, Vec4};
use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Default max AABB cell count for offline mesh builds (screenshot + preview).
/// Overridable via `MCAEDIT_VIEW_MAX_CELLS` / `--max-cells` (Taihe-scale ~175k needs ≫ 48³).
pub const VIEW_MAX_CELLS_DEFAULT: usize = 2_000_000;

/// Backward-compatible alias (historical 48³ hard limit removed).
pub const VIEW_MAX_CELLS: usize = VIEW_MAX_CELLS_DEFAULT;

/// Resolve max cells: explicit arg → env → default.
pub fn resolve_view_max_cells(explicit: Option<usize>) -> usize {
    if let Some(n) = explicit.filter(|n| *n > 0) {
        return n;
    }
    std::env::var("MCAEDIT_VIEW_MAX_CELLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(VIEW_MAX_CELLS_DEFAULT)
}

#[derive(Clone, Debug)]
pub struct ViewScreenshotRequest {
    pub from: (i32, i32, i32),
    pub to: (i32, i32, i32),
    pub width: u32,
    pub height: u32,
    pub camera: Option<(f32, f32, f32)>,
    pub look: Option<(f32, f32, f32)>,
    pub out: PathBuf,
    /// Client jar or versions/<ver> directory (`--minecraft`).
    pub minecraft: Option<PathBuf>,
    /// Explicit assets jar (`--assets-jar`).
    pub assets_jar: Option<PathBuf>,
    /// When true, skip jar lookup (palette colors only).
    pub no_textures: bool,
    /// Cap AABB volume; `None` → env / default.
    pub max_cells: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ViewScreenshotResult {
    pub out: PathBuf,
    pub width: u32,
    pub height: u32,
    pub blocks: usize,
    pub faces: usize,
    pub triangles: usize,
    /// `jar path=...` or `palette reason=...`.
    pub textures: String,
}

/// Shared face-culled mesh used by screenshot and live preview.
#[derive(Clone, Debug, Default)]
pub struct ViewMesh {
    pub triangles: Vec<ViewTriangle>,
    pub blocks: usize,
    pub faces: usize,
    pub from: (i32, i32, i32),
    pub to: (i32, i32, i32),
    /// Texture images; vertex `tex_id == 0` means solid `color` only.
    /// Indices are 1-based into this vec (`textures[tex_id - 1]`).
    pub textures: Vec<RgbaImage>,
}

#[derive(Clone, Copy, Debug)]
pub struct ViewTriangle {
    pub a: ViewVertex,
    pub b: ViewVertex,
    pub c: ViewVertex,
}

#[derive(Clone, Copy, Debug)]
pub struct ViewVertex {
    pub pos: Vec3,
    pub normal: Vec3,
    pub color: Vec3,
    pub uv: Vec2,
    /// 0 = solid color; else 1-based index into [`ViewMesh::textures`].
    pub tex_id: u16,
}

/// RGBA8 frame buffer (row-major).
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Disk fingerprint for live preview reload (region mtime + session cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewWatchToken {
    pub region_stamp: u64,
    pub meta_mtime_ms: u64,
    pub head_mtime_ms: u64,
    pub head: usize,
    pub history_size: usize,
    pub dirty: bool,
}

const SKY: [u8; 4] = [0x78, 0xA7, 0xFF, 0xFF];

impl<'a> WorldView<'a> {
    pub fn view_screenshot(&self, req: &ViewScreenshotRequest) -> Result<ViewScreenshotResult> {
        if req.width < 64 || req.height < 64 {
            return Err(Error::msg("width/height must be >= 64"));
        }
        let (mut atlas, textures_label) =
            resolve_atlas_cli(req.minecraft.as_deref(), req.assets_jar.as_deref(), req.no_textures);
        let mesh = self.build_view_mesh_with_textures_limited(
            req.from,
            req.to,
            atlas.as_mut(),
            req.max_cells,
        )?;
        let (look, camera) = default_camera_for_mesh(&mesh, req.look, req.camera);
        let frame = rasterize_view_mesh(&mesh, req.width, req.height, camera, look);
        if let Some(parent) = req.out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let img = frame_to_image(&frame);
        img.save(&req.out)
            .map_err(|e| Error::msg(format!("save png failed: {e}")))?;
        Ok(ViewScreenshotResult {
            out: req.out.clone(),
            width: req.width,
            height: req.height,
            blocks: mesh.blocks,
            faces: mesh.faces,
            triangles: mesh.triangles.len(),
            textures: textures_label,
        })
    }

    /// Build a face-culled cube mesh for an AABB (shared by screenshot and live preview).
    pub fn build_view_mesh(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> Result<ViewMesh> {
        self.build_view_mesh_with_textures_limited(from, to, None, None)
    }

    /// Like [`Self::build_view_mesh`], optionally sampling Minecraft block textures.
    pub fn build_view_mesh_with_textures(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        atlas: Option<&mut BlockTextureAtlas>,
    ) -> Result<ViewMesh> {
        self.build_view_mesh_with_textures_limited(from, to, atlas, None)
    }

    /// Mesh build with optional `--max-cells` / env override.
    ///
    /// Performance: load each overlapping chunk/section **once** into a dense
    /// occupancy+palette grid, then face-cull in memory (no per-cell disk I/O).
    pub fn build_view_mesh_with_textures_limited(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        mut atlas: Option<&mut BlockTextureAtlas>,
        max_cells: Option<usize>,
    ) -> Result<ViewMesh> {
        let (min_x, max_x) = (from.0.min(to.0), from.0.max(to.0));
        let (min_y, max_y) = (from.1.min(to.1), from.1.max(to.1));
        let (min_z, max_z) = (from.2.min(to.2), from.2.max(to.2));
        let sx = (max_x - min_x + 1) as usize;
        let sy = (max_y - min_y + 1) as usize;
        let sz = (max_z - min_z + 1) as usize;
        let cells = sx.saturating_mul(sy).saturating_mul(sz);
        let limit = resolve_view_max_cells(max_cells);
        if cells == 0 || cells > limit {
            return Err(Error::msg(format!(
                "view box too large ({sx}x{sy}x{sz}={cells} cells; max={limit}; raise --max-cells / MCAEDIT_VIEW_MAX_CELLS)"
            )));
        }

        // Dense grid: 0 = air, else 1-based palette index.
        let mut grid = vec![0u16; cells];
        let mut palette: Vec<BlockState> = Vec::new();
        let mut palette_index: HashMap<BlockState, u16> = HashMap::new();

        let idx = |x: i32, y: i32, z: i32| -> usize {
            let xi = (x - min_x) as usize;
            let yi = (y - min_y) as usize;
            let zi = (z - min_z) as usize;
            (yi * sz + zi) * sx + xi
        };

        let mut insert_state = |b: BlockState| -> u16 {
            if b.is_air_like() {
                return 0;
            }
            if let Some(id) = palette_index.get(&b) {
                return *id;
            }
            if palette.len() >= u16::MAX as usize - 1 {
                return 0;
            }
            palette.push(b.clone());
            let id = palette.len() as u16; // 1-based
            palette_index.insert(b, id);
            id
        };

        // Chunk-batched preload (dominant speedup vs per-cell get_block disk thrash).
        let min_cx = min_x >> 4;
        let max_cx = max_x >> 4;
        let min_cz = min_z >> 4;
        let max_cz = max_z >> 4;
        let min_sy = (min_y >> 4) as i8;
        let max_sy = (max_y >> 4) as i8;
        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let chunk = self.load_chunk(cx, cz)?;
                let bx0 = (cx << 4).max(min_x);
                let bx1 = ((cx << 4) + 15).min(max_x);
                let bz0 = (cz << 4).max(min_z);
                let bz1 = ((cz << 4) + 15).min(max_z);
                for sy_i in min_sy..=max_sy {
                    let section = chunk.read_section_blocks(sy_i)?;
                    let by0 = ((sy_i as i32) << 4).max(min_y);
                    let by1 = (((sy_i as i32) << 4) + 15).min(max_y);
                    for y in by0..=by1 {
                        let ly = (y & 15) as u8;
                        for z in bz0..=bz1 {
                            let lz = (z & 15) as u8;
                            for x in bx0..=bx1 {
                                let lx = (x & 15) as u8;
                                let b = section.get(lx, ly, lz).clone();
                                let id = insert_state(b);
                                if id != 0 {
                                    grid[idx(x, y, z)] = id;
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut mesh = ViewMesh {
            triangles: Vec::new(),
            blocks: 0,
            faces: 0,
            from: (min_x, min_y, min_z),
            to: (max_x, max_y, max_z),
            textures: Vec::new(),
        };
        mesh.triangles.reserve(cells / 4);
        let mut tex_keys: HashMap<(u16, u8), u16> = HashMap::new();

        let neighbor_air = |x: i32, y: i32, z: i32| -> bool {
            if x < min_x || x > max_x || y < min_y || y > max_y || z < min_z || z > max_z {
                return true;
            }
            grid[idx(x, y, z)] == 0
        };

        let face_key = |f: CubeFace| -> u8 {
            match f {
                CubeFace::NegX => 0,
                CubeFace::PosX => 1,
                CubeFace::NegY => 2,
                CubeFace::PosY => 3,
                CubeFace::NegZ => 4,
                CubeFace::PosZ => 5,
            }
        };

        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let pid = grid[idx(x, y, z)];
                    if pid == 0 {
                        continue;
                    }
                    mesh.blocks += 1;
                    let b = &palette[(pid as usize) - 1];
                    let palette_col = color_for_block(b);
                    let tint = if needs_biome_tint(&b.name) {
                        palette_col
                    } else {
                        Vec3::ONE
                    };
                    for (face_kind, mut face) in
                        cube_faces(x as f32, y as f32, z as f32, palette_col)
                    {
                        let nx = x + face.normal.x as i32;
                        let ny = y + face.normal.y as i32;
                        let nz = z + face.normal.z as i32;
                        if !neighbor_air(nx, ny, nz) {
                            continue;
                        }
                        let tex_id = match atlas.as_mut() {
                            Some(a) => {
                                let key = (pid, face_key(face_kind));
                                if let Some(id) = tex_keys.get(&key) {
                                    *id
                                } else {
                                    let id = ensure_face_texture(a, &mut mesh.textures, &b.name, face_kind);
                                    tex_keys.insert(key, id);
                                    id
                                }
                            }
                            None => 0,
                        };
                        let vert_color = if tex_id > 0 { tint } else { palette_col };
                        mesh.faces += 1;
                        for v in [&mut face.v0, &mut face.v1, &mut face.v2, &mut face.v3] {
                            v.tex_id = tex_id;
                            v.color = vert_color;
                        }
                        mesh.triangles.push(ViewTriangle {
                            a: face.v0,
                            b: face.v1,
                            c: face.v2,
                        });
                        mesh.triangles.push(ViewTriangle {
                            a: face.v0,
                            b: face.v2,
                            c: face.v3,
                        });
                    }
                }
            }
        }
        Ok(mesh)
    }
}

/// Open atlas from an explicit jar path, or auto-detect Minecraft 26.2.
/// Never panics: missing/invalid jar → `(None, "palette reason=...")`.
pub fn resolve_atlas_for_view(
    explicit: Option<&Path>,
    no_textures: bool,
) -> (Option<BlockTextureAtlas>, String) {
    if no_textures {
        return (None, "palette reason=no-textures".into());
    }
    if let Some(p) = explicit {
        return match BlockTextureAtlas::open(p) {
            Ok(a) => {
                let label = format!("jar path={}", a.path().display());
                (Some(a), label)
            }
            Err(_) => (None, "palette reason=jar-open-failed".into()),
        };
    }
    match BlockTextureAtlas::discover(DEFAULT_MC_VERSION) {
        Some(a) => {
            let label = format!("jar path={}", a.path().display());
            (Some(a), label)
        }
        None => (None, "palette reason=jar-not-found".into()),
    }
}

/// Resolve using `--minecraft` / `--assets-jar` / env / auto-detect.
pub fn resolve_atlas_cli(
    minecraft: Option<&Path>,
    assets_jar: Option<&Path>,
    no_textures: bool,
) -> (Option<BlockTextureAtlas>, String) {
    if no_textures {
        return (None, "palette reason=no-textures".into());
    }
    match BlockTextureAtlas::resolve(minecraft, assets_jar, DEFAULT_MC_VERSION) {
        Some(a) => {
            let label = format!("jar path={}", a.path().display());
            (Some(a), label)
        }
        None => (None, "palette reason=jar-not-found".into()),
    }
}

fn ensure_face_texture(
    atlas: &mut BlockTextureAtlas,
    textures: &mut Vec<RgbaImage>,
    block_name: &str,
    face: CubeFace,
) -> u16 {
    let Some(img) = atlas.image_for_block_face(block_name, face) else {
        return 0;
    };
    if textures.len() >= u16::MAX as usize - 1 {
        return 0;
    }
    textures.push(img);
    textures.len() as u16 // 1-based
}

/// Soft-rasterize a mesh (same lighting/cull pipeline as screenshot).
pub fn rasterize_view_mesh(
    mesh: &ViewMesh,
    width: u32,
    height: u32,
    camera: Vec3,
    look: Vec3,
) -> RgbaFrame {
    let mut img = RgbaImage::from_pixel(width, height, Rgba(SKY));
    let mut zbuf = vec![f32::INFINITY; (width * height) as usize];
    let view = Mat4::look_at_rh(camera, look, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0f32.to_radians(), width as f32 / height as f32, 0.1, 8192.0);
    let vp = proj * view;

    for tri in &mesh.triangles {
        let Some(a) = project(tri.a, vp, width as f32, height as f32) else {
            continue;
        };
        let Some(b) = project(tri.b, vp, width as f32, height as f32) else {
            continue;
        };
        let Some(c) = project(tri.c, vp, width as f32, height as f32) else {
            continue;
        };
        draw_triangle(&mut img, &mut zbuf, a, b, c, &mesh.textures);
    }

    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for p in img.pixels() {
        pixels.extend_from_slice(&p.0);
    }
    RgbaFrame {
        width,
        height,
        pixels,
    }
}

/// Minecraft `Direction.getShade` face multipliers (flat face shading).
///
/// Classic ladder: **top 1.0**, **N/S 0.8**, **E/W 0.6**, **bottom 0.5**.
/// Used by both screenshot and live preview soft-raster (no Lambert).
pub fn mc_face_shade(normal: Vec3) -> f32 {
    let n = normal.normalize_or_zero();
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();
    if ay >= ax && ay >= az {
        if n.y >= 0.0 {
            1.0
        } else {
            0.5
        }
    } else if az >= ax {
        0.8
    } else {
        0.6
    }
}

/// Approximate Minecraft lightmap brightness for sky/block levels `0..=15`.
///
/// Matches the long-standing CPU curve used before GPU lightmap textures:
/// `(1 - x^4) * (1 - ambient) + ambient` with `x = 1 - level/15`.
/// Offline view assumes outdoor daytime → callers usually pass sky=15, block=0.
pub fn mc_lightmap_brightness(sky: u8, block: u8) -> f32 {
    let level = sky.max(block).min(15);
    let ambient = 0.04_f32;
    let x = 1.0 - (level as f32) / 15.0;
    let shaped = 1.0 - x * x * x * x;
    (shaped * (1.0 - ambient) + ambient).clamp(0.0, 1.0)
}

/// Combined MC shade for a face under full outdoor skylight (preview/screenshot default).
pub fn mc_vertex_shade(normal: Vec3) -> f32 {
    (mc_face_shade(normal) * mc_lightmap_brightness(15, 0)).clamp(0.0, 1.0)
}

pub fn default_camera_for_mesh(
    mesh: &ViewMesh,
    look: Option<(f32, f32, f32)>,
    camera: Option<(f32, f32, f32)>,
) -> (Vec3, Vec3) {
    let (min_x, min_y, min_z) = mesh.from;
    let (max_x, max_y, max_z) = mesh.to;
    let sx = (max_x - min_x + 1) as f32;
    let sy = (max_y - min_y + 1) as f32;
    let sz = (max_z - min_z + 1) as f32;
    let look_v = look.map(triple_to_vec3).unwrap_or_else(|| {
        Vec3::new(
            (min_x + max_x) as f32 * 0.5 + 0.5,
            (min_y + max_y) as f32 * 0.5 + 0.5,
            (min_z + max_z) as f32 * 0.5 + 0.5,
        )
    });
    let span = sx.max(sy).max(sz).max(8.0);
    let camera_v = camera
        .map(triple_to_vec3)
        .unwrap_or(look_v + Vec3::new(0.0, span * 1.2, span * 0.6));
    (look_v, camera_v)
}

/// Orbit camera helper for interactive preview.
pub fn orbit_camera(look: Vec3, yaw_deg: f32, pitch_deg: f32, distance: f32) -> Vec3 {
    let yaw = yaw_deg.to_radians();
    let pitch = pitch_deg.clamp(-89.0, 89.0).to_radians();
    let x = distance * yaw.cos() * pitch.cos();
    let y = distance * pitch.sin();
    let z = distance * yaw.sin() * pitch.cos();
    look + Vec3::new(x, y, z)
}

/// Collect a cheap reload token for session work copy + history cursor.
pub fn preview_watch_token(session_root: &Path) -> Result<PreviewWatchToken> {
    let region_dir = session_root.join("world/region");
    let mut region_stamp = 0u64;
    if region_dir.is_dir() {
        for ent in std::fs::read_dir(&region_dir)? {
            let ent = ent?;
            let path = ent.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !(name.ends_with(".mca") || name.ends_with(".linear")) {
                continue;
            }
            let meta = ent.metadata()?;
            let m = file_mtime_ms(&meta);
            let len = meta.len();
            // Mix path + mtime + length so renames/truncates also bust cache.
            let mut h = region_stamp;
            h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(m);
            h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(len);
            for b in name.bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as u64);
            }
            region_stamp ^= h;
        }
    }

    let meta_path = session_root.join("meta.json");
    let head_path = session_root.join("HEAD");
    let meta_mtime_ms = path_mtime_ms(&meta_path);
    let head_mtime_ms = path_mtime_ms(&head_path);

    let mut dirty = false;
    let mut head = 0usize;
    let mut history_size = 0usize;
    if meta_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&meta_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                dirty = v.get("dirty").and_then(|d| d.as_bool()).unwrap_or(false);
            }
        }
    }
    if head_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&head_path) {
            head = text.trim().parse().unwrap_or(0);
        }
    }
    let history_dir = session_root.join("history");
    if history_dir.is_dir() {
        history_size = std::fs::read_dir(&history_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext == "json")
            })
            .count();
    }

    Ok(PreviewWatchToken {
        region_stamp,
        meta_mtime_ms,
        head_mtime_ms,
        head,
        history_size,
        dirty,
    })
}

/// Inclusive AABB as `(from, to)` block coordinates.
pub type PreviewAabb = ((i32, i32, i32), (i32, i32, i32));

/// Suggest a focus AABB from work-region chunk presence (clamped to 48³).
pub fn suggest_preview_aabb(session: &Session) -> Result<PreviewAabb> {
    let region_dir = session.work_region_dir();
    let mut min_cx = i32::MAX;
    let mut max_cx = i32::MIN;
    let mut min_cz = i32::MAX;
    let mut max_cz = i32::MIN;
    let mut any = false;
    if region_dir.is_dir() {
        for ent in std::fs::read_dir(&region_dir)? {
            let ent = ent?;
            let path = ent.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) if n.ends_with(".mca") || n.ends_with(".linear") => n,
                _ => continue,
            };
            if parse_region_name(name).is_err() {
                continue;
            }
            let store = RegionStore::open(&path);
            for (cx, cz) in store.list_present_chunks()? {
                any = true;
                min_cx = min_cx.min(cx);
                max_cx = max_cx.max(cx);
                min_cz = min_cz.min(cz);
                max_cz = max_cz.max(cz);
            }
        }
    }
    if !any {
        return Ok(((0, 56, 0), (31, 87, 31)));
    }

    let mut min_x = min_cx * 16;
    let mut max_x = max_cx * 16 + 15;
    let mut min_z = min_cz * 16;
    let mut max_z = max_cz * 16 + 15;
    let min_y = 48;
    let max_y = 95;
    // Soft clamp XZ so auto preview stays within default max cells (~2e6 with Y≈48).
    let max_edge = 128;
    let mid_x = (min_x + max_x) / 2;
    let mid_z = (min_z + max_z) / 2;
    if max_x - min_x + 1 > max_edge {
        min_x = mid_x - max_edge / 2 + 1;
        max_x = mid_x + max_edge / 2;
    }
    if max_z - min_z + 1 > max_edge {
        min_z = mid_z - max_edge / 2 + 1;
        max_z = mid_z + max_edge / 2;
    }
    Ok(((min_x, min_y, min_z), (max_x, max_y, max_z)))
}

fn frame_to_image(frame: &RgbaFrame) -> RgbaImage {
    let mut img = RgbaImage::new(frame.width, frame.height);
    for y in 0..frame.height {
        for x in 0..frame.width {
            let i = ((y * frame.width + x) * 4) as usize;
            img.put_pixel(
                x,
                y,
                Rgba([
                    frame.pixels[i],
                    frame.pixels[i + 1],
                    frame.pixels[i + 2],
                    frame.pixels[i + 3],
                ]),
            );
        }
    }
    img
}

fn path_mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .map(|m| file_mtime_ms(&m))
        .unwrap_or(0)
}

fn file_mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
struct Face {
    normal: Vec3,
    v0: ViewVertex,
    v1: ViewVertex,
    v2: ViewVertex,
    v3: ViewVertex,
}

#[derive(Clone, Copy, Debug)]
struct ScreenVertex {
    x: f32,
    y: f32,
    z: f32,
    shade: f32,
    color: Vec3,
    uv: Vec2,
    tex_id: u16,
}

fn cube_faces(x: f32, y: f32, z: f32, color: Vec3) -> [(CubeFace, Face); 6] {
    let p000 = Vec3::new(x, y, z);
    let p001 = Vec3::new(x, y, z + 1.0);
    let p010 = Vec3::new(x, y + 1.0, z);
    let p011 = Vec3::new(x, y + 1.0, z + 1.0);
    let p100 = Vec3::new(x + 1.0, y, z);
    let p101 = Vec3::new(x + 1.0, y, z + 1.0);
    let p110 = Vec3::new(x + 1.0, y + 1.0, z);
    let p111 = Vec3::new(x + 1.0, y + 1.0, z + 1.0);
    let uvs = uv_quad();

    [
        (
            CubeFace::NegX,
            make_face(Vec3::new(-1.0, 0.0, 0.0), p000, p010, p011, p001, color, uvs),
        ),
        (
            CubeFace::PosX,
            make_face(Vec3::new(1.0, 0.0, 0.0), p100, p101, p111, p110, color, uvs),
        ),
        (
            CubeFace::NegY,
            make_face(Vec3::new(0.0, -1.0, 0.0), p000, p001, p101, p100, color, uvs),
        ),
        (
            CubeFace::PosY,
            make_face(Vec3::new(0.0, 1.0, 0.0), p010, p110, p111, p011, color, uvs),
        ),
        (
            CubeFace::NegZ,
            make_face(Vec3::new(0.0, 0.0, -1.0), p000, p100, p110, p010, color, uvs),
        ),
        (
            CubeFace::PosZ,
            make_face(Vec3::new(0.0, 0.0, 1.0), p001, p011, p111, p101, color, uvs),
        ),
    ]
}

fn uv_quad() -> [Vec2; 4] {
    // v=0 at top of PNG (Minecraft convention).
    [
        Vec2::new(0.0, 0.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 0.0),
    ]
}

fn make_face(
    normal: Vec3,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    d: Vec3,
    color: Vec3,
    uvs: [Vec2; 4],
) -> Face {
    Face {
        normal,
        v0: ViewVertex {
            pos: a,
            normal,
            color,
            uv: uvs[0],
            tex_id: 0,
        },
        v1: ViewVertex {
            pos: b,
            normal,
            color,
            uv: uvs[1],
            tex_id: 0,
        },
        v2: ViewVertex {
            pos: c,
            normal,
            color,
            uv: uvs[2],
            tex_id: 0,
        },
        v3: ViewVertex {
            pos: d,
            normal,
            color,
            uv: uvs[3],
            tex_id: 0,
        },
    }
}

fn project(v: ViewVertex, vp: Mat4, width: f32, height: f32) -> Option<ScreenVertex> {
    let p = vp * Vec4::new(v.pos.x, v.pos.y, v.pos.z, 1.0);
    if p.w <= 0.0 {
        return None;
    }
    let ndc = p.truncate() / p.w;
    if ndc.x < -1.5 || ndc.x > 1.5 || ndc.y < -1.5 || ndc.y > 1.5 || ndc.z < -2.0 || ndc.z > 2.0 {
        return None;
    }
    let sx = (ndc.x * 0.5 + 0.5) * (width - 1.0);
    let sy = (1.0 - (ndc.y * 0.5 + 0.5)) * (height - 1.0);
    // Minecraft flat face shading (not Lambert).
    let shade = mc_vertex_shade(v.normal);
    Some(ScreenVertex {
        x: sx,
        y: sy,
        z: ndc.z,
        shade,
        color: v.color,
        uv: v.uv,
        tex_id: v.tex_id,
    })
}

fn draw_triangle(
    img: &mut RgbaImage,
    zbuf: &mut [f32],
    a: ScreenVertex,
    b: ScreenVertex,
    c: ScreenVertex,
    textures: &[RgbaImage],
) {
    let p0 = Vec2::new(a.x, a.y);
    let p1 = Vec2::new(b.x, b.y);
    let p2 = Vec2::new(c.x, c.y);
    let area = edge(p0, p1, p2);
    if area <= 1e-5 {
        return; // degenerate or back-facing
    }
    let min_x = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as u32;
    let min_y = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as u32;
    let max_x = p0.x.max(p1.x).max(p2.x).ceil().min((img.width() - 1) as f32) as u32;
    let max_y = p0.y.max(p1.y).max(p2.y).ceil().min((img.height() - 1) as f32) as u32;
    if min_x > max_x || min_y > max_y {
        return;
    }

    let tex = if a.tex_id > 0 {
        textures.get((a.tex_id as usize) - 1)
    } else {
        None
    };

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(p1, p2, p) / area;
            let w1 = edge(p2, p0, p) / area;
            let w2 = edge(p0, p1, p) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * c.z;
            let idx = (y * img.width() + x) as usize;
            if z >= zbuf[idx] {
                continue;
            }

            let shade = w0 * a.shade + w1 * b.shade + w2 * c.shade;
            let vert_color =
                (w0 * a.color + w1 * b.color + w2 * c.color).clamp(Vec3::ZERO, Vec3::ONE);
            let (rgb, alpha) = if let Some(tex) = tex {
                let uv = w0 * a.uv + w1 * b.uv + w2 * c.uv;
                let (sample, a8) = sample_nearest(tex, uv.x, uv.y);
                if a8 < 16 {
                    continue; // cutout / glass holes
                }
                // Vertex color is tint (usually ONE; grass/leaves multiply).
                ((sample * vert_color) * shade, a8)
            } else {
                (vert_color * shade, 255)
            };
            let col = rgb.clamp(Vec3::ZERO, Vec3::ONE);
            zbuf[idx] = z;
            img.put_pixel(
                x,
                y,
                Rgba([
                    (col.x * 255.0) as u8,
                    (col.y * 255.0) as u8,
                    (col.z * 255.0) as u8,
                    alpha,
                ]),
            );
        }
    }
}

fn sample_nearest(tex: &RgbaImage, u: f32, v: f32) -> (Vec3, u8) {
    let w = tex.width().max(1);
    let h = tex.height().max(1);
    let uf = u.fract();
    let vf = v.fract();
    let u_pos = if uf < 0.0 { uf + 1.0 } else { uf };
    let v_pos = if vf < 0.0 { vf + 1.0 } else { vf };
    let x = ((u_pos * w as f32) as u32).min(w - 1);
    let y = ((v_pos * h as f32) as u32).min(h - 1);
    let p = tex.get_pixel(x, y).0;
    (
        Vec3::new(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0),
        p[3],
    )
}

fn edge(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)
}

fn triple_to_vec3(t: (f32, f32, f32)) -> Vec3 {
    Vec3::new(t.0, t.1, t.2)
}

fn color_for_block(b: &BlockState) -> Vec3 {
    let name = b
        .name
        .rsplit_once(':')
        .map(|(_, n)| n)
        .unwrap_or(b.name.as_str());
    let rgb = if name.contains("grass") {
        [0x6f, 0xb8, 0x56]
    } else if name.contains("leaves") {
        [0x5c, 0x9c, 0x49]
    } else if name.contains("water") {
        [0x3f, 0x76, 0xe4]
    } else if name.contains("lava") {
        [0xe2, 0x58, 0x22]
    } else if name.contains("sand") {
        [0xdf, 0xd0, 0x90]
    } else if name.contains("dirt") || name.contains("mud") {
        [0x82, 0x5a, 0x3a]
    } else if name.contains("log") || name.contains("wood") || name.contains("planks") {
        [0x9a, 0x74, 0x52]
    } else if name.contains("stone") || name.contains("deepslate") || name.contains("cobble") {
        [0x7f, 0x7f, 0x7f]
    } else if name.contains("snow") || name.contains("quartz") {
        [0xe8, 0xee, 0xf4]
    } else if name.contains("glass") {
        [0xc8, 0xe6, 0xf3]
    } else {
        hash_color(name)
    };
    Vec3::new(rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0)
}

fn hash_color(name: &str) -> [u8; 3] {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let r = 64 + ((h & 0x7f) as u8);
    let g = 64 + (((h >> 8) & 0x7f) as u8);
    let b = 64 + (((h >> 16) & 0x7f) as u8);
    [r, g, b]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::BlockTextureAtlas;
    use std::collections::HashSet;
    use std::fs;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn mc_face_shade_ladder_top_sides_bottom() {
        let top = mc_face_shade(Vec3::Y);
        let south = mc_face_shade(Vec3::Z);
        let east = mc_face_shade(Vec3::X);
        let bottom = mc_face_shade(-Vec3::Y);
        assert!((top - 1.0).abs() < 1e-5);
        assert!((south - 0.8).abs() < 1e-5);
        assert!((east - 0.6).abs() < 1e-5);
        assert!((bottom - 0.5).abs() < 1e-5);
        assert!(top > south && south > east && east > bottom);
        // Opposite cardinals share the same step.
        assert!((mc_face_shade(-Vec3::Z) - south).abs() < 1e-5);
        assert!((mc_face_shade(-Vec3::X) - east).abs() < 1e-5);
    }

    #[test]
    fn mc_lightmap_full_sky_is_bright() {
        assert!((mc_lightmap_brightness(15, 0) - 1.0).abs() < 1e-3);
        assert!(mc_lightmap_brightness(15, 0) > mc_lightmap_brightness(8, 0));
        assert!(mc_lightmap_brightness(8, 0) > mc_lightmap_brightness(0, 0));
    }

    #[test]
    fn rasterized_faces_keep_mc_brightness_order() {
        // White unit quads facing +Y / +Z / +X / -Y; sample mean luminance.
        let white = Vec3::ONE;
        let mk = |normal: Vec3, origin: Vec3| -> ViewTriangle {
            // Small quad centered near origin, facing `normal`.
            let t = if normal.y.abs() > 0.5 {
                Vec3::X
            } else {
                Vec3::Y
            };
            let b = normal.cross(t).normalize();
            let t = b.cross(normal).normalize();
            let o = origin;
            ViewTriangle {
                a: ViewVertex {
                    pos: o - t - b,
                    normal,
                    color: white,
                    uv: Vec2::ZERO,
                    tex_id: 0,
                },
                b: ViewVertex {
                    pos: o - t + b,
                    normal,
                    color: white,
                    uv: Vec2::ZERO,
                    tex_id: 0,
                },
                c: ViewVertex {
                    pos: o + t + b,
                    normal,
                    color: white,
                    uv: Vec2::ZERO,
                    tex_id: 0,
                },
            }
        };
        let mean_luma = |normal: Vec3| -> f32 {
            let mesh = ViewMesh {
                triangles: vec![mk(normal, Vec3::ZERO)],
                blocks: 1,
                faces: 1,
                from: (-1, -1, -1),
                to: (1, 1, 1),
                textures: vec![],
            };
            // Offset camera so look_at up=Y is never parallel to view (top/bottom).
            let cam = normal * 3.0 + Vec3::new(0.4, 0.0, 0.25);
            let frame = rasterize_view_mesh(&mesh, 64, 64, cam, Vec3::ZERO);
            let mut sum = 0.0f32;
            let mut n = 0u32;
            for i in (0..frame.pixels.len()).step_by(4) {
                let r = frame.pixels[i] as f32;
                let g = frame.pixels[i + 1] as f32;
                let b = frame.pixels[i + 2] as f32;
                // Skip sky pixels.
                if (r - SKY[0] as f32).abs() < 1.0
                    && (g - SKY[1] as f32).abs() < 1.0
                    && (b - SKY[2] as f32).abs() < 1.0
                {
                    continue;
                }
                sum += 0.299 * r + 0.587 * g + 0.114 * b;
                n += 1;
            }
            assert!(n > 20, "expected shaded face pixels, got {n}");
            sum / n as f32
        };
        let top = mean_luma(Vec3::Y);
        let south = mean_luma(Vec3::Z);
        let east = mean_luma(Vec3::X);
        let bottom = mean_luma(-Vec3::Y);
        assert!(
            top > south && south > east && east > bottom,
            "luma order top={top} south={south} east={east} bottom={bottom}"
        );
    }

    #[test]
    fn hash_color_is_stable() {
        assert_eq!(hash_color("mystery_block"), hash_color("mystery_block"));
    }

    #[test]
    fn known_palette_for_stone() {
        let c = color_for_block(&BlockState::new("minecraft:stone"));
        assert!(c.x > 0.45 && c.x < 0.55);
        assert!(c.y > 0.45 && c.y < 0.55);
        assert!(c.z > 0.45 && c.z < 0.55);
    }

    #[test]
    fn face_cull_for_adjacent_blocks() {
        let mut occ = HashSet::new();
        occ.insert((0, 0, 0));
        occ.insert((1, 0, 0));
        assert_eq!(visible_faces(&occ), 10);
    }

    #[test]
    fn orbit_camera_moves_with_yaw() {
        let look = Vec3::ZERO;
        let a = orbit_camera(look, 0.0, 20.0, 10.0);
        let b = orbit_camera(look, 90.0, 20.0, 10.0);
        assert!((a - b).length() > 1.0);
    }

    #[test]
    fn preview_watch_token_changes_on_region_touch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("world/region")).unwrap();
        fs::create_dir_all(root.join("history")).unwrap();
        fs::write(root.join("meta.json"), r#"{"id":"t","source_world":".","dim":".","created_at":"0","dirty":false,"next_action_hint":1}"#).unwrap();
        fs::write(root.join("HEAD"), "0\n").unwrap();
        let a = preview_watch_token(root).unwrap();
        let mut f = fs::File::create(root.join("world/region/r.0.0.mca")).unwrap();
        f.write_all(b"fake-mca").unwrap();
        f.sync_all().unwrap();
        // Ensure mtime can advance on coarse FS clocks.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(root.join("world/region/r.0.0.mca"))
            .unwrap();
        f.write_all(b"!").unwrap();
        f.sync_all().unwrap();
        let b = preview_watch_token(root).unwrap();
        assert_ne!(a.region_stamp, b.region_stamp);
    }

    #[test]
    fn preview_watch_token_tracks_head_and_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("world/region")).unwrap();
        fs::create_dir_all(root.join("history")).unwrap();
        fs::write(
            root.join("meta.json"),
            r#"{"id":"t","source_world":".","dim":".","created_at":"0","dirty":false,"next_action_hint":1}"#,
        )
        .unwrap();
        fs::write(root.join("HEAD"), "0\n").unwrap();
        let a = preview_watch_token(root).unwrap();
        fs::write(
            root.join("meta.json"),
            r#"{"id":"t","source_world":".","dim":".","created_at":"0","dirty":true,"next_action_hint":1}"#,
        )
        .unwrap();
        fs::write(root.join("HEAD"), "2\n").unwrap();
        fs::write(root.join("history/1.json"), "{}").unwrap();
        fs::write(root.join("history/2.json"), "{}").unwrap();
        let b = preview_watch_token(root).unwrap();
        assert!(b.dirty);
        assert_eq!(b.head, 2);
        assert_eq!(b.history_size, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn rasterize_empty_mesh_is_sky() {
        let mesh = ViewMesh::default();
        let frame = rasterize_view_mesh(&mesh, 64, 64, Vec3::new(0.0, 10.0, 10.0), Vec3::ZERO);
        assert_eq!(frame.pixels.len(), 64 * 64 * 4);
        assert_eq!(&frame.pixels[0..4], &SKY);
    }

    #[test]
    fn rasterize_textured_quad_samples_png() {
        let mut tex = RgbaImage::new(2, 2);
        for (x, y, p) in tex.enumerate_pixels_mut() {
            *p = if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 255, 0, 255])
            };
            let _ = y;
        }
        let color = Vec3::ONE;
        let n = Vec3::new(0.0, 0.0, 1.0);
        let mesh = ViewMesh {
            triangles: vec![ViewTriangle {
                a: ViewVertex {
                    pos: Vec3::new(-1.0, -1.0, 0.0),
                    normal: n,
                    color,
                    uv: Vec2::new(0.0, 1.0),
                    tex_id: 1,
                },
                b: ViewVertex {
                    pos: Vec3::new(1.0, -1.0, 0.0),
                    normal: n,
                    color,
                    uv: Vec2::new(1.0, 1.0),
                    tex_id: 1,
                },
                c: ViewVertex {
                    pos: Vec3::new(-1.0, 1.0, 0.0),
                    normal: n,
                    color,
                    uv: Vec2::new(0.0, 0.0),
                    tex_id: 1,
                },
            }],
            blocks: 1,
            faces: 1,
            from: (-1, -1, 0),
            to: (1, 1, 0),
            textures: vec![tex],
        };
        let frame = rasterize_view_mesh(
            &mesh,
            64,
            64,
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::ZERO,
        );
        // Center pixel should not be sky after raster.
        let i = ((32 * 64 + 32) * 4) as usize;
        assert_ne!(&frame.pixels[i..i + 3], &SKY[0..3]);
    }

    #[test]
    fn atlas_missing_falls_back_to_palette_label() {
        let (atlas, label) = resolve_atlas_for_view(Some(Path::new("/nope/missing.jar")), false);
        // resolve tries explicit then auto-detect — may still find a real jar on developer machines.
        if atlas.is_none() {
            assert!(label.starts_with("palette"), "{label}");
        }
    }

    #[test]
    fn texture_labels_distinguish_jar_and_palette() {
        assert_eq!(
            resolve_atlas_cli(None, None, true).1,
            "palette reason=no-textures"
        );
        let dir = tempfile::tempdir().unwrap();
        // minimal jar via assets tests pattern
        let jar = dir.path().join("t.jar");
        {
            use std::io::Write;
            use zip::write::SimpleFileOptions;
            use zip::ZipWriter;
            let mut img = RgbaImage::new(2, 2);
            for p in img.pixels_mut() {
                *p = Rgba([1, 2, 3, 255]);
            }
            let png = dir.path().join("stone.png");
            img.save(&png).unwrap();
            let file = fs::File::create(&jar).unwrap();
            let mut zip = ZipWriter::new(file);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("assets/minecraft/textures/block/stone.png", opts)
                .unwrap();
            zip.write_all(&fs::read(&png).unwrap()).unwrap();
            zip.finish().unwrap();
        }
        let (_a, label) = resolve_atlas_cli(Some(&jar), None, false);
        assert!(label.starts_with("jar path="), "{label}");
        assert!(label.contains("t.jar"), "{label}");
    }

    #[test]
    fn resolve_view_max_cells_honors_explicit() {
        assert_eq!(resolve_view_max_cells(Some(12345)), 12345);
        assert!(resolve_view_max_cells(None) >= VIEW_MAX_CELLS_DEFAULT);
    }

    #[test]
    fn fixture_jar_packs_into_mesh_textures() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("t.jar");
        {
            let mut img = RgbaImage::new(2, 2);
            for p in img.pixels_mut() {
                *p = Rgba([0xAA, 0xAA, 0xAA, 0xFF]);
            }
            let png = dir.path().join("stone.png");
            img.save(&png).unwrap();
            let file = fs::File::create(&jar).unwrap();
            let mut zip = ZipWriter::new(file);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("assets/minecraft/textures/block/stone.png", opts)
                .unwrap();
            zip.write_all(&fs::read(&png).unwrap()).unwrap();
            zip.finish().unwrap();
        }
        let mut atlas = BlockTextureAtlas::open(&jar).unwrap();
        let img = atlas
            .image_for_block_face("minecraft:stone", CubeFace::PosY)
            .unwrap();
        assert_eq!(img.get_pixel(0, 0).0[0], 0xAA);
    }

    fn visible_faces(occ: &HashSet<(i32, i32, i32)>) -> usize {
        let dirs = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        let mut faces = 0usize;
        for &(x, y, z) in occ {
            for (dx, dy, dz) in dirs {
                if !occ.contains(&(x + dx, y + dy, z + dz)) {
                    faces += 1;
                }
            }
        }
        faces
    }
}
