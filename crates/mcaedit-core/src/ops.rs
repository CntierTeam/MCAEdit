//! Region geometry helpers (AABB / sphere / cylinder).

use crate::blockstate::BlockState;

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

impl Aabb {
    pub fn from_corners(x1: i32, y1: i32, z1: i32, x2: i32, y2: i32, z2: i32) -> Self {
        Self {
            min_x: x1.min(x2),
            min_y: y1.min(y2),
            min_z: z1.min(z2),
            max_x: x1.max(x2),
            max_y: y1.max(y2),
            max_z: z1.max(z2),
        }
    }

    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        (self.min_x..=self.max_x).contains(&x)
            && (self.min_y..=self.max_y).contains(&y)
            && (self.min_z..=self.max_z).contains(&z)
    }

    pub fn on_wall(&self, x: i32, _y: i32, z: i32) -> bool {
        x == self.min_x || x == self.max_x || z == self.min_z || z == self.max_z
    }

    pub fn on_outline(&self, x: i32, y: i32, z: i32) -> bool {
        x == self.min_x
            || x == self.max_x
            || y == self.min_y
            || y == self.max_y
            || z == self.min_z
            || z == self.max_z
    }

    pub fn interior(&self, x: i32, y: i32, z: i32) -> bool {
        self.contains(x, y, z) && !self.on_outline(x, y, z)
    }

    pub fn volume(&self) -> u64 {
        let dx = (self.max_x - self.min_x + 1) as u64;
        let dy = (self.max_y - self.min_y + 1) as u64;
        let dz = (self.max_z - self.min_z + 1) as u64;
        dx * dy * dz
    }
}

/// `--match air` / `minecraft:air` ⇒ any air-like; else exact BlockState equality.
pub fn matches_filter(block: &BlockState, filter: &BlockState) -> bool {
    if filter.is_air_like() || filter.name == "air" {
        return block.is_air_like();
    }
    block == filter
}

pub fn parse_match_filter(s: &str) -> Result<BlockState, String> {
    let t = s.trim();
    if t == "air" {
        return Ok(BlockState::air());
    }
    BlockState::parse(t)
}

/// Iterate AABB cells; `keep` returns true if the cell should receive `block`.
pub fn aabb_positions(box_: Aabb, keep: impl Fn(i32, i32, i32) -> bool) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for y in box_.min_y..=box_.max_y {
        for z in box_.min_z..=box_.max_z {
            for x in box_.min_x..=box_.max_x {
                if keep(x, y, z) {
                    out.push((x, y, z));
                }
            }
        }
    }
    out
}

pub fn sphere_positions(
    cx: i32,
    cy: i32,
    cz: i32,
    radius: f64,
    hollow: bool,
) -> Vec<(i32, i32, i32)> {
    let r = radius.max(0.0);
    let ri = r.ceil() as i32;
    let r2 = r * r;
    let inner = if hollow {
        let ir = (r - 1.0).max(0.0);
        ir * ir
    } else {
        -1.0
    };
    let mut out = Vec::new();
    for y in (cy - ri)..=(cy + ri) {
        for z in (cz - ri)..=(cz + ri) {
            for x in (cx - ri)..=(cx + ri) {
                let dx = (x - cx) as f64 + 0.5;
                let dy = (y - cy) as f64 + 0.5;
                let dz = (z - cz) as f64 + 0.5;
                let d2 = dx * dx + dy * dy + dz * dz;
                if d2 <= r2 && (!hollow || d2 >= inner) {
                    out.push((x, y, z));
                }
            }
        }
    }
    out
}

/// Vertical cylinder: circle in XZ, height along Y from `y_base` inclusive for `height` blocks.
pub fn cyl_positions(
    cx: i32,
    cz: i32,
    y_base: i32,
    radius: f64,
    height: i32,
    hollow: bool,
) -> Vec<(i32, i32, i32)> {
    let r = radius.max(0.0);
    let ri = r.ceil() as i32;
    let r2 = r * r;
    let inner = if hollow {
        let ir = (r - 1.0).max(0.0);
        ir * ir
    } else {
        -1.0
    };
    let h = height.max(1);
    let mut out = Vec::new();
    for y in y_base..(y_base + h) {
        for z in (cz - ri)..=(cz + ri) {
            for x in (cx - ri)..=(cx + ri) {
                let dx = (x - cx) as f64 + 0.5;
                let dz = (z - cz) as f64 + 0.5;
                let d2 = dx * dx + dz * dz;
                if d2 <= r2 && (!hollow || d2 >= inner) {
                    out.push((x, y, z));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walls_exclude_interior() {
        let b = Aabb::from_corners(0, 0, 0, 2, 2, 2);
        let walls = aabb_positions(b, |x, y, z| b.on_wall(x, y, z));
        assert!(walls.contains(&(0, 1, 1)));
        assert!(!walls.iter().any(|&(x, y, z)| x == 1 && y == 1 && z == 1));
    }

    #[test]
    fn sphere_solid_includes_center() {
        let pts = sphere_positions(0, 0, 0, 2.0, false);
        assert!(pts.contains(&(0, 0, 0)));
    }
}
