use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::world::WorldView;
use glam::{Mat4, Vec2, Vec3, Vec4};
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

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

#[derive(Clone, Copy, Debug)]
struct Vertex {
    pos: Vec3,
    normal: Vec3,
    color: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct Triangle(Vertex, Vertex, Vertex);

#[derive(Clone, Copy, Debug)]
struct ScreenVertex {
    x: f32,
    y: f32,
    z: f32,
    shade: f32,
    color: Vec3,
}

#[derive(Default)]
struct MeshStats {
    blocks: usize,
    faces: usize,
}

const SKY: [u8; 4] = [0x78, 0xA7, 0xFF, 0xFF];

impl<'a> WorldView<'a> {
    pub fn view_screenshot(&self, req: &ViewScreenshotRequest) -> Result<ViewScreenshotResult> {
        if req.width < 64 || req.height < 64 {
            return Err(Error::msg("width/height must be >= 64"));
        }
        let (min_x, max_x) = (req.from.0.min(req.to.0), req.from.0.max(req.to.0));
        let (min_y, max_y) = (req.from.1.min(req.to.1), req.from.1.max(req.to.1));
        let (min_z, max_z) = (req.from.2.min(req.to.2), req.from.2.max(req.to.2));
        let sx = (max_x - min_x + 1) as usize;
        let sy = (max_y - min_y + 1) as usize;
        let sz = (max_z - min_z + 1) as usize;
        let cells = sx.saturating_mul(sy).saturating_mul(sz);
        if cells == 0 || cells > 48 * 48 * 48 {
            return Err(Error::msg("view box too large (max 48^3 cells)"));
        }

        let mut tris = Vec::new();
        let mut stats = MeshStats::default();
        self.build_mesh(min_x, min_y, min_z, max_x, max_y, max_z, &mut tris, &mut stats)?;
        let look = req.look.map(triple_to_vec3).unwrap_or_else(|| {
            Vec3::new(
                (min_x + max_x) as f32 * 0.5 + 0.5,
                (min_y + max_y) as f32 * 0.5 + 0.5,
                (min_z + max_z) as f32 * 0.5 + 0.5,
            )
        });
        let span = (sx.max(sy).max(sz) as f32).max(8.0);
        let camera = req
            .camera
            .map(triple_to_vec3)
            .unwrap_or(look + Vec3::new(0.0, span * 1.2, span * 0.6));

        let img = rasterize(&tris, req.width, req.height, camera, look);
        if let Some(parent) = req.out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        img.save(&req.out)
            .map_err(|e| Error::msg(format!("save png failed: {e}")))?;
        Ok(ViewScreenshotResult {
            out: req.out.clone(),
            width: req.width,
            height: req.height,
            blocks: stats.blocks,
            faces: stats.faces,
            triangles: tris.len(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_mesh(
        &self,
        min_x: i32,
        min_y: i32,
        min_z: i32,
        max_x: i32,
        max_y: i32,
        max_z: i32,
        out: &mut Vec<Triangle>,
        stats: &mut MeshStats,
    ) -> Result<()> {
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = self.get_block(x, y, z)?;
                    if b.is_air_like() {
                        continue;
                    }
                    stats.blocks += 1;
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
                            stats.faces += 1;
                            out.push(Triangle(face.v0, face.v1, face.v2));
                            out.push(Triangle(face.v0, face.v2, face.v3));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Face {
    normal: Vec3,
    v0: Vertex,
    v1: Vertex,
    v2: Vertex,
    v3: Vertex,
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
        v0: Vertex {
            pos: a,
            normal,
            color,
        },
        v1: Vertex {
            pos: b,
            normal,
            color,
        },
        v2: Vertex {
            pos: c,
            normal,
            color,
        },
        v3: Vertex {
            pos: d,
            normal,
            color,
        },
    }
}

fn rasterize(
    tris: &[Triangle],
    width: u32,
    height: u32,
    camera: Vec3,
    look: Vec3,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba(SKY));
    let mut zbuf = vec![f32::INFINITY; (width * height) as usize];
    let light_dir = Vec3::new(0.45, 1.0, 0.35).normalize();
    let view = Mat4::look_at_rh(camera, look, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0f32.to_radians(), width as f32 / height as f32, 0.1, 8192.0);
    let vp = proj * view;

    for tri in tris {
        let Some(a) = project(tri.0, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        let Some(b) = project(tri.1, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        let Some(c) = project(tri.2, vp, width as f32, height as f32, light_dir) else {
            continue;
        };
        draw_triangle(&mut img, &mut zbuf, a, b, c);
    }
    img
}

fn project(v: Vertex, vp: Mat4, width: f32, height: f32, light_dir: Vec3) -> Option<ScreenVertex> {
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
