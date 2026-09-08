use crate::error::{Error, Result};
use crate::region::{atomic_write, copy_region_if_needed, local_chunk, region_coords, region_file_name};
use fastnbt::Value;
use mca::{Compression, RegionReader, RegionWriter};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityRecord {
    pub uuid: String,
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Full entity NBT as JSON object.
    pub nbt: JsonValue,
}

impl EntityRecord {
    pub fn from_nbt_json(nbt: JsonValue) -> Result<Self> {
        let id = nbt
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let (x, y, z) = pos_of(&nbt);
        let uuid = uuid_of(&nbt).unwrap_or_else(|| Uuid::new_v4().to_string());
        Ok(Self {
            uuid,
            id,
            x,
            y,
            z,
            nbt,
        })
    }

    pub fn brief(&self) -> String {
        format!(
            "{id} {uuid} @ {x:.1},{y:.1},{z:.1}",
            id = self.id,
            uuid = self.uuid,
            x = self.x,
            y = self.y,
            z = self.z
        )
    }
}

fn pos_of(nbt: &JsonValue) -> (f64, f64, f64) {
    if let Some(arr) = nbt.get("Pos").and_then(|v| v.as_array()) {
        let x = arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
        return (x, y, z);
    }
    (0.0, 0.0, 0.0)
}

fn uuid_of(nbt: &JsonValue) -> Option<String> {
    if let Some(s) = nbt.get("UUID").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    // UUID IntArray [4]
    if let Some(arr) = nbt.get("UUID").and_then(|v| v.as_array()) {
        if arr.len() == 4 {
            let parts: Option<Vec<u32>> = arr
                .iter()
                .map(|v| v.as_i64().map(|i| i as u32))
                .collect();
            if let Some(p) = parts {
                let most = ((p[0] as u64) << 32) | p[1] as u64;
                let least = ((p[2] as u64) << 32) | p[3] as u64;
                let u = Uuid::from_u64_pair(most, least);
                return Some(u.to_string());
            }
        }
    }
    nbt.get("UUIDLeast").map(|_| Uuid::new_v4().to_string())
}

#[derive(Clone, Debug)]
pub struct EntityChunk {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub entities: Vec<JsonValue>,
}

impl EntityChunk {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut compound = std::collections::HashMap::new();
        compound.insert(
            "Position".into(),
            Value::IntArray(fastnbt::IntArray::new(vec![self.chunk_x, self.chunk_z])),
        );
        let list = self
            .entities
            .iter()
            .map(json_to_nbt)
            .collect::<Result<Vec<_>>>()?;
        compound.insert("Entities".into(), Value::List(list));
        Ok(fastnbt::to_bytes(&Value::Compound(compound))?)
    }

    pub fn from_bytes(bytes: &[u8], chunk_x: i32, chunk_z: i32) -> Result<Self> {
        let value: Value = fastnbt::from_bytes(bytes)?;
        let Value::Compound(map) = value else {
            return Err(Error::msg("entity chunk root not compound"));
        };
        let entities = match map.get("Entities") {
            Some(Value::List(list)) => list
                .iter()
                .cloned()
                .map(nbt_to_json)
                .collect::<Result<Vec<_>>>()?,
            _ => Vec::new(),
        };
        Ok(Self {
            chunk_x,
            chunk_z,
            entities,
        })
    }
}

fn nbt_to_json(value: Value) -> Result<JsonValue> {
    crate::chunk_nbt::nbt_to_json(value)
}

fn json_to_nbt(value: &JsonValue) -> Result<Value> {
    crate::chunk_nbt::json_to_nbt(value)
}

pub struct EntityStore {
    dir: PathBuf,
}

impl EntityStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn read_chunk(&self, chunk_x: i32, chunk_z: i32) -> Result<EntityChunk> {
        let (rx, rz) = region_coords(chunk_x, chunk_z);
        let path = self.dir.join(region_file_name(rx, rz));
        if !path.exists() {
            return Ok(EntityChunk {
                chunk_x,
                chunk_z,
                entities: Vec::new(),
            });
        }
        let bytes = fs::read(&path)?;
        let mut region = RegionReader::new(&bytes)?;
        let (lx, lz) = local_chunk(chunk_x, chunk_z);
        let Some(raw) = region.chunk(lx, lz)? else {
            return Ok(EntityChunk {
                chunk_x,
                chunk_z,
                entities: Vec::new(),
            });
        };
        EntityChunk::from_bytes(raw, chunk_x, chunk_z)
    }

    pub fn write_chunk(&self, chunk: &EntityChunk) -> Result<()> {
        let (rx, rz) = region_coords(chunk.chunk_x, chunk.chunk_z);
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(region_file_name(rx, rz));
        let nbt = chunk.to_bytes()?;
        let (lx, lz) = local_chunk(chunk.chunk_x, chunk.chunk_z);
        if path.exists() {
            let bytes = fs::read(&path)?;
            let region = RegionReader::new(&bytes)?;
            let mut writer = region.into_writer(())?;
            writer.set_chunk(lx, lz, nbt, Compression::default())?;
            let mut out = Vec::new();
            writer.write(&mut out)?;
            atomic_write(&path, &out)?;
        } else {
            let mut writer = RegionWriter::new();
            writer.set_chunk(lx, lz, nbt, Compression::default())?;
            let mut out = Vec::new();
            writer.write(&mut out)?;
            atomic_write(&path, &out)?;
        }
        Ok(())
    }

    pub fn list_near(
        &self,
        x: f64,
        y: f64,
        z: f64,
        radius: f64,
    ) -> Result<Vec<EntityRecord>> {
        let r2 = radius * radius;
        let min_cx = ((x - radius).floor() as i32) >> 4;
        let max_cx = ((x + radius).floor() as i32) >> 4;
        let min_cz = ((z - radius).floor() as i32) >> 4;
        let max_cz = ((z + radius).floor() as i32) >> 4;
        let mut out = Vec::new();
        for cx in min_cx..=max_cx {
            for cz in min_cz..=max_cz {
                let chunk = self.read_chunk(cx, cz)?;
                for ent in chunk.entities {
                    let rec = EntityRecord::from_nbt_json(ent)?;
                    let dx = rec.x - x;
                    let dy = rec.y - y;
                    let dz = rec.z - z;
                    if dx * dx + dy * dy + dz * dz <= r2 {
                        out.push(rec);
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn find_by_uuid(&self, uuid: &str, hint_cx: i32, hint_cz: i32) -> Result<Option<(i32, i32, usize, JsonValue)>> {
        // search hint chunk first, then neighbors
        for cz in (hint_cz - 1)..=(hint_cz + 1) {
            for cx in (hint_cx - 1)..=(hint_cx + 1) {
                let chunk = self.read_chunk(cx, cz)?;
                for (i, ent) in chunk.entities.iter().enumerate() {
                    let rec = EntityRecord::from_nbt_json(ent.clone())?;
                    if rec.uuid == uuid {
                        return Ok(Some((cx, cz, i, ent.clone())));
                    }
                }
            }
        }
        Ok(None)
    }
}

pub fn ensure_entity_region_copied(src: &Path, dst: &Path, rx: i32, rz: i32) -> Result<PathBuf> {
    copy_region_if_needed(src, dst, rx, rz)
}

