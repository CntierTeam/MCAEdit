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
