use crate::blockstate::BlockState;
use crate::chunk_nbt::{json_to_nbt, nbt_to_json};
use crate::error::{Error, Result};
use crate::palette::{SectionBlocks, SectionDiff};
use fastnbt::Value;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ChunkData {
    pub root: JsonValue,
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl ChunkData {
    pub fn empty(chunk_x: i32, chunk_z: i32) -> Self {
        let mut root = serde_json::Map::new();
        root.insert("DataVersion".into(), JsonValue::from(4556)); // MC 26.x-ish
        root.insert("xPos".into(), JsonValue::from(chunk_x));
        root.insert("zPos".into(), JsonValue::from(chunk_z));
        root.insert("yPos".into(), JsonValue::from(-4));
        root.insert("Status".into(), JsonValue::String("minecraft:full".into()));
        root.insert("sections".into(), JsonValue::Array(Vec::new()));
        root.insert("block_entities".into(), JsonValue::Array(Vec::new()));
        Self {
            root: JsonValue::Object(root),
            chunk_x,
            chunk_z,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let value: Value = fastnbt::from_bytes(bytes)?;
        let root = nbt_to_json(value)?;
        let chunk_x = root.get("xPos").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let chunk_z = root.get("zPos").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        Ok(Self {
            root,
            chunk_x,
            chunk_z,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let nbt = json_to_nbt(&self.root)?;
        Ok(fastnbt::to_bytes(&nbt)?)
    }

    pub fn sections_mut(&mut self) -> Result<&mut Vec<JsonValue>> {
        let sections = self
            .root
            .get_mut("sections")
            .ok_or_else(|| Error::msg("chunk missing sections"))?;
        sections
            .as_array_mut()
            .ok_or_else(|| Error::msg("sections is not a list"))
    }

    pub fn sections(&self) -> Result<&Vec<JsonValue>> {
        self.root
            .get("sections")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::msg("chunk missing sections"))
    }

    pub fn find_section_index(&self, section_y: i8) -> Result<Option<usize>> {
        for (i, sec) in self.sections()?.iter().enumerate() {
            if section_y_of(sec)? == section_y {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    pub fn read_section_blocks(&self, section_y: i8) -> Result<SectionBlocks> {
        if let Some(idx) = self.find_section_index(section_y)? {
            section_blocks_from_json(&self.sections()?[idx])
        } else {
            Ok(SectionBlocks::air())
        }
    }

    pub fn write_section_blocks(&mut self, section_y: i8, blocks: &SectionBlocks) -> Result<()> {
        let (palette, data) = blocks.to_palette_nbt()?;
        let mut bs = serde_json::Map::new();
        bs.insert("palette".into(), JsonValue::Array(palette));
        if let Some(data) = data {
            let arr: Vec<JsonValue> = data.into_iter().map(JsonValue::from).collect();
            bs.insert("data".into(), JsonValue::Array(arr));
        }
        let block_states = JsonValue::Object(bs);

        if let Some(idx) = self.find_section_index(section_y)? {
            let sec = &mut self.sections_mut()?[idx];
            let obj = sec
                .as_object_mut()
                .ok_or_else(|| Error::msg("section not object"))?;
            obj.insert("block_states".into(), block_states);
        } else {
            let mut obj = serde_json::Map::new();
            obj.insert("Y".into(), JsonValue::from(section_y as i64));
            obj.insert("block_states".into(), block_states);
            let mut biomes = serde_json::Map::new();
            biomes.insert(
                "palette".into(),
                JsonValue::Array(vec![JsonValue::String("minecraft:plains".into())]),
            );
            obj.insert("biomes".into(), JsonValue::Object(biomes));
            self.sections_mut()?.push(JsonValue::Object(obj));
        }
        Ok(())
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Result<BlockState> {
        let (lx, ly, lz, sy) = local_in_chunk(x, y, z);
        Ok(self.read_section_blocks(sy)?.get(lx, ly, lz).clone())
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) -> Result<BlockState> {
        let (lx, ly, lz, sy) = local_in_chunk(x, y, z);
        let mut section = self.read_section_blocks(sy)?;
        let before = section.get(lx, ly, lz).clone();
        section.set(lx, ly, lz, state);
        self.write_section_blocks(sy, &section)?;
        Ok(before)
    }

    pub fn apply_section_diff(&mut self, section_y: i8, diff: &SectionDiff) -> Result<usize> {
        let mut section = self.read_section_blocks(section_y)?;
        let changed = diff.apply_to(&mut section);
        if changed > 0 {
            self.write_section_blocks(section_y, &section)?;
        }
        Ok(changed)
    }

    pub fn section_palette_list(&self, section_y: i8) -> Result<Vec<BlockState>> {
        let Some(idx) = self.find_section_index(section_y)? else {
            return Ok(vec![BlockState::air()]);
        };
        let sec = &self.sections()?[idx];
        let palette = sec
            .pointer("/block_states/palette")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::msg("section missing block_states.palette"))?;
        // Parse entries one-by-one (no data array).
        let mut out = Vec::with_capacity(palette.len());
        for entry in palette {
            let one = SectionBlocks::from_palette_nbt(std::slice::from_ref(entry), Some(&[]))?;
            out.push(one.palette[0].clone());
        }
        if out.is_empty() {
            out.push(BlockState::air());
        }
        Ok(out)
    }

    pub fn count_non_air(&self) -> Result<(usize, BTreeMap<String, usize>)> {
        let mut total = 0usize;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for sec in self.sections()? {
            let y = section_y_of(sec)?;
            let blocks = self.read_section_blocks(y)?;
            let (t, c) = blocks.non_air_counts();
            total += t;
            for (k, v) in c {
                *counts.entry(k).or_default() += v;
            }
        }
        Ok((total, counts))
    }
}

pub fn local_in_chunk(x: i32, y: i32, z: i32) -> (u8, u8, u8, i8) {
    let lx = (x & 15) as u8;
    let lz = (z & 15) as u8;
    let sy = (y >> 4) as i8;
    let ly = (y & 15) as u8;
    (lx, ly, lz, sy)
}

pub fn section_y_of(sec: &JsonValue) -> Result<i8> {
    let y = sec
        .get("Y")
        .or_else(|| sec.get("y"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| Error::msg("section missing Y"))?;
    Ok(y as i8)
}

fn section_blocks_from_json(sec: &JsonValue) -> Result<SectionBlocks> {
    let Some(bs) = sec.get("block_states") else {
        return Ok(SectionBlocks::air());
    };
    let palette = bs
        .get("palette")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::msg("block_states missing palette"))?;
    let data: Option<Vec<i64>> = bs.get("data").and_then(|d| {
        d.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
                .collect()
        })
    });
    SectionBlocks::from_palette_nbt(palette, data.as_deref())
}
