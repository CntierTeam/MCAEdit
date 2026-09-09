use crate::action::{Action, ActionPayload, BlockChange, SpawnedEntity};
use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::world::WorldView;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const TEMPLATE_DIR_NAME: &str = "templates";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    /// Inclusive size: dx, dy, dz (number of blocks on each axis)
    pub size: [u32; 3],
    pub palette: Vec<String>,
    /// Flat indices in order y asc, then z asc, then x asc. Length = dx*dy*dz.
    pub blocks: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<TemplateEntity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateEntity {
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
    pub nbt: serde_json::Value,
}

impl Template {
    pub fn templates_root(cwd: &Path) -> PathBuf {
        cwd.join(crate::session::SESSION_ROOT_NAME)
            .join(TEMPLATE_DIR_NAME)
    }

    pub fn path(cwd: &Path, name: &str) -> PathBuf {
        Self::templates_root(cwd).join(format!("{name}.json"))
    }

    pub fn list(cwd: &Path) -> Result<Vec<String>> {
        let root = Self::templates_root(cwd);
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(stem) = name.strip_suffix(".json") {
                names.push(stem.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn load(cwd: &Path, name: &str) -> Result<Self> {
        let path = Self::path(cwd, name);
        if !path.exists() {
            return Err(Error::msg(format!("template `{name}` not found")));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn save_to_disk(&self, cwd: &Path) -> Result<PathBuf> {
        let root = Self::templates_root(cwd);
        fs::create_dir_all(&root)?;
        let path = Self::path(cwd, &self.name);
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }

    pub fn delete(cwd: &Path, name: &str) -> Result<()> {
        let path = Self::path(cwd, name);
        if !path.exists() {
            return Err(Error::msg(format!("template `{name}` not found")));
        }
        fs::remove_file(path)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        world: &WorldView<'_>,
        name: &str,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<Self> {
        let (min_x, max_x) = (x1.min(x2), x1.max(x2));
        let (min_y, max_y) = (y1.min(y2), y1.max(y2));
        let (min_z, max_z) = (z1.min(z2), z1.max(z2));
        let dx = (max_x - min_x + 1) as u32;
        let dy = (max_y - min_y + 1) as u32;
        let dz = (max_z - min_z + 1) as u32;
        if dx as usize * dy as usize * dz as usize > 64 * 64 * 64 {
            return Err(Error::msg("template too large (max 64^3)"));
        }

        let mut palette: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        palette.insert(BlockState::air().to_compact());
        let mut blocks = Vec::with_capacity((dx * dy * dz) as usize);

        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = world.get_block(x, y, z)?;
                    let key = if b.is_air_like() {
                        BlockState::air().to_compact()
                    } else {
                        b.to_compact()
                    };
                    let (id, _) = palette.insert_full(key);
                    blocks.push(id as u16);
                }
            }
        }

        let mut entities = Vec::new();
        let store = world.entity_store();
        let cx0 = min_x >> 4;
        let cx1 = max_x >> 4;
        let cz0 = min_z >> 4;
        let cz1 = max_z >> 4;
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let (rx, rz) = crate::region::region_coords(cx, cz);
                let _ = crate::region::copy_region_if_needed(
                    &world.session.source_entities_dir(),
                    &world.session.work_entities_dir(),
                    rx,
                    rz,
                )?;
                let chunk = store.read_chunk(cx, cz)?;
                for ent in chunk.entities {
                    let rec = crate::entity::EntityRecord::from_nbt_json(ent.clone())?;
                    let ix = rec.x.floor() as i32;
                    let iy = rec.y.floor() as i32;
                    let iz = rec.z.floor() as i32;
                    if (min_x..=max_x).contains(&ix)
                        && (min_y..=max_y).contains(&iy)
                        && (min_z..=max_z).contains(&iz)
                    {
                        entities.push(TemplateEntity {
                            dx: rec.x - min_x as f64,
                            dy: rec.y - min_y as f64,
                            dz: rec.z - min_z as f64,
                            nbt: ent,
                        });
                    }
                }
            }
        }

        Ok(Self {
            name: name.to_string(),
            size: [dx, dy, dz],
            palette: palette.into_iter().collect(),
            blocks,
            entities,
        })
    }

    pub fn paste_into(
        &self,
        world: &mut WorldView<'_>,
        origin_x: i32,
        origin_y: i32,
        origin_z: i32,
    ) -> Result<Action> {
        let changes = self.plan_paste_changes(world, origin_x, origin_y, origin_z)?;
        let mut dirty_chunks: BTreeMap<(i32, i32), crate::chunk::ChunkData> = BTreeMap::new();
        let mut section_map: BTreeMap<(i32, i32, i8), crate::palette::SectionBlocks> =
            BTreeMap::new();
        for c in &changes {
            let cx = c.x >> 4;
            let cz = c.z >> 4;
            if let std::collections::btree_map::Entry::Vacant(e) = dirty_chunks.entry((cx, cz)) {
                e.insert(world.load_chunk(cx, cz)?);
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
        for ((cx, cz, sy), section) in &section_map {
            dirty_chunks
                .get_mut(&(*cx, *cz))
                .unwrap()
                .write_section_blocks(*sy, section)?;
        }
        for chunk in dirty_chunks.values() {
            world.save_chunk(chunk)?;
        }

        let mut spawned = Vec::new();
        for ent in &self.entities {
            let mut nbt = ent.nbt.clone();
            if let Some(obj) = nbt.as_object_mut() {
                obj.insert(
                    "Pos".into(),
                    serde_json::json!([
                        origin_x as f64 + ent.dx,
                        origin_y as f64 + ent.dy,
                        origin_z as f64 + ent.dz
                    ]),
                );
                // New UUID so paste does not collide with source
                let u = uuid::Uuid::new_v4();
                let (most, least) = u.as_u64_pair();
                obj.insert(
                    "UUID".into(),
                    serde_json::json!([
                        (most >> 32) as i32,
                        most as i32,
                        (least >> 32) as i32,
                        least as i32
                    ]),
                );
            }
            let (cx, cz, nbt) = world.insert_entity_raw(nbt)?;
            spawned.push(SpawnedEntity {
                chunk_x: cx,
                chunk_z: cz,
                entity: nbt,
            });
        }

        let action = Action {
            id: 0,
            description: format!(
                "template paste {} @ {},{},{}",
                self.name, origin_x, origin_y, origin_z
            ),
            payload: ActionPayload::PasteTemplate {
                name: self.name.clone(),
                origin: [origin_x, origin_y, origin_z],
                changes,
                spawned,
            },
        };
        let action = world.session.history.push(action)?;
        world.session.mark_dirty()?;
        Ok(action)
    }

    /// Plan block changes for paste without writing (used by stack/move).
    pub fn plan_paste_changes(
        &self,
        world: &WorldView<'_>,
        origin_x: i32,
        origin_y: i32,
        origin_z: i32,
    ) -> Result<Vec<BlockChange>> {
        let [dx, dy, dz] = self.size;
        let mut changes = Vec::new();
        let mut i = 0usize;
        for y in 0..dy as i32 {
            for z in 0..dz as i32 {
                for x in 0..dx as i32 {
                    let id = *self
                        .blocks
                        .get(i)
                        .ok_or_else(|| Error::msg("template blocks truncated"))?
                        as usize;
                    i += 1;
                    let name = self
                        .palette
                        .get(id)
                        .ok_or_else(|| Error::msg(format!("bad palette id {id}")))?;
                    let after = BlockState::parse(name).map_err(Error::msg)?;
                    let wx = origin_x + x;
                    let wy = origin_y + y;
                    let wz = origin_z + z;
                    let before = world.get_block(wx, wy, wz)?;
                    if before != after {
                        changes.push(BlockChange {
                            x: wx,
                            y: wy,
                            z: wz,
                            before,
                            after,
                        });
                    }
                }
            }
        }
        Ok(changes)
    }

    pub fn clipboard_path(session: &crate::session::Session) -> PathBuf {
        session.root.join("clipboard.json")
    }

    pub fn save_clipboard(&self, session: &crate::session::Session) -> Result<PathBuf> {
        let path = Self::clipboard_path(session);
        let mut clone = self.clone();
        clone.name = "clipboard".into();
        fs::write(&path, serde_json::to_string_pretty(&clone)?)?;
        Ok(path)
    }

    pub fn load_clipboard(session: &crate::session::Session) -> Result<Self> {
        let path = Self::clipboard_path(session);
        if !path.exists() {
            return Err(Error::msg("clipboard empty (edit copy first)"));
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    fn index_of(dx: u32, dz: u32, x: u32, y: u32, z: u32) -> usize {
        ((y * dz + z) * dx + x) as usize
    }

    /// Rotate clipboard around +Y by yaw degrees (90 / 180 / 270 clockwise looking down).
    pub fn rotate_yaw(&mut self, yaw: i32) -> Result<()> {
        let yaw = yaw.rem_euclid(360);
        if yaw == 0 {
            return Ok(());
        }
        if !matches!(yaw, 90 | 180 | 270) {
            return Err(Error::msg("rotate yaw must be 90, 180, or 270"));
        }
        let [dx, dy, dz] = self.size;
        let (ndx, ndz) = if yaw == 180 { (dx, dz) } else { (dz, dx) };
        let mut new_blocks = vec![0u16; (ndx * dy * ndz) as usize];
        for y in 0..dy {
            for z in 0..dz {
                for x in 0..dx {
                    let id = self.blocks[Self::index_of(dx, dz, x, y, z)];
                    let (nx, nz) = match yaw {
                        90 => (z, dx - 1 - x),
                        180 => (dx - 1 - x, dz - 1 - z),
                        270 => (dz - 1 - z, x),
                        _ => unreachable!(),
                    };
                    new_blocks[Self::index_of(ndx, ndz, nx, y, nz)] = id;
                }
            }
        }
        self.blocks = new_blocks;
        self.size = [ndx, dy, ndz];
        for ent in &mut self.entities {
            let (nx, nz) = match yaw {
                90 => (ent.dz, (dx as f64) - 1.0 - ent.dx),
                180 => ((dx as f64) - 1.0 - ent.dx, (dz as f64) - 1.0 - ent.dz),
                270 => ((dz as f64) - 1.0 - ent.dz, ent.dx),
                _ => unreachable!(),
            };
            ent.dx = nx;
            ent.dz = nz;
        }
        Ok(())
    }

    /// Flip clipboard on axis `x` / `y` / `z`.
    pub fn flip(&mut self, axis: char) -> Result<()> {
        let axis = axis.to_ascii_lowercase();
        let [dx, dy, dz] = self.size;
        let mut new_blocks = vec![0u16; self.blocks.len()];
        for y in 0..dy {
            for z in 0..dz {
                for x in 0..dx {
                    let id = self.blocks[Self::index_of(dx, dz, x, y, z)];
                    let (nx, ny, nz) = match axis {
                        'x' => (dx - 1 - x, y, z),
                        'y' => (x, dy - 1 - y, z),
                        'z' => (x, y, dz - 1 - z),
                        _ => return Err(Error::msg("flip axis must be x, y, or z")),
                    };
                    new_blocks[Self::index_of(dx, dz, nx, ny, nz)] = id;
                }
            }
        }
        self.blocks = new_blocks;
        for ent in &mut self.entities {
            match axis {
                'x' => ent.dx = (dx as f64) - 1.0 - ent.dx,
                'y' => ent.dy = (dy as f64) - 1.0 - ent.dy,
                'z' => ent.dz = (dz as f64) - 1.0 - ent.dz,
                _ => {}
            }
        }
        Ok(())
    }

    pub fn brief_lines(&self) -> Vec<String> {
        vec![
            format!("template={}", self.name),
            format!(
                "size={}x{}x{}",
                self.size[0], self.size[1], self.size[2]
            ),
            format!("palette_n={}", self.palette.len()),
            format!("entities={}", self.entities.len()),
        ]
    }
}
