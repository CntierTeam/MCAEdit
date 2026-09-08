use crate::action::{Action, ActionPayload, BlockChange};
use crate::blockstate::BlockState;
use crate::chunk::ChunkData;
use crate::entity::{EntityRecord, EntityStore};
use crate::error::{Error, Result};
use crate::palette::SectionDiff;
use crate::region::{copy_region_if_needed, region_coords, RegionStore};
use crate::session::Session;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
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
            None => Ok(ChunkData::empty(chunk_x, chunk_z)),
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
        let (min_x, max_x) = (x1.min(x2), x1.max(x2));
        let (min_y, max_y) = (y1.min(y2), y1.max(y2));
        let (min_z, max_z) = (z1.min(z2), z1.max(z2));
        let mut changes = Vec::new();
        let mut dirty_chunks: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();

        for x in min_x..=max_x {
            for y in min_y..=max_y {
                for z in min_z..=max_z {
                    let cx = x >> 4;
                    let cz = z >> 4;
                    if let std::collections::btree_map::Entry::Vacant(e) = dirty_chunks.entry((cx, cz)) {
                        e.insert(self.load_chunk(cx, cz)?);
                    }
                    let chunk = dirty_chunks.get_mut(&(cx, cz)).unwrap();
                    let (lx, ly, lz, sy) = crate::chunk::local_in_chunk(x, y, z);
                    // mutate section once per unique section key via cache on chunk root later;
                    // for MVP still per-cell but avoid full JSON pack each time by batching sections.
                    let _ = (lx, ly, lz, sy);
                    let before = chunk.get_block(x, y, z)?;
                    if before != block {
                        changes.push(BlockChange {
                            x,
                            y,
                            z,
                            before,
                            after: block.clone(),
                        });
                    }
                }
            }
        }

        // apply changes grouped by section
        let mut section_map: BTreeMap<(i32, i32, i8), crate::palette::SectionBlocks> =
            BTreeMap::new();
        for c in &changes {
            let cx = c.x >> 4;
            let cz = c.z >> 4;
            let (_, _, _, sy) = crate::chunk::local_in_chunk(c.x, c.y, c.z);
            let key = (cx, cz, sy);
            if let std::collections::btree_map::Entry::Vacant(e) = section_map.entry(key) {
                let chunk = dirty_chunks.get(&(cx, cz)).unwrap();
                e.insert(chunk.read_section_blocks(sy)?);
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
            let chunk = dirty_chunks.get_mut(&(cx, cz)).unwrap();
            chunk.write_section_blocks(sy, &section)?;
        }
        for chunk in dirty_chunks.values() {
            self.save_chunk(chunk)?;
        }
        let action = Action {
            id: 0,
            description: format!(
                "fill ({min_x},{min_y},{min_z})..({max_x},{max_y},{max_z}) -> {block}"
            ),
            payload: ActionPayload::SetBlocks { changes },
        };
        let action = self.session.history.push(action)?;
        self.session.mark_dirty()?;
        Ok(action)
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
