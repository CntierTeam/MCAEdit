use crate::action::{Action, ActionPayload, BiomeChange, BlockChange};
use crate::blockstate::BlockState;
use crate::chunk::{biome_cell_origin, ChunkData};
use crate::entity::{EntityRecord, EntityStore};
use crate::error::{Error, Result};
use crate::mask::Mask;
use crate::palette::SectionDiff;
use crate::pattern::Pattern;
use crate::region::{copy_region_if_needed, region_coords, RegionStore};
use crate::session::Session;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// World operations against a session working copy.
pub struct WorldView<'a> {
    pub session: &'a mut Session,
}

impl<'a> WorldView<'a> {
    pub fn new(session: &'a mut Session) -> Self {
        Self { session }
    }

    fn ensure_region(&self, chunk_x: i32, chunk_z: i32) -> Result<std::path::PathBuf> {
        let (rx, rz) = region_coords(chunk_x, chunk_z);
        copy_region_if_needed(
            &self.session.source_region_dir(),
            &self.session.work_region_dir(),
            rx,
            rz,
        )
    }

    fn ensure_entity_region(&self, chunk_x: i32, chunk_z: i32) -> Result<std::path::PathBuf> {
        let (rx, rz) = region_coords(chunk_x, chunk_z);
        copy_region_if_needed(
            &self.session.source_entities_dir(),
            &self.session.work_entities_dir(),
            rx,
            rz,
        )
    }

    pub fn load_chunk(&self, chunk_x: i32, chunk_z: i32) -> Result<ChunkData> {
        let path = self.ensure_region(chunk_x, chunk_z)?;
        let store = RegionStore::open(path);
        match store.read_chunk(chunk_x, chunk_z)? {
            Some(c) => Ok(c),
            None => Ok(ChunkData::empty_with_version(
                chunk_x,
                chunk_z,
                self.session
                    .meta
                    .data_version
                    .unwrap_or(crate::mc_version::DEFAULT_DATA_VERSION),
            )),
        }
    }

    pub fn save_chunk(&self, chunk: &ChunkData) -> Result<()> {
        let path = self.ensure_region(chunk.chunk_x, chunk.chunk_z)?;
        let mut store = RegionStore::open(path);
        store.write_chunk(chunk)
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Result<BlockState> {
        let cx = x >> 4;
        let cz = z >> 4;
        self.load_chunk(cx, cz)?.get_block(x, y, z)
    }

    pub fn set_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        block: BlockState,
    ) -> Result<Action> {
        let cx = x >> 4;
        let cz = z >> 4;
        let mut chunk = self.load_chunk(cx, cz)?;
        let before = chunk.set_block(x, y, z, block.clone())?;
        self.save_chunk(&chunk)?;
        let action = Action {
            id: 0,
            description: format!("set-block {x} {y} {z} -> {block}"),
            payload: ActionPayload::SetBlocks {
                changes: vec![BlockChange {
                    x,
                    y,
                    z,
                    before,
                    after: block,
                }],
            },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    /// Apply planned block changes (section-batched) and push one history entry.
    pub fn apply_changes(
        &mut self,
        changes: Vec<BlockChange>,
        description: impl Into<String>,
    ) -> Result<Action> {
        if changes.is_empty() {
            let action = Action {
                id: 0,
                description: description.into(),
                payload: ActionPayload::SetBlocks { changes },
            };
            let action = self.session.history.push(action)?;
            self.session.mark_dirty()?;
            return Ok(action);
        }

        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
        let mut section_map: BTreeMap<(i32, i32, i8), crate::palette::SectionBlocks> =
            BTreeMap::new();
        for c in &changes {
            let cx = c.x >> 4;
            let cz = c.z >> 4;
            if let std::collections::btree_map::Entry::Vacant(e) = dirty_chunks.entry((cx, cz)) {
                e.insert(self.load_chunk(cx, cz)?);
            }
            let (_, _, _, sy) = crate::chunk::local_in_chunk(c.x, c.y, c.z);
            let key = (cx, cz, sy);
            if let std::collections::btree_map::Entry::Vacant(e) = section_map.entry(key) {
                e.insert(dirty_chunks.get(&(cx, cz)).unwrap().read_section_blocks(sy)?);
            }
        }
        for c in &changes {
            let cx = c.x >> 4;
            let cz = c.z >> 4;
            let (lx, ly, lz, sy) = crate::chunk::local_in_chunk(c.x, c.y, c.z);
            section_map
                .get_mut(&(cx, cz, sy))
                .unwrap()
                .set(lx, ly, lz, c.after.clone());
        }
        for ((cx, cz, sy), section) in section_map {
            dirty_chunks
                .get_mut(&(cx, cz))
                .unwrap()
                .write_section_blocks(sy, &section)?;
        }
        for chunk in dirty_chunks.values() {
            self.save_chunk(chunk)?;
        }
        let action = Action {
            id: 0,
            description: description.into(),
            payload: ActionPayload::SetBlocks { changes },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    fn plan_set_positions(
        &mut self,
        positions: &[(i32, i32, i32)],
        block: &BlockState,
    ) -> Result<Vec<BlockChange>> {
        self.plan_set_pattern_positions(positions, &Pattern::single(block.clone()), None)
    }

    fn plan_set_pattern_positions(
        &mut self,
        positions: &[(i32, i32, i32)],
        pattern: &Pattern,
        mask: Option<&Mask>,
    ) -> Result<Vec<BlockChange>> {
        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
        let mut changes = Vec::new();
        for &(x, y, z) in positions {
            let cx = x >> 4;
            let cz = z >> 4;
            if let std::collections::btree_map::Entry::Vacant(e) = dirty_chunks.entry((cx, cz)) {
                e.insert(self.load_chunk(cx, cz)?);
            }
            let before = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
            if let Some(m) = mask {
                if !m.matches(&before) {
                    continue;
                }
            }
            let after = pattern.pick_at(x, y, z).clone();
            if before != after {
                changes.push(BlockChange {
                    x,
                    y,
                    z,
                    before,
                    after,
                });
            }
        }
        Ok(changes)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        block: BlockState,
    ) -> Result<Action> {
        self.fill_pattern(
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
            Pattern::single(block),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_pattern(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        pattern: Pattern,
        mask: Option<Mask>,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let positions = crate::ops::aabb_positions(box_, |_, _, _| true);
        let changes =
            self.plan_set_pattern_positions(&positions, &pattern, mask.as_ref())?;
        let desc = match &mask {
            Some(m) => format!(
                "fill ({},{},{})..({},{},{}) -> {} mask={}",
                box_.min_x,
                box_.min_y,
                box_.min_z,
                box_.max_x,
                box_.max_y,
                box_.max_z,
                pattern.describe(),
                m.describe()
            ),
            None => format!(
                "fill ({},{},{})..({},{},{}) -> {}",
                box_.min_x,
                box_.min_y,
                box_.min_z,
                box_.max_x,
                box_.max_y,
                box_.max_z,
                pattern.describe()
            ),
        };
        self.apply_changes(changes, desc)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        match_block: BlockState,
        with: BlockState,
    ) -> Result<Action> {
        self.replace_mask_pattern(
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
            Mask::parse(&match_block.to_compact()).unwrap_or(Mask::Exact(match_block)),
            Pattern::single(with),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_mask_pattern(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        mask: Mask,
        pattern: Pattern,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let positions = crate::ops::aabb_positions(box_, |_, _, _| true);
        let changes = self.plan_set_pattern_positions(&positions, &pattern, Some(&mask))?;
        self.apply_changes(
            changes,
            format!(
                "replace {} -> {} in ({},{},{})..({},{},{})",
                mask.describe(),
                pattern.describe(),
                box_.min_x,
                box_.min_y,
                box_.min_z,
                box_.max_x,
                box_.max_y,
                box_.max_z
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn walls(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        block: BlockState,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let positions = crate::ops::aabb_positions(box_, |x, y, z| box_.on_wall(x, y, z));
        let changes = self.plan_set_positions(&positions, &block)?;
        self.apply_changes(changes, format!("walls -> {block}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn outline(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        block: BlockState,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let positions = crate::ops::aabb_positions(box_, |x, y, z| box_.on_outline(x, y, z));
        let changes = self.plan_set_positions(&positions, &block)?;
        self.apply_changes(changes, format!("outline -> {block}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn hollow(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let air = BlockState::air();
        let positions = crate::ops::aabb_positions(box_, |x, y, z| box_.interior(x, y, z));
        let changes = self.plan_set_positions(&positions, &air)?;
        self.apply_changes(changes, "hollow (interior -> air)")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn overlay(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        block: BlockState,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
        let mut positions = Vec::new();
        for z in box_.min_z..=box_.max_z {
            for x in box_.min_x..=box_.max_x {
                let mut top: Option<i32> = None;
                for y in box_.min_y..=box_.max_y {
                    let cx = x >> 4;
                    let cz = z >> 4;
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        dirty_chunks.entry((cx, cz))
                    {
                        e.insert(self.load_chunk(cx, cz)?);
                    }
                    let b = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                    if !b.is_air_like() {
                        top = Some(y);
                    }
                }
                if let Some(y) = top {
                    let ny = y + 1;
                    if ny <= box_.max_y {
                        positions.push((x, ny, z));
                    } else {
                        // place just above selection top if column peaked at max_y
                        positions.push((x, ny, z));
                    }
                }
            }
        }
        let changes = self.plan_set_positions(&positions, &block)?;
        self.apply_changes(changes, format!("overlay -> {block}"))
    }

    pub fn sphere(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        radius: f64,
        block: BlockState,
        hollow: bool,
    ) -> Result<Action> {
        let positions = crate::ops::sphere_positions(cx, cy, cz, radius, hollow);
        let changes = self.plan_set_positions(&positions, &block)?;
        let kind = if hollow { "hsphere" } else { "sphere" };
        self.apply_changes(
            changes,
            format!("{kind} @{cx},{cy},{cz} r={radius} -> {block}"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn cyl(
        &mut self,
        cx: i32,
        cz: i32,
        y_base: i32,
        radius: f64,
        height: i32,
        block: BlockState,
        hollow: bool,
    ) -> Result<Action> {
        let positions = crate::ops::cyl_positions(cx, cz, y_base, radius, height, hollow);
        let changes = self.plan_set_positions(&positions, &block)?;
        let kind = if hollow { "hcyl" } else { "cyl" };
        self.apply_changes(
            changes,
            format!("{kind} @{cx},{y_base},{cz} r={radius} h={height} -> {block}"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stack(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        count: i32,
        dx: i32,
        dy: i32,
        dz: i32,
    ) -> Result<Action> {
        if count < 1 {
            return Err(Error::msg("stack count must be >= 1"));
        }
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let tpl = crate::template::Template::capture(
            self,
            "stack-src",
            box_.min_x,
            box_.min_y,
            box_.min_z,
            box_.max_x,
            box_.max_y,
            box_.max_z,
        )?;
        let mut changes = Vec::new();
        for i in 1..=count {
            let ox = box_.min_x + dx * i;
            let oy = box_.min_y + dy * i;
            let oz = box_.min_z + dz * i;
            changes.extend(tpl.plan_paste_changes(self, ox, oy, oz)?);
        }
        self.apply_changes(
            changes,
            format!("stack n={count} dir=({dx},{dy},{dz})"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn move_region(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        dx: i32,
        dy: i32,
        dz: i32,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let tpl = crate::template::Template::capture(
            self,
            "move-src",
            box_.min_x,
            box_.min_y,
            box_.min_z,
            box_.max_x,
            box_.max_y,
            box_.max_z,
        )?;
        let air = BlockState::air();
        let clear_pos = crate::ops::aabb_positions(box_, |_, _, _| true);
        let mut changes = self.plan_set_positions(&clear_pos, &air)?;
        let paste =
            tpl.plan_paste_changes(self, box_.min_x + dx, box_.min_y + dy, box_.min_z + dz)?;
        // Later writes win on overlapping cells: clear first then paste overrides.
        let mut by_pos: BTreeMap<(i32, i32, i32), BlockChange> = BTreeMap::new();
        for c in changes {
            by_pos.insert((c.x, c.y, c.z), c);
        }
        for c in paste {
            by_pos.insert((c.x, c.y, c.z), c);
        }
        changes = by_pos.into_values().collect();
        self.apply_changes(changes, format!("move by ({dx},{dy},{dz})"))
    }

    pub fn clipboard_copy(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<crate::template::Template> {
        let tpl = crate::template::Template::capture(self, "clipboard", x1, y1, z1, x2, y2, z2)?;
        tpl.save_clipboard(self.session)?;
        Ok(tpl)
    }

    pub fn clipboard_cut(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<Action> {
        let _ = self.clipboard_copy(x1, y1, z1, x2, y2, z2)?;
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let air = BlockState::air();
        let positions = crate::ops::aabb_positions(box_, |_, _, _| true);
        let changes = self.plan_set_positions(&positions, &air)?;
        self.apply_changes(changes, "cut (clipboard + clear)")
    }

    pub fn clipboard_paste(&mut self, ox: i32, oy: i32, oz: i32) -> Result<Action> {
        let tpl = crate::template::Template::load_clipboard(self.session)?;
        tpl.paste_into(self, ox, oy, oz)
    }

    pub fn clipboard_rotate_yaw(&mut self, yaw: i32) -> Result<crate::template::Template> {
        let mut tpl = crate::template::Template::load_clipboard(self.session)?;
        tpl.rotate_yaw(yaw)?;
        tpl.save_clipboard(self.session)?;
        Ok(tpl)
    }

    pub fn clipboard_flip(&mut self, axis: char) -> Result<crate::template::Template> {
        let mut tpl = crate::template::Template::load_clipboard(self.session)?;
        tpl.flip(axis)?;
        tpl.save_clipboard(self.session)?;
        Ok(tpl)
    }

    /// Sphere brush: apply pattern inside radius, optional mask.
    #[allow(clippy::too_many_arguments)]
    pub fn brush_sphere(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        radius: f64,
        pattern: Pattern,
        mask: Option<Mask>,
        hollow: bool,
    ) -> Result<Action> {
        let positions = crate::ops::sphere_positions(cx, cy, cz, radius, hollow);
        let changes =
            self.plan_set_pattern_positions(&positions, &pattern, mask.as_ref())?;
        let kind = if hollow { "brush hsphere" } else { "brush sphere" };
        self.apply_changes(
            changes,
            format!(
                "{kind} @{cx},{cy},{cz} r={radius} -> {}{}",
                pattern.describe(),
                mask.as_ref()
                    .map(|m| format!(" mask={}", m.describe()))
                    .unwrap_or_default()
            ),
        )
    }

    /// Vertical cylinder brush.
    #[allow(clippy::too_many_arguments)]
    pub fn brush_cyl(
        &mut self,
        cx: i32,
        cz: i32,
        y_base: i32,
        radius: f64,
        height: i32,
        pattern: Pattern,
        mask: Option<Mask>,
        hollow: bool,
    ) -> Result<Action> {
        let positions = crate::ops::cyl_positions(cx, cz, y_base, radius, height, hollow);
        let changes =
            self.plan_set_pattern_positions(&positions, &pattern, mask.as_ref())?;
        let kind = if hollow { "brush hcyl" } else { "brush cyl" };
        self.apply_changes(
            changes,
            format!(
                "{kind} @{cx},{y_base},{cz} r={radius} h={height} -> {}{}",
                pattern.describe(),
                mask.as_ref()
                    .map(|m| format!(" mask={}", m.describe()))
                    .unwrap_or_default()
            ),
        )
    }

    /// Clipboard brush: paste clipboard at `--at`, optionally clipped to sphere radius + mask.
    pub fn brush_clipboard(
        &mut self,
        ox: i32,
        oy: i32,
        oz: i32,
        radius: Option<f64>,
        mask: Option<Mask>,
    ) -> Result<Action> {
        let tpl = crate::template::Template::load_clipboard(self.session)?;
        let mut changes = tpl.plan_paste_changes(self, ox, oy, oz)?;
        if let Some(r) = radius {
            let r2 = r * r;
            changes.retain(|c| {
                let dx = (c.x - ox) as f64 + 0.5;
                let dy = (c.y - oy) as f64 + 0.5;
                let dz = (c.z - oz) as f64 + 0.5;
                dx * dx + dy * dy + dz * dz <= r2
            });
        }
        if let Some(ref m) = mask {
            changes.retain(|c| m.matches(&c.before));
        }
        self.apply_changes(
            changes,
            format!(
                "brush clipboard @{ox},{oy},{oz}{}",
                radius
                    .map(|r| format!(" r={r}"))
                    .unwrap_or_default()
            ),
        )
    }

    /// Heightmap smooth over AABB: average neighbor surface heights, then raise/lower columns.
    #[allow(clippy::too_many_arguments)]
    pub fn smooth(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        iterations: u32,
        kernel: i32,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let k = kernel.max(1);
        let iters = iterations.max(1);
        let mut heights: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        let mut top_block: BTreeMap<(i32, i32), BlockState> = BTreeMap::new();
        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();

        for z in box_.min_z..=box_.max_z {
            for x in box_.min_x..=box_.max_x {
                let mut top: Option<(i32, BlockState)> = None;
                for y in box_.min_y..=box_.max_y {
                    let cx = x >> 4;
                    let cz = z >> 4;
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        dirty_chunks.entry((cx, cz))
                    {
                        e.insert(self.load_chunk(cx, cz)?);
                    }
                    let b = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                    if !b.is_air_like() {
                        top = Some((y, b));
                    }
                }
                if let Some((y, b)) = top {
                    heights.insert((x, z), y);
                    top_block.insert((x, z), b);
                } else {
                    heights.insert((x, z), box_.min_y - 1);
                }
            }
        }

        let mut smoothed = heights.clone();
        for _ in 0..iters {
            let prev = smoothed.clone();
            for z in box_.min_z..=box_.max_z {
                for x in box_.min_x..=box_.max_x {
                    let mut sum = 0i64;
                    let mut n = 0i64;
                    for dz in -k..=k {
                        for dx in -k..=k {
                            let xx = x + dx;
                            let zz = z + dz;
                            if let Some(&h) = prev.get(&(xx, zz)) {
                                sum += h as i64;
                                n += 1;
                            }
                        }
                    }
                    if n > 0 {
                        let avg = ((sum as f64) / (n as f64)).round() as i32;
                        let clamped = avg.clamp(box_.min_y - 1, box_.max_y);
                        smoothed.insert((x, z), clamped);
                    }
                }
            }
        }

        let air = BlockState::air();
        let mut changes = Vec::new();
        for z in box_.min_z..=box_.max_z {
            for x in box_.min_x..=box_.max_x {
                let old_h = *heights.get(&(x, z)).unwrap_or(&(box_.min_y - 1));
                let new_h = *smoothed.get(&(x, z)).unwrap_or(&old_h);
                if new_h == old_h {
                    continue;
                }
                let fill_block = top_block
                    .get(&(x, z))
                    .cloned()
                    .unwrap_or_else(|| BlockState::parse("minecraft:stone").unwrap());
                if new_h > old_h {
                    let start = (old_h + 1).max(box_.min_y);
                    for y in start..=new_h.min(box_.max_y) {
                        let cx = x >> 4;
                        let cz = z >> 4;
                        if let std::collections::btree_map::Entry::Vacant(e) =
                            dirty_chunks.entry((cx, cz))
                        {
                            e.insert(self.load_chunk(cx, cz)?);
                        }
                        let before = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                        if before != fill_block {
                            changes.push(BlockChange {
                                x,
                                y,
                                z,
                                before,
                                after: fill_block.clone(),
                            });
                        }
                    }
                } else {
                    // Lower: clear from new_h+1 .. old_h
                    let clear_from = (new_h + 1).max(box_.min_y);
                    for y in clear_from..=old_h.min(box_.max_y) {
                        let cx = x >> 4;
                        let cz = z >> 4;
                        if let std::collections::btree_map::Entry::Vacant(e) =
                            dirty_chunks.entry((cx, cz))
                        {
                            e.insert(self.load_chunk(cx, cz)?);
                        }
                        let before = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                        if !before.is_air_like() {
                            changes.push(BlockChange {
                                x,
                                y,
                                z,
                                before,
                                after: air.clone(),
                            });
                        }
                    }
                }
            }
        }

        self.apply_changes(
            changes,
            format!(
                "smooth ({},{},{})..({},{},{}) iters={iters} kernel={k}",
                box_.min_x, box_.min_y, box_.min_z, box_.max_x, box_.max_y, box_.max_z
            ),
        )
    }

    /// Paint biomes over AABB (4×4×4 cells intersecting the box). Undoable.
    #[allow(clippy::too_many_arguments)]
    pub fn biome_paint(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        biome: &str,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let biome = normalize_biome_id(biome);
        let mut cells: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
        let min_x = box_.min_x & !3;
        let min_y = box_.min_y & !3;
        let min_z = box_.min_z & !3;
        let mut y = min_y;
        while y <= box_.max_y {
            let mut z = min_z;
            while z <= box_.max_z {
                let mut x = min_x;
                while x <= box_.max_x {
                    if x <= box_.max_x
                        && x + 3 >= box_.min_x
                        && y <= box_.max_y
                        && y + 3 >= box_.min_y
                        && z <= box_.max_z
                        && z + 3 >= box_.min_z
                    {
                        cells.insert(biome_cell_origin(x, y, z));
                    }
                    x += 4;
                }
                z += 4;
            }
            y += 4;
        }
        self.apply_biome_cells(
            cells,
            &biome,
            None,
            format!(
                "biome {biome} ({},{},{})..({},{},{})",
                box_.min_x, box_.min_y, box_.min_z, box_.max_x, box_.max_y, box_.max_z
            ),
        )
    }

    /// Biome sphere brush: world-space sphere, snapped to 4×4×4 biome cells.
    /// Optional `--mask` filters on the block at each cell origin.
    #[allow(clippy::too_many_arguments)]
    pub fn brush_biome_sphere(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        radius: f64,
        biome: &str,
        mask: Option<Mask>,
        hollow: bool,
    ) -> Result<Action> {
        let biome = normalize_biome_id(biome);
        let positions = crate::ops::sphere_positions(cx, cy, cz, radius, hollow);
        let cells: BTreeSet<(i32, i32, i32)> = positions
            .into_iter()
            .map(|(x, y, z)| biome_cell_origin(x, y, z))
            .collect();
        let kind = if hollow {
            "brush biome hsphere"
        } else {
            "brush biome sphere"
        };
        self.apply_biome_cells(
            cells,
            &biome,
            mask.as_ref(),
            format!(
                "{kind} @{cx},{cy},{cz} r={radius} -> {biome}{}",
                mask.as_ref()
                    .map(|m| format!(" mask={}", m.describe()))
                    .unwrap_or_default()
            ),
        )
    }

    /// Biome vertical cylinder brush (world-space), snapped to 4×4×4 cells.
    #[allow(clippy::too_many_arguments)]
    pub fn brush_biome_cyl(
        &mut self,
        cx: i32,
        cz: i32,
        y_base: i32,
        radius: f64,
        height: i32,
        biome: &str,
        mask: Option<Mask>,
        hollow: bool,
    ) -> Result<Action> {
        let biome = normalize_biome_id(biome);
        let positions = crate::ops::cyl_positions(cx, cz, y_base, radius, height, hollow);
        let cells: BTreeSet<(i32, i32, i32)> = positions
            .into_iter()
            .map(|(x, y, z)| biome_cell_origin(x, y, z))
            .collect();
        let kind = if hollow {
            "brush biome hcyl"
        } else {
            "brush biome cyl"
        };
        self.apply_biome_cells(
            cells,
            &biome,
            mask.as_ref(),
            format!(
                "{kind} @{cx},{y_base},{cz} r={radius} h={height} -> {biome}{}",
                mask.as_ref()
                    .map(|m| format!(" mask={}", m.describe()))
                    .unwrap_or_default()
            ),
        )
    }

    fn apply_biome_cells(
        &mut self,
        cells: BTreeSet<(i32, i32, i32)>,
        biome: &str,
        mask: Option<&Mask>,
        description: impl Into<String>,
    ) -> Result<Action> {
        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
        let mut changes = Vec::new();
        for (x, y, z) in cells {
            let cx = x >> 4;
            let cz = z >> 4;
            if let std::collections::btree_map::Entry::Vacant(e) = dirty_chunks.entry((cx, cz)) {
                e.insert(self.load_chunk(cx, cz)?);
            }
            if let Some(m) = mask {
                let block = dirty_chunks.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                if !m.matches(&block) {
                    continue;
                }
            }
            let before = dirty_chunks.get(&(cx, cz)).unwrap().get_biome(x, y, z)?;
            if before != biome {
                changes.push(BiomeChange {
                    x,
                    y,
                    z,
                    before,
                    after: biome.to_string(),
                });
            }
        }
        for c in &changes {
            let key = (c.x >> 4, c.z >> 4);
            dirty_chunks
                .get_mut(&key)
                .unwrap()
                .set_biome(c.x, c.y, c.z, &c.after)?;
        }
        for chunk in dirty_chunks.values() {
            self.save_chunk(chunk)?;
        }
        let action = Action {
            id: 0,
            description: description.into(),
            payload: ActionPayload::SetBiomes { changes },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    /// 3D voxel neighbourhood smooth over AABB (majority vote).
    ///
    /// Each iteration, every block in the AABB is replaced by the most common
    /// neighbour state in a Chebyshev ball of radius `kernel` (including self).
    /// Ties keep the current block. Suitable for softening caves / structures;
    /// for heightmap-only terrain use [`Self::smooth`].
    ///
    /// `--mode solid` (via `solid_air`): vote air vs non-air first; solid winners
    /// take the majority solid neighbour state.
    #[allow(clippy::too_many_arguments)]
    pub fn smooth3d(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        iterations: u32,
        kernel: i32,
        solid_air: bool,
    ) -> Result<Action> {
        let box_ = crate::ops::Aabb::from_corners(x1, y1, z1, x2, y2, z2);
        let k = kernel.max(1);
        let iters = iterations.max(1);
        let mut dirty: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();

        // Read expanded volume so edge neighbourhood is complete.
        let rx0 = box_.min_x - k;
        let rx1 = box_.max_x + k;
        let ry0 = box_.min_y - k;
        let ry1 = box_.max_y + k;
        let rz0 = box_.min_z - k;
        let rz1 = box_.max_z + k;

        let mut grid: BTreeMap<(i32, i32, i32), BlockState> = BTreeMap::new();
        for z in rz0..=rz1 {
            for y in ry0..=ry1 {
                for x in rx0..=rx1 {
                    let cx = x >> 4;
                    let cz = z >> 4;
                    if let std::collections::btree_map::Entry::Vacant(e) = dirty.entry((cx, cz)) {
                        e.insert(self.load_chunk(cx, cz)?);
                    }
                    let b = dirty.get(&(cx, cz)).unwrap().get_block(x, y, z)?;
                    grid.insert((x, y, z), b);
                }
            }
        }

        for _ in 0..iters {
            let prev = grid.clone();
            for z in box_.min_z..=box_.max_z {
                for y in box_.min_y..=box_.max_y {
                    for x in box_.min_x..=box_.max_x {
                        let mut counts: BTreeMap<String, (u32, BlockState)> = BTreeMap::new();
                        let mut solid_n = 0u32;
                        let mut air_n = 0u32;
                        let mut solid_counts: BTreeMap<String, (u32, BlockState)> = BTreeMap::new();
                        for dz in -k..=k {
                            for dy in -k..=k {
                                for dx in -k..=k {
                                    let xx = x + dx;
                                    let yy = y + dy;
                                    let zz = z + dz;
                                    let Some(b) = prev.get(&(xx, yy, zz)) else {
                                        continue;
                                    };
                                    let key = b.to_compact();
                                    let e = counts.entry(key.clone()).or_insert((0, b.clone()));
                                    e.0 += 1;
                                    if b.is_air_like() {
                                        air_n += 1;
                                    } else {
                                        solid_n += 1;
                                        let e = solid_counts.entry(key).or_insert((0, b.clone()));
                                        e.0 += 1;
                                    }
                                }
                            }
                        }
                        let next = if solid_air {
                            if solid_n > air_n {
                                solid_counts
                                    .into_iter()
                                    .max_by_key(|(_, (n, _))| *n)
                                    .map(|(_, (_, b))| b)
                                    .unwrap_or_else(|| prev[&(x, y, z)].clone())
                            } else if air_n > solid_n {
                                BlockState::air()
                            } else {
                                prev[&(x, y, z)].clone()
                            }
                        } else {
                            counts
                                .into_iter()
                                .max_by_key(|(_, (n, _))| *n)
                                .map(|(_, (_, b))| b)
                                .unwrap_or_else(|| prev[&(x, y, z)].clone())
                        };
                        grid.insert((x, y, z), next);
                    }
                }
            }
        }

        let mut changes = Vec::new();
        for z in box_.min_z..=box_.max_z {
            for y in box_.min_y..=box_.max_y {
                for x in box_.min_x..=box_.max_x {
                    let after = grid[&(x, y, z)].clone();
                    let before = dirty
                        .get(&(x >> 4, z >> 4))
                        .unwrap()
                        .get_block(x, y, z)?;
                    if before != after {
                        changes.push(BlockChange {
                            x,
                            y,
                            z,
                            before,
                            after,
                        });
                    }
                }
            }
        }

        let mode = if solid_air { "solid" } else { "majority" };
        self.apply_changes(
            changes,
            format!(
                "smooth3d ({},{},{})..({},{},{}) iters={iters} kernel={k} mode={mode}",
                box_.min_x, box_.min_y, box_.min_z, box_.max_x, box_.max_y, box_.max_z
            ),
        )
    }

    pub fn get_biome(&self, x: i32, y: i32, z: i32) -> Result<String> {
        self.load_chunk(x >> 4, z >> 4)?.get_biome(x, y, z)
    }

    pub fn gen_terrain(
        &mut self,
        seed: u64,
        dim: &str,
        from_cx: i32,
        from_cz: i32,
        to_cx: i32,
        to_cz: i32,
    ) -> Result<Vec<String>> {
        crate::terrain_bridge::generate_chunks(self, seed, dim, from_cx, from_cz, to_cx, to_cz)
    }

    pub fn fix_light(
        &mut self,
        from_cx: i32,
        from_cz: i32,
        to_cx: i32,
        to_cz: i32,
        seed: u64,
        dim: &str,
    ) -> Result<Vec<String>> {
        crate::terrain_bridge::fix_light(self, from_cx, from_cz, to_cx, to_cz, seed, dim)
    }

    pub fn tick_participate(
        &mut self,
        from_cx: i32,
        from_cz: i32,
        to_cx: i32,
        to_cz: i32,
        rounds: u32,
        random_tick_speed: u32,
    ) -> Result<Vec<String>> {
        self.tick_offline(from_cx, from_cz, to_cx, to_cz, rounds, random_tick_speed)
            .map(|(lines, _)| lines)
    }

    /// First-class offline tick: step scheduled tick NBT queues and apply
    /// approximate random-tick growth (crops / cane / grass). Growth mutations
    /// are undoable as one history entry.
    ///
    /// Does **not** round-trip chunks through the terrain bridge (that path can
    /// drop blocks). Use `edit gen` / `fix-light` for bridge features.
    pub fn tick_offline(
        &mut self,
        from_cx: i32,
        from_cz: i32,
        to_cx: i32,
        to_cz: i32,
        rounds: u32,
        random_tick_speed: u32,
    ) -> Result<(Vec<String>, Option<Action>)> {
        let (planned, stats) = crate::tick::plan_offline_random_ticks(
            self,
            from_cx,
            from_cz,
            to_cx,
            to_cz,
            rounds,
            random_tick_speed,
        )?;
        let mut lines = vec![format!(
            "tick chunks={} rounds={} speed={} samples={} candidates={} due_block={} due_fluid={}",
            stats.chunks,
            rounds.max(1),
            random_tick_speed.max(1),
            stats.samples,
            stats.candidates,
            stats.due_block,
            stats.due_fluid
        )];
        let action = if planned.is_empty() {
            None
        } else {
            Some(self.apply_changes(
                planned,
                format!(
                    "tick growth chunks~{} samples={} candidates={} mutations={}",
                    stats.chunks, stats.samples, stats.candidates, stats.mutations
                ),
            )?)
        };
        lines.push(format!(
            "tick_growth mutations={} undoable={}",
            stats.mutations,
            action.is_some()
        ));
        Ok((lines, action))
    }

    pub fn set_section(
        &mut self,
        cx: i32,
        cy: i32,
        cz: i32,
        after: SectionDiff,
    ) -> Result<Action> {
        let mut chunk = self.load_chunk(cx, cz)?;
        let section_y = cy as i8;
        let section = chunk.read_section_blocks(section_y)?;
        let before = after.capture_before(&section);
        chunk.apply_section_diff(section_y, &after)?;
        self.save_chunk(&chunk)?;
        let action = Action {
            id: 0,
            description: format!("set-section {cx},{cy},{cz}"),
            payload: ActionPayload::SetSection {
                cx,
                cy,
                cz,
                before,
                after,
            },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    pub fn entity_store(&self) -> EntityStore {
        EntityStore::new(self.session.work_entities_dir())
    }

    /// Insert entity into work copy without pushing history.
    pub fn insert_entity_raw(
        &mut self,
        mut nbt: JsonValue,
    ) -> Result<(i32, i32, JsonValue)> {
        if nbt.get("id").and_then(|v| v.as_str()).is_none() {
            return Err(Error::msg("entity NBT requires id"));
        }
        let rec = EntityRecord::from_nbt_json(nbt.clone())?;
        if nbt.get("UUID").is_none() {
            let u = Uuid::parse_str(&rec.uuid).unwrap_or_else(|_| Uuid::new_v4());
            let (most, least) = u.as_u64_pair();
            let arr = vec![
                JsonValue::from((most >> 32) as i32),
                JsonValue::from(most as i32),
                JsonValue::from((least >> 32) as i32),
                JsonValue::from(least as i32),
            ];
            nbt.as_object_mut()
                .unwrap()
                .insert("UUID".into(), JsonValue::Array(arr));
        }
        let cx = rec.x.floor() as i32 >> 4;
        let cz = rec.z.floor() as i32 >> 4;
        let _ = self.ensure_entity_region(cx, cz)?;
        let store = self.entity_store();
        let mut chunk = store.read_chunk(cx, cz)?;
        chunk.entities.push(nbt.clone());
        store.write_chunk(&chunk)?;
        Ok((cx, cz, nbt))
    }

    pub fn entity_spawn(&mut self, nbt: JsonValue) -> Result<Action> {
        let rec = EntityRecord::from_nbt_json(nbt.clone())?;
        let (cx, cz, nbt) = self.insert_entity_raw(nbt)?;
        let action = Action {
            id: 0,
            description: format!("entity spawn {}", rec.id),
            payload: ActionPayload::EntitySpawn {
                chunk_x: cx,
                chunk_z: cz,
                entity: nbt,
            },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    pub fn entity_remove(&mut self, uuid: &str, hint_x: i32, hint_z: i32) -> Result<Action> {
        let cx = hint_x >> 4;
        let cz = hint_z >> 4;
        let _ = self.ensure_entity_region(cx, cz)?;
        let store = self.entity_store();
        let Some((ecx, ecz, idx, ent)) = store.find_by_uuid(uuid, cx, cz)? else {
            return Err(Error::msg(format!("entity {uuid} not found")));
        };
        let mut chunk = store.read_chunk(ecx, ecz)?;
        chunk.entities.remove(idx);
        store.write_chunk(&chunk)?;
        let action = Action {
            id: 0,
            description: format!("entity rm {uuid}"),
            payload: ActionPayload::EntityRemove {
                chunk_x: ecx,
                chunk_z: ecz,
                entity: ent,
            },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    pub fn entity_set(
        &mut self,
        uuid: &str,
        hint_x: i32,
        hint_z: i32,
        after: JsonValue,
    ) -> Result<Action> {
        let cx = hint_x >> 4;
        let cz = hint_z >> 4;
        let _ = self.ensure_entity_region(cx, cz)?;
        let store = self.entity_store();
        let Some((ecx, ecz, idx, before)) = store.find_by_uuid(uuid, cx, cz)? else {
            return Err(Error::msg(format!("entity {uuid} not found")));
        };
        let mut chunk = store.read_chunk(ecx, ecz)?;
        chunk.entities[idx] = after.clone();
        store.write_chunk(&chunk)?;
        let action = Action {
            id: 0,
            description: format!("entity set {uuid}"),
            payload: ActionPayload::EntitySet {
                chunk_x: ecx,
                chunk_z: ecz,
                uuid: uuid.to_string(),
                before,
                after,
            },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
    }

    pub fn apply_action_forward(&mut self, action: &Action) -> Result<()> {
        match &action.payload {
            ActionPayload::SetBlocks { changes } => {
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_block(c.x, c.y, c.z, c.after.clone())?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
            }
            ActionPayload::SetBiomes { changes } => {
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_biome(c.x, c.y, c.z, &c.after)?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
            }
            ActionPayload::SetSection {
                cx, cy, cz, after, ..
            } => {
                let mut chunk = self.load_chunk(*cx, *cz)?;
                chunk.apply_section_diff(*cy as i8, after)?;
                self.save_chunk(&chunk)?;
            }
            ActionPayload::EntitySpawn {
                chunk_x,
                chunk_z,
                entity,
            } => {
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                chunk.entities.push(entity.clone());
                store.write_chunk(&chunk)?;
            }
            ActionPayload::EntitySet {
                chunk_x,
                chunk_z,
                uuid,
                after,
                ..
            } => {
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                if let Some(ent) = chunk.entities.iter_mut().find(|e| {
                    EntityRecord::from_nbt_json((*e).clone())
                        .map(|r| r.uuid == *uuid)
                        .unwrap_or(false)
                }) {
                    *ent = after.clone();
                }
                store.write_chunk(&chunk)?;
            }
            ActionPayload::EntityRemove {
                chunk_x,
                chunk_z,
                entity,
            } => {
                let rec = EntityRecord::from_nbt_json(entity.clone())?;
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                chunk.entities.retain(|e| {
                    EntityRecord::from_nbt_json(e.clone())
                        .map(|r| r.uuid != rec.uuid)
                        .unwrap_or(true)
                });
                store.write_chunk(&chunk)?;
            }
            ActionPayload::PasteTemplate {
                changes, spawned, ..
            } => {
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_block(c.x, c.y, c.z, c.after.clone())?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
                for s in spawned {
                    let _ = self.ensure_entity_region(s.chunk_x, s.chunk_z)?;
                    let store = self.entity_store();
                    let mut chunk = store.read_chunk(s.chunk_x, s.chunk_z)?;
                    chunk.entities.push(s.entity.clone());
                    store.write_chunk(&chunk)?;
                }
            }
        }
        Ok(())
    }

    pub fn apply_action_backward(&mut self, action: &Action) -> Result<()> {
        match &action.payload {
            ActionPayload::SetBlocks { changes } => {
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_block(c.x, c.y, c.z, c.before.clone())?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
            }
            ActionPayload::SetBiomes { changes } => {
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_biome(c.x, c.y, c.z, &c.before)?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
            }
            ActionPayload::SetSection {
                cx, cy, cz, before, ..
            } => {
                let mut chunk = self.load_chunk(*cx, *cz)?;
                chunk.apply_section_diff(*cy as i8, before)?;
                self.save_chunk(&chunk)?;
            }
            ActionPayload::EntitySpawn {
                chunk_x,
                chunk_z,
                entity,
            } => {
                let rec = EntityRecord::from_nbt_json(entity.clone())?;
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                chunk.entities.retain(|e| {
                    EntityRecord::from_nbt_json(e.clone())
                        .map(|r| r.uuid != rec.uuid)
                        .unwrap_or(true)
                });
                store.write_chunk(&chunk)?;
            }
            ActionPayload::EntitySet {
                chunk_x,
                chunk_z,
                uuid,
                before,
                ..
            } => {
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                if let Some(ent) = chunk.entities.iter_mut().find(|e| {
                    EntityRecord::from_nbt_json((*e).clone())
                        .map(|r| r.uuid == *uuid)
                        .unwrap_or(false)
                }) {
                    *ent = before.clone();
                }
                store.write_chunk(&chunk)?;
            }
            ActionPayload::EntityRemove {
                chunk_x,
                chunk_z,
                entity,
            } => {
                let _ = self.ensure_entity_region(*chunk_x, *chunk_z)?;
                let store = self.entity_store();
                let mut chunk = store.read_chunk(*chunk_x, *chunk_z)?;
                chunk.entities.push(entity.clone());
                store.write_chunk(&chunk)?;
            }
            ActionPayload::PasteTemplate {
                changes, spawned, ..
            } => {
                // Remove spawned entities first, then restore blocks.
                for s in spawned {
                    let rec = EntityRecord::from_nbt_json(s.entity.clone())?;
                    let _ = self.ensure_entity_region(s.chunk_x, s.chunk_z)?;
                    let store = self.entity_store();
                    let mut chunk = store.read_chunk(s.chunk_x, s.chunk_z)?;
                    chunk.entities.retain(|e| {
                        EntityRecord::from_nbt_json(e.clone())
                            .map(|r| r.uuid != rec.uuid)
                            .unwrap_or(true)
                    });
                    store.write_chunk(&chunk)?;
                }
                let mut chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
                for c in changes {
                    let key = (c.x >> 4, c.z >> 4);
                    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                        e.insert(self.load_chunk(key.0, key.1)?);
                    }
                    chunks
                        .get_mut(&key)
                        .unwrap()
                        .set_block(c.x, c.y, c.z, c.before.clone())?;
                }
                for ch in chunks.values() {
                    self.save_chunk(ch)?;
                }
            }
        }
        Ok(())
    }

    pub fn undo(&mut self, n: usize) -> Result<Vec<Action>> {
        let mut done = Vec::new();
        for _ in 0..n {
            let action = self.session.history.undo_target()?;
            self.apply_action_backward(&action)?;
            self.session.history.mark_undone()?;
            done.push(action);
        }
        self.session.mark_dirty()?;
        Ok(done)
    }

    pub fn redo(&mut self, n: usize) -> Result<Vec<Action>> {
        let mut done = Vec::new();
        for _ in 0..n {
            let action = self.session.history.redo_target()?;
            self.apply_action_forward(&action)?;
            self.session.history.mark_redone()?;
            done.push(action);
        }
        self.session.mark_dirty()?;
        Ok(done)
    }

    pub fn revert_to(&mut self, index: usize) -> Result<Vec<Action>> {
        let mut undone = Vec::new();
        while self.session.history.position() > index {
            let batch = self.undo(1)?;
            undone.extend(batch);
        }
        while self.session.history.position() < index {
            let batch = self.redo(1)?;
            undone.extend(batch);
        }
        Ok(undone)
    }

    pub fn summary_chunk(&self, cx: i32, cz: i32) -> Result<String> {
        let chunk = self.load_chunk(cx, cz)?;
        let (total, counts) = chunk.count_non_air()?;
        let mut top: Vec<_> = counts.into_iter().collect();
        top.sort_by_key(|a| std::cmp::Reverse(a.1));
        top.truncate(8);
        let top_s: Vec<String> = top
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        Ok(format!(
            "chunk={cx},{cz} non_air={total} top=[{}]",
            top_s.join(" ")
        ))
    }

    pub fn slice(
        &self,
        y: i32,
        x1: i32,
        z1: i32,
        x2: i32,
        z2: i32,
    ) -> Result<Vec<String>> {
        let (min_x, max_x) = (x1.min(x2), x1.max(x2));
        let (min_z, max_z) = (z1.min(z2), z1.max(z2));
        if max_x - min_x > 64 || max_z - min_z > 64 {
            return Err(Error::msg("slice too large (max 65x65)"));
        }
        let mut lines = Vec::new();
        lines.push(format!("slice y={y} ({min_x},{min_z})..({max_x},{max_z})"));
        for z in min_z..=max_z {
            let mut row = Vec::new();
            for x in min_x..=max_x {
                let b = self.get_block(x, y, z)?;
                row.push(b.short_label());
            }
            lines.push(row.join(" "));
        }
        Ok(lines)
    }

    /// AABB select with palette rebuild:
    /// 1) block id table for the box
    /// 2) ASCII 3D as Y layers of those ids
    pub fn select_box(
        &self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<Vec<String>> {
        let (min_x, max_x) = (x1.min(x2), x1.max(x2));
        let (min_y, max_y) = (y1.min(y2), y1.max(y2));
        let (min_z, max_z) = (z1.min(z2), z1.max(z2));
        let sx = (max_x - min_x + 1) as usize;
        let sy = (max_y - min_y + 1) as usize;
        let sz = (max_z - min_z + 1) as usize;
        if sx * sy * sz > 32 * 32 * 32 {
            return Err(Error::msg("select box too large (max 32^3 cells)"));
        }
        if sx > 48 || sz > 48 {
            return Err(Error::msg("select box xz too wide (max 48)"));
        }

        // Pass 1: rebuild local palette (air forced to id 0 when present).
        let mut palette: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        palette.insert(BlockState::air().to_compact());
        let mut cells: Vec<u16> = Vec::with_capacity(sx * sy * sz);
        let mut non_air = 0usize;

        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = self.get_block(x, y, z)?;
                    let key = if b.is_air_like() {
                        BlockState::air().to_compact()
                    } else {
                        non_air += 1;
                        b.to_compact()
                    };
                    let (id, _) = palette.insert_full(key);
                    cells.push(id as u16);
                }
            }
        }

        // If volume is all non-air, drop unused air id 0 and remap.
        let air_used = cells.contains(&0);
        if !air_used && palette.len() > 1 {
            palette.shift_remove_index(0);
            for id in &mut cells {
                *id -= 1;
            }
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "box ({min_x},{min_y},{min_z})..({max_x},{max_y},{max_z}) size={sx}x{sy}x{sz} non_air={non_air}"
        ));
        lines.push(format!("palette n={}", palette.len()));
        for (id, name) in palette.iter().enumerate() {
            lines.push(format!("{id} {name}"));
        }

        // Pass 2: ASCII 3D — Y layers top→bottom, each row is +Z, cells along +X.
        lines.push(format!(
            "ascii x→ +X  z↓ +Z  y layers {max_y}→{min_y} (ids space-separated)"
        ));
        let id_width = {
            let max_id = palette.len().saturating_sub(1);
            if max_id < 10 {
                1
            } else if max_id < 100 {
                2
            } else {
                3
            }
        };

        for y in (min_y..=max_y).rev() {
            lines.push(format!("y={y}"));
            let y_off = ((y - min_y) as usize) * sz * sx;
            for zi in 0..sz {
                let mut row = Vec::with_capacity(sx);
                for xi in 0..sx {
                    let id = cells[y_off + zi * sx + xi];
                    row.push(format!("{id:>width$}", width = id_width));
                }
                lines.push(row.join(" "));
            }
        }
        Ok(lines)
    }
}

fn normalize_biome_id(biome: &str) -> String {
    if biome.contains(':') {
        biome.to_string()
    } else {
        format!("minecraft:{biome}")
    }
}
