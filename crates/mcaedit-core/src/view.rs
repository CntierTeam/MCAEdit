use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::region::{parse_region_name, RegionStore};
use crate::session::Session;
use crate::world::WorldView;
use glam::{Mat4, Vec2, Vec3, Vec4};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Max AABB edge volume for offline mesh builds (screenshot + preview).
pub const VIEW_MAX_CELLS: usize = 48 * 48 * 48;

#[derive(Clone, Debug)]
pub struct ViewScreenshotRequest {
    pub from: (i32, i32, i32),
    pub to: (i32, i32, i32),
    pub width: u32,
    pub height: u32,
    pub camera: Option<(f32, f32, f32)>,
    pub look: Option<(f32, f32, f32)>,
    pub out: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ViewScreenshotResult {
    pub out: PathBuf,
    pub width: u32,
    pub height: u32,
    pub blocks: usize,
    pub faces: usize,
    pub triangles: usize,
}

/// Shared face-culled mesh used by screenshot and live preview.
#[derive(Clone, Debug, Default)]
pub struct ViewMesh {
    pub triangles: Vec<ViewTriangle>,
    pub blocks: usize,
    pub faces: usize,
    pub from: (i32, i32, i32),
    pub to: (i32, i32, i32),
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
        let mesh = self.build_view_mesh(req.from, req.to)?;
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
        })
    }

    /// Build a face-culled cube mesh for an AABB (shared by screenshot + preview).
    pub fn build_view_mesh(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> Result<ViewMesh> {
        let (min_x, max_x) = (from.0.min(to.0), from.0.max(to.0));
        let (min_y, max_y) = (from.1.min(to.1), from.1.max(to.1));
        let (min_z, max_z) = (from.2.min(to.2), from.2.max(to.2));
        let sx = (max_x - min_x + 1) as usize;
        let sy = (max_y - min_y + 1) as usize;
        let sz = (max_z - min_z + 1) as usize;
        let cells = sx.saturating_mul(sy).saturating_mul(sz);
        if cells == 0 || cells > VIEW_MAX_CELLS {
            return Err(Error::msg("view box too large (max 48^3 cells)"));
        }

        let mut mesh = ViewMesh {
            triangles: Vec::new(),
            blocks: 0,
            faces: 0,
            from: (min_x, min_y, min_z),
            to: (max_x, max_y, max_z),
        };
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = self.get_block(x, y, z)?;
                    if b.is_air_like() {
                        continue;
                    }
                    mesh.blocks += 1;
                    let color = color_for_block(&b);
                    for face in cube_faces(x as f32, y as f32, z as f32, color) {
                        let nx = x + face.normal.x as i32;
                        let ny = y + face.normal.y as i32;
                        let nz = z + face.normal.z as i32;
                        let neighbor_is_air = if nx < min_x
                            || nx > max_x
                            || ny < min_y
                            || ny > max_y
                            || nz < min_z
                            || nz > max_z
                        {
                            true
                        } else {
                            self.get_block(nx, ny, nz)?.is_air_like()
                        };
                        if neighbor_is_air {
                            mesh.faces += 1;
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
        }
        Ok(mesh)
    }
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
    let light_dir = Vec3::new(0.45, 1.0, 0.35).normalize();
    let view = Mat4::look_at_rh(camera, look, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0f32.to_radians(), width as f32 / height as f32, 0.1, 8192.0);
    let vp = proj * view;

    for tri in &mesh.triangles {
        let Some(a) = project(tri.a, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        let Some(b) = project(tri.b, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        let Some(c) = project(tri.c, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        draw_triangle(&mut img, &mut zbuf, a, b, c);
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
    // Clamp XZ to 48 so volume stays within VIEW_MAX_CELLS with Y span 48.
    let mid_x = (min_x + max_x) / 2;
    let mid_z = (min_z + max_z) / 2;
    if max_x - min_x + 1 > 48 {
        min_x = mid_x - 23;
        max_x = mid_x + 24;
    }
    if max_z - min_z + 1 > 48 {
        min_z = mid_z - 23;
        max_z = mid_z + 24;
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
}

fn cube_faces(x: f32, y: f32, z: f32, color: Vec3) -> [Face; 6] {
    let p000 = Vec3::new(x, y, z);
    let p001 = Vec3::new(x, y, z + 1.0);
    let p010 = Vec3::new(x, y + 1.0, z);
    let p011 = Vec3::new(x, y + 1.0, z + 1.0);
    let p100 = Vec3::new(x + 1.0, y, z);
    let p101 = Vec3::new(x + 1.0, y, z + 1.0);
    let p110 = Vec3::new(x + 1.0, y + 1.0, z);
    let p111 = Vec3::new(x + 1.0, y + 1.0, z + 1.0);

    [
        make_face(Vec3::new(-1.0, 0.0, 0.0), p000, p010, p011, p001, color),
        make_face(Vec3::new(1.0, 0.0, 0.0), p100, p101, p111, p110, color),
        make_face(Vec3::new(0.0, -1.0, 0.0), p000, p001, p101, p100, color),
        make_face(Vec3::new(0.0, 1.0, 0.0), p010, p110, p111, p011, color),
        make_face(Vec3::new(0.0, 0.0, -1.0), p000, p100, p110, p010, color),
        make_face(Vec3::new(0.0, 0.0, 1.0), p001, p011, p111, p101, color),
    ]
}

fn make_face(normal: Vec3, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: Vec3) -> Face {
    Face {
        normal,
        v0: ViewVertex {
            pos: a,
            normal,
            color,
        },
        v1: ViewVertex {
            pos: b,
            normal,
            color,
        },
        v2: ViewVertex {
            pos: c,
            normal,
            color,
        },
        v3: ViewVertex {
            pos: d,
            normal,
            color,
        },
    }
}

fn project(
    v: ViewVertex,
    vp: Mat4,
    width: f32,
    height: f32,
    light_dir: Vec3,
) -> Option<ScreenVertex> {
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
    let shade = (0.28 + 0.72 * v.normal.normalize().dot(light_dir).max(0.0)).clamp(0.0, 1.0);
    Some(ScreenVertex {
        x: sx,
        y: sy,
        z: ndc.z,
        shade,
        color: v.color,
    })
}

fn draw_triangle(
    img: &mut RgbaImage,
    zbuf: &mut [f32],
    a: ScreenVertex,
    b: ScreenVertex,
    c: ScreenVertex,
) {
    let p0 = Vec2::new(a.x, a.y);
    let p1 = Vec2::new(b.x, b.y);
    let p2 = Vec2::new(c.x, c.y);
    let area = edge(p0, p1, p2);
    if area.abs() < 1e-5 {
        return;
    }
    let min_x = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as u32;
    let min_y = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as u32;
    let max_x = p0.x.max(p1.x).max(p2.x).ceil().min((img.width() - 1) as f32) as u32;
    let max_y = p0.y.max(p1.y).max(p2.y).ceil().min((img.height() - 1) as f32) as u32;
    if min_x > max_x || min_y > max_y {
        return;
    }

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
            zbuf[idx] = z;
            let col = (w0 * (a.color * a.shade) + w1 * (b.color * b.shade) + w2 * (c.color * c.shade))
                .clamp(Vec3::ZERO, Vec3::ONE);
            img.put_pixel(
                x,
                y,
                Rgba([
                    (col.x * 255.0) as u8,
                    (col.y * 255.0) as u8,
                    (col.z * 255.0) as u8,
                    255,
                ]),
            );
        }
    }
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
    use std::collections::HashSet;
    use std::fs;
    use std::io::Write;

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
