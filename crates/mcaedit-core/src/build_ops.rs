//! High-level building helpers (柱网 / 瓦垄 / stairs) for LLM-friendly edits.

use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::ops::Aabb;
use crate::pattern::Pattern;
use crate::world::WorldView;
use crate::Action;

fn grid_axis(min: i32, max: i32, spacing: i32) -> Vec<i32> {
    let spacing = spacing.max(1);
    let mut out = Vec::new();
    let mut c = min;
    while c <= max {
        out.push(c);
        c += spacing;
    }
    out
}

/// Place vertical pillars on a spacing grid inside AABB (柱网 / colonnade).
#[allow(clippy::too_many_arguments)]
pub fn grid_columns(
    world: &mut WorldView<'_>,
    x1: i32,
    y1: i32,
    z1: i32,
    x2: i32,
    y2: i32,
    z2: i32,
    spacing_x: i32,
    spacing_z: i32,
    pattern: Pattern,
) -> Result<Action> {
    let box_ = Aabb::from_corners(x1, y1, z1, x2, y2, z2);
    let xs = grid_axis(box_.min_x, box_.max_x, spacing_x);
    let zs = grid_axis(box_.min_z, box_.max_z, spacing_z);
    let mut positions = Vec::with_capacity(xs.len() * zs.len() * ((box_.max_y - box_.min_y + 1) as usize));
    for &x in &xs {
        for &z in &zs {
            for y in box_.min_y..=box_.max_y {
                positions.push((x, y, z));
            }
        }
    }
    let changes = world.plan_set_pattern_positions(&positions, &pattern, None)?;
    world.apply_changes(
        changes,
        format!(
            "grid ({},{},{})..({},{},{}) spacing={},{}",
            box_.min_x,
            box_.min_y,
            box_.min_z,
            box_.max_x,
            box_.max_y,
            box_.max_z,
            spacing_x.max(1),
            spacing_z.max(1)
        ),
    )
}

/// Repeating roof tile rows along an axis (瓦垄).
#[allow(clippy::too_many_arguments)]
pub fn roof_rows(
    world: &mut WorldView<'_>,
    x1: i32,
    y1: i32,
    z1: i32,
    x2: i32,
    y2: i32,
    z2: i32,
    axis: char,
    period: i32,
    block: BlockState,
    stairs: Option<BlockState>,
    stairs_facing: &str,
    stairs_half: &str,
) -> Result<Action> {
    let period = period.max(1);
    let axis = axis.to_ascii_lowercase();
    if axis != 'x' && axis != 'z' {
        return Err(Error::msg("roof-rows --axis must be x or z"));
    }
    let box_ = Aabb::from_corners(x1, y1, z1, x2, y2, z2);
    let facing = normalize_facing(stairs_facing)?;
    let half = normalize_half(stairs_half)?;
    let stair_block = stairs.map(|base| {
        with_props(
            base,
            &[
                ("facing", facing),
                ("half", half),
                ("shape", "straight"),
            ],
        )
    });

    let mut positions_block = Vec::new();
    let mut positions_stair = Vec::new();
    for y in box_.min_y..=box_.max_y {
        for z in box_.min_z..=box_.max_z {
            for x in box_.min_x..=box_.max_x {
                let along = if axis == 'x' {
                    x - box_.min_x
                } else {
                    z - box_.min_z
                };
                if along % period == 0 {
                    positions_block.push((x, y, z));
                } else if stair_block.is_some() {
                    positions_stair.push((x, y, z));
                }
            }
        }
    }
    let mut changes =
        world.plan_set_pattern_positions(&positions_block, &Pattern::single(block), None)?;
    if let Some(s) = stair_block {
        changes.extend(world.plan_set_pattern_positions(
            &positions_stair,
            &Pattern::single(s),
            None,
        )?);
    }
    world.apply_changes(
        changes,
        format!(
            "roof-rows ({},{},{})..({},{},{}) axis={axis} period={period}",
            box_.min_x, box_.min_y, box_.min_z, box_.max_x, box_.max_y, box_.max_z
        ),
    )
}

/// Fill AABB with stairs of given facing/half/shape.
#[allow(clippy::too_many_arguments)]
pub fn stairs_fill(
    world: &mut WorldView<'_>,
    x1: i32,
    y1: i32,
    z1: i32,
    x2: i32,
    y2: i32,
    z2: i32,
    block: BlockState,
    facing: &str,
    half: &str,
    shape: &str,
) -> Result<Action> {
    let facing = normalize_facing(facing)?;
    let half = normalize_half(half)?;
    let shape = normalize_shape(shape)?;
    let state = with_props(block, &[("facing", facing), ("half", half), ("shape", shape)]);
    let box_ = Aabb::from_corners(x1, y1, z1, x2, y2, z2);
    let positions = crate::ops::aabb_positions(box_, |_, _, _| true);
    let changes = world.plan_set_pattern_positions(&positions, &Pattern::single(state), None)?;
    world.apply_changes(
        changes,
        format!(
            "stairs ({},{},{})..({},{},{}) facing={facing} half={half} shape={shape}",
            box_.min_x, box_.min_y, box_.min_z, box_.max_x, box_.max_y, box_.max_z
        ),
    )
}

fn with_props(mut base: BlockState, props: &[(&str, &str)]) -> BlockState {
    let mut map = base.properties;
    for (k, v) in props {
        map.insert((*k).to_string(), (*v).to_string());
    }
    base.properties = map;
    base
}

fn normalize_facing(s: &str) -> Result<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "north" | "n" => Ok("north"),
        "south" | "s" => Ok("south"),
        "east" | "e" => Ok("east"),
        "west" | "w" => Ok("west"),
        other => Err(Error::msg(format!(
            "bad stairs facing `{other}` (north|south|east|west)"
        ))),
    }
}

fn normalize_half(s: &str) -> Result<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "bottom" | "bot" | "lower" => Ok("bottom"),
        "top" | "upper" => Ok("top"),
        other => Err(Error::msg(format!("bad stairs half `{other}` (bottom|top)"))),
    }
}

fn normalize_shape(s: &str) -> Result<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "straight" => Ok("straight"),
        "inner_left" | "inner-left" => Ok("inner_left"),
        "inner_right" | "inner-right" => Ok("inner_right"),
        "outer_left" | "outer-left" => Ok("outer_left"),
        "outer_right" | "outer-right" => Ok("outer_right"),
        other => Err(Error::msg(format!(
            "bad stairs shape `{other}` (straight|inner_left|inner_right|outer_left|outer_right)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_aliases() {
        assert_eq!(normalize_facing("N").unwrap(), "north");
        assert_eq!(normalize_half("top").unwrap(), "top");
        assert_eq!(normalize_shape("straight").unwrap(), "straight");
    }

    #[test]
    fn grid_axis_steps() {
        assert_eq!(grid_axis(0, 10, 4), vec![0, 4, 8]);
    }
}
