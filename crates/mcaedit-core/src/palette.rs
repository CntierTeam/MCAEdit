use crate::bitstorage::BitStorage;
use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub const SECTION_SIZE: usize = 4096;

fn ceil_log2(n: usize) -> u8 {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

/// Vanilla block-state disc bits for a palette of `size` entries.
pub fn block_bits_for_palette_size(size: usize) -> u8 {
    match ceil_log2(size) {
        0 => 0,
        1..=4 => 4,
        b @ 5..=8 => b,
        b => b,
    }
}

/// Compact section: palette + 4096 indices (heap only).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionBlocks {
    pub palette: Vec<BlockState>,
    pub indices: Vec<u16>,
}

impl SectionBlocks {
    pub fn filled(state: BlockState) -> Self {
        Self {
            palette: vec![state],
            indices: vec![0; SECTION_SIZE],
        }
    }

    pub fn air() -> Self {
        Self::filled(BlockState::air())
    }

    pub fn index(x: u8, y: u8, z: u8) -> usize {
        ((y as usize) << 4 | z as usize) << 4 | x as usize
    }

    pub fn get(&self, x: u8, y: u8, z: u8) -> &BlockState {
        let id = self.indices[Self::index(x, y, z)] as usize;
        &self.palette[id]
    }

    pub fn set(&mut self, x: u8, y: u8, z: u8, state: BlockState) {
        let idx = Self::index(x, y, z);
        if let Some(pos) = self.palette.iter().position(|s| s == &state) {
            self.indices[idx] = pos as u16;
            return;
        }
        let pos = self.palette.len();
        if pos > u16::MAX as usize {
            // should never happen for a section
            return;
        }
        self.palette.push(state);
        self.indices[idx] = pos as u16;
    }

    pub fn from_palette_nbt(palette: &[JsonValue], data: Option<&[i64]>) -> Result<Self> {
        let states: Vec<BlockState> = palette
            .iter()
            .map(blockstate_from_nbt_json)
            .collect::<Result<Vec<_>>>()?;
        if states.is_empty() {
            return Err(Error::msg("empty section palette"));
        }
        if states.len() == 1 || data.map(|d| d.is_empty()).unwrap_or(true) {
            return Ok(Self::filled(states[0].clone()));
        }
        let bits = block_bits_for_palette_size(states.len());
        if bits == 0 {
            return Ok(Self::filled(states[0].clone()));
        }
        let raw = data.unwrap_or(&[]);
        let storage =
            BitStorage::from_raw(bits, SECTION_SIZE, raw.to_vec()).map_err(Error::msg)?;
        let mut indices = Vec::with_capacity(SECTION_SIZE);
        for i in 0..SECTION_SIZE {
            let id = storage.get(i) as usize;
            if id >= states.len() {
                return Err(Error::msg(format!("palette index {id} out of range")));
            }
            indices.push(id as u16);
        }
        Ok(Self {
            palette: states,
            indices,
        })
    }

    pub fn to_palette_nbt(&self) -> Result<(Vec<JsonValue>, Option<Vec<i64>>)> {
        // Re-pack to drop unused palette entries.
        let mut used: IndexSet<String> = IndexSet::new();
        let mut ids = Vec::with_capacity(SECTION_SIZE);
        for &id in &self.indices {
            let key = self.palette[id as usize].to_compact();
            let (idx, _) = used.insert_full(key);
            ids.push(idx as u32);
        }
        let states: Vec<BlockState> = used
            .iter()
            .map(|s| BlockState::parse(s).map_err(Error::msg))
            .collect::<Result<_>>()?;
        let palette_json: Vec<JsonValue> = states.iter().map(blockstate_to_nbt_json).collect();
        let bits = block_bits_for_palette_size(states.len());
        if bits == 0 {
            return Ok((palette_json, None));
        }
        let storage = BitStorage::pack_values(bits, &ids);
        Ok((palette_json, Some(storage.into_raw())))
    }

    pub fn non_air_counts(&self) -> (usize, BTreeMap<String, usize>) {
        let mut total = 0usize;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for &id in &self.indices {
            let b = &self.palette[id as usize];
            if !b.is_air_like() {
                total += 1;
                *counts.entry(b.to_compact()).or_default() += 1;
            }
        }
        (total, counts)
    }
}

fn blockstate_from_nbt_json(v: &JsonValue) -> Result<BlockState> {
    match v {
        JsonValue::String(s) => BlockState::parse(s).map_err(Error::msg),
        JsonValue::Object(map) => {
            let name = map
                .get("Name")
                .or_else(|| map.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| Error::msg("palette entry missing Name"))?
                .to_string();
            let mut properties = BTreeMap::new();
            if let Some(JsonValue::Object(props)) =
                map.get("Properties").or_else(|| map.get("properties"))
            {
                for (k, val) in props {
                    let s = match val {
                        JsonValue::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    properties.insert(k.clone(), s);
                }
            }
            Ok(BlockState { name, properties })
        }
        other => Err(Error::msg(format!("bad palette entry: {other}"))),
    }
}

fn blockstate_to_nbt_json(state: &BlockState) -> JsonValue {
    let mut map = serde_json::Map::new();
    map.insert("Name".into(), JsonValue::String(state.name.clone()));
    if !state.properties.is_empty() {
        let mut props = serde_json::Map::new();
        for (k, v) in &state.properties {
            props.insert(k.clone(), JsonValue::String(v.clone()));
        }
        map.insert("Properties".into(), JsonValue::Object(props));
    }
    JsonValue::Object(map)
}

/// Sparse section diff. Cells set to void_air are skipped (keep existing).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SectionDiff {
    pub cells: BTreeMap<String, BlockState>,
}

impl SectionDiff {
    pub fn set(&mut self, x: u8, y: u8, z: u8, state: BlockState) {
        self.cells.insert(format!("{x},{y},{z}"), state);
    }

    pub fn apply_to(&self, section: &mut SectionBlocks) -> usize {
        let mut changed = 0;
        for (key, state) in &self.cells {
            if state.is_empty_sentinel() {
                continue;
            }
            let (x, y, z) = parse_local(key);
            if section.get(x, y, z) != state {
                section.set(x, y, z, state.clone());
                changed += 1;
            }
        }
        changed
    }

    pub fn capture_before(&self, section: &SectionBlocks) -> SectionDiff {
        let mut before = SectionDiff::default();
        for key in self.cells.keys() {
            let (x, y, z) = parse_local(key);
            before
                .cells
                .insert(key.clone(), section.get(x, y, z).clone());
        }
        before
    }
}

fn parse_local(key: &str) -> (u8, u8, u8) {
    let mut it = key.split(',');
    let x = it.next().unwrap_or("0").parse().unwrap_or(0);
    let y = it.next().unwrap_or("0").parse().unwrap_or(0);
    let z = it.next().unwrap_or("0").parse().unwrap_or(0);
    (x, y, z)
}
