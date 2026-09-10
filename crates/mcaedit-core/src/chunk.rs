use crate::bitstorage::BitStorage;
use crate::blockstate::BlockState;
use crate::chunk_nbt::{json_to_nbt, nbt_to_json};
use crate::error::{Error, Result};
use crate::palette::{SectionBlocks, SectionDiff};
use fastnbt::Value;
use indexmap::IndexSet;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub const BIOME_SECTION_SIZE: usize = 64; // 4×4×4

#[derive(Clone, Debug)]
pub struct ChunkData {
    pub root: JsonValue,
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl ChunkData {
    pub fn empty(chunk_x: i32, chunk_z: i32) -> Self {
        Self::empty_with_version(chunk_x, chunk_z, crate::mc_version::DEFAULT_DATA_VERSION)
    }

    pub fn empty_with_version(chunk_x: i32, chunk_z: i32, data_version: i32) -> Self {
        let mut root = serde_json::Map::new();
        root.insert("DataVersion".into(), JsonValue::from(data_version));
        root.insert("xPos".into(), JsonValue::from(chunk_x));
        root.insert("zPos".into(), JsonValue::from(chunk_z));
        root.insert("yPos".into(), JsonValue::from(-4));
        root.insert("Status".into(), JsonValue::String("minecraft:full".into()));
        root.insert("sections".into(), JsonValue::Array(Vec::new()));
        root.insert("block_entities".into(), JsonValue::Array(Vec::new()));
        // Empty structure starts/refs so tools can clear/set without creating the tag.
        let mut structures = serde_json::Map::new();
        structures.insert("starts".into(), JsonValue::Object(serde_json::Map::new()));
        structures.insert(
            "References".into(),
            JsonValue::Object(serde_json::Map::new()),
        );
        root.insert("structures".into(), JsonValue::Object(structures));
        Self {
            root: JsonValue::Object(root),
            chunk_x,
            chunk_z,
        }
    }

    pub fn data_version(&self) -> Option<i32> {
        self.root
            .get("DataVersion")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
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

    /// Y indices of present sections (may be empty for brand-new chunks).
    pub fn section_ys(&self) -> Vec<i8> {
        let Ok(sections) = self.sections() else {
            return Vec::new();
        };
        sections
            .iter()
            .filter_map(|s| section_y_of(s).ok())
            .collect()
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

    /// Biome id at block coords (4×4×4 resolution inside the section).
    pub fn get_biome(&self, x: i32, y: i32, z: i32) -> Result<String> {
        let (_, _, _, sy) = local_in_chunk(x, y, z);
        let biomes = self.read_section_biomes(sy)?;
        let (bx, by, bz) = biome_local(x, y, z);
        Ok(biomes.get(bx, by, bz).to_string())
    }

    pub fn set_biome(&mut self, x: i32, y: i32, z: i32, biome: &str) -> Result<String> {
        let (_, _, _, sy) = local_in_chunk(x, y, z);
        let mut biomes = self.read_section_biomes(sy)?;
        let (bx, by, bz) = biome_local(x, y, z);
        let before = biomes.get(bx, by, bz).to_string();
        if before == biome {
            return Ok(before);
        }
        biomes.set(bx, by, bz, biome);
        self.write_section_biomes(sy, &biomes)?;
        Ok(before)
    }

    pub fn read_section_biomes(&self, section_y: i8) -> Result<SectionBiomes> {
        if let Some(idx) = self.find_section_index(section_y)? {
            section_biomes_from_json(&self.sections()?[idx])
        } else {
            Ok(SectionBiomes::filled("minecraft:plains"))
        }
    }

    pub fn write_section_biomes(&mut self, section_y: i8, biomes: &SectionBiomes) -> Result<()> {
        let (palette, data) = biomes.to_palette_nbt()?;
        let mut bio = serde_json::Map::new();
        bio.insert(
            "palette".into(),
            JsonValue::Array(palette.into_iter().map(JsonValue::String).collect()),
        );
        if let Some(data) = data {
            let arr: Vec<JsonValue> = data.into_iter().map(JsonValue::from).collect();
            bio.insert("data".into(), JsonValue::Array(arr));
        }
        let biomes_json = JsonValue::Object(bio);

        if let Some(idx) = self.find_section_index(section_y)? {
            let sec = &mut self.sections_mut()?[idx];
            let obj = sec
                .as_object_mut()
                .ok_or_else(|| Error::msg("section not object"))?;
            obj.insert("biomes".into(), biomes_json);
        } else {
            // Ensure section exists with air blocks + biomes.
            self.write_section_blocks(section_y, &SectionBlocks::air())?;
            let idx = self
                .find_section_index(section_y)?
                .ok_or_else(|| Error::msg("section missing after create"))?;
            let sec = &mut self.sections_mut()?[idx];
            sec.as_object_mut()
                .ok_or_else(|| Error::msg("section not object"))?
                .insert("biomes".into(), biomes_json);
        }
        Ok(())
    }

    pub fn apply_section_diff(&mut self, section_y: i8, diff: &SectionDiff) -> Result<usize> {
        let mut section = self.read_section_blocks(section_y)?;
        let changed = diff.apply_to(&mut section);
        if changed > 0 {
            self.write_section_blocks(section_y, &section)?;
        }
        Ok(changed)
    }

    /// Read vanilla nibble light arrays (`BlockLight` / `SkyLight`).
    /// Missing arrays → `(None, None)`; each present array is 2048 bytes.
    #[allow(clippy::type_complexity)]
    pub fn read_section_light(&self, section_y: i8) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>)> {
        let Some(idx) = self.find_section_index(section_y)? else {
            return Ok((None, None));
        };
        let sec = &self.sections()?[idx];
        let block = sec
            .get("BlockLight")
            .and_then(json_byte_array)
            .filter(|b| b.len() == 2048);
        let sky = sec
            .get("SkyLight")
            .and_then(json_byte_array)
            .filter(|b| b.len() == 2048);
        Ok((block, sky))
    }

    /// Write vanilla nibble light arrays (`BlockLight` / `SkyLight`, 2048 bytes each).
    /// Creates an air section if missing so light can be stored.
    pub fn write_section_light(
        &mut self,
        section_y: i8,
        block_light: &[u8],
        sky_light: &[u8],
    ) -> Result<()> {
        if block_light.len() != 2048 || sky_light.len() != 2048 {
            return Err(Error::msg("section light arrays must be 2048 bytes"));
        }
        if self.find_section_index(section_y)?.is_none() {
            self.write_section_blocks(section_y, &SectionBlocks::air())?;
        }
        let idx = self
            .find_section_index(section_y)?
            .ok_or_else(|| Error::msg("section missing after create"))?;
        let sec = &mut self.sections_mut()?[idx];
        let obj = sec
            .as_object_mut()
            .ok_or_else(|| Error::msg("section not object"))?;
        let bl: Vec<JsonValue> = block_light
            .iter()
            .map(|&b| JsonValue::from(b as i8 as i64))
            .collect();
        let sl: Vec<JsonValue> = sky_light
            .iter()
            .map(|&b| JsonValue::from(b as i8 as i64))
            .collect();
        obj.insert("BlockLight".into(), JsonValue::Array(bl));
        obj.insert("SkyLight".into(), JsonValue::Array(sl));
        Ok(())
    }

    pub fn set_light_on(&mut self, on: bool) {
        if let Some(obj) = self.root.as_object_mut() {
            obj.insert("isLightOn".into(), JsonValue::Bool(on));
        }
    }

    pub fn set_status_full(&mut self) {
        if let Some(obj) = self.root.as_object_mut() {
            obj.insert(
                "Status".into(),
                JsonValue::String("minecraft:full".into()),
            );
        }
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

fn json_byte_array(v: &JsonValue) -> Option<Vec<u8>> {
    match v {
        JsonValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for e in arr {
                let b = e
                    .as_i64()
                    .or_else(|| e.as_u64().map(|u| u as i64))?
                    as i8 as u8;
                out.push(b);
            }
            Some(out)
        }
        JsonValue::String(s) => {
            // Some serializers store byte arrays as base64 — not used by us.
            let _ = s;
            None
        }
        _ => None,
    }
}

/// Local biome cell (0..3) inside a section for block world coords.
pub fn biome_local(x: i32, y: i32, z: i32) -> (u8, u8, u8) {
    let bx = ((x & 15) >> 2) as u8;
    let by = ((y & 15) >> 2) as u8;
    let bz = ((z & 15) >> 2) as u8;
    (bx, by, bz)
}

/// Min block corner of the 4×4×4 biome cell containing (x,y,z).
pub fn biome_cell_origin(x: i32, y: i32, z: i32) -> (i32, i32, i32) {
    (x & !3, y & !3, z & !3)
}

fn biome_bits_for_palette_size(size: usize) -> u8 {
    if size <= 1 {
        return 0;
    }
    let b = (usize::BITS - (size - 1).leading_zeros()) as u8;
    b.max(1)
}

/// Section biomes: palette of biome ids + 64 indices.
#[derive(Clone, Debug)]
pub struct SectionBiomes {
    pub palette: Vec<String>,
    pub indices: Vec<u16>,
}

impl SectionBiomes {
    pub fn filled(biome: impl Into<String>) -> Self {
        Self {
            palette: vec![normalize_biome(biome.into())],
            indices: vec![0; BIOME_SECTION_SIZE],
        }
    }

    pub fn index(x: u8, y: u8, z: u8) -> usize {
        ((y as usize) << 4) | ((z as usize) << 2) | (x as usize)
    }

    pub fn get(&self, x: u8, y: u8, z: u8) -> &str {
        let id = self.indices[Self::index(x, y, z)] as usize;
        &self.palette[id]
    }

    pub fn set(&mut self, x: u8, y: u8, z: u8, biome: &str) {
        let biome = normalize_biome(biome.to_string());
        let idx = Self::index(x, y, z);
        if let Some(pos) = self.palette.iter().position(|s| s == &biome) {
            self.indices[idx] = pos as u16;
            return;
        }
        let pos = self.palette.len();
        self.palette.push(biome);
        self.indices[idx] = pos as u16;
    }

    pub fn to_palette_nbt(&self) -> Result<(Vec<String>, Option<Vec<i64>>)> {
        let mut used: IndexSet<String> = IndexSet::new();
        let mut ids = Vec::with_capacity(BIOME_SECTION_SIZE);
        for &id in &self.indices {
            let key = self.palette[id as usize].clone();
            let (idx, _) = used.insert_full(key);
            ids.push(idx as u32);
        }
        let palette: Vec<String> = used.into_iter().collect();
        let bits = biome_bits_for_palette_size(palette.len());
        if bits == 0 {
            return Ok((palette, None));
        }
        let storage = BitStorage::pack_values(bits, &ids);
        Ok((palette, Some(storage.into_raw())))
    }

    pub fn from_palette_nbt(palette: &[String], data: Option<&[i64]>) -> Result<Self> {
        if palette.is_empty() {
            return Ok(Self::filled("minecraft:plains"));
        }
        if palette.len() == 1 || data.map(|d| d.is_empty()).unwrap_or(true) {
            return Ok(Self::filled(palette[0].clone()));
        }
        let bits = biome_bits_for_palette_size(palette.len());
        if bits == 0 {
            return Ok(Self::filled(palette[0].clone()));
        }
        let raw = data.unwrap_or(&[]);
        let storage =
            BitStorage::from_raw(bits, BIOME_SECTION_SIZE, raw.to_vec()).map_err(Error::msg)?;
        let mut indices = Vec::with_capacity(BIOME_SECTION_SIZE);
        for i in 0..BIOME_SECTION_SIZE {
            let id = storage.get(i) as usize;
            if id >= palette.len() {
                return Err(Error::msg(format!("biome palette index {id} out of range")));
            }
            indices.push(id as u16);
        }
        Ok(Self {
            palette: palette.to_vec(),
            indices,
        })
    }
}

fn normalize_biome(s: String) -> String {
    let t = s.trim();
    if t.contains(':') {
        t.to_string()
    } else {
        format!("minecraft:{t}")
    }
}

fn section_biomes_from_json(sec: &JsonValue) -> Result<SectionBiomes> {
    let Some(bio) = sec.get("biomes") else {
        return Ok(SectionBiomes::filled("minecraft:plains"));
    };
    let palette: Vec<String> = bio
        .get("palette")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::msg("biomes missing palette"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(|s| normalize_biome(s.to_string()))
                .ok_or_else(|| Error::msg("biome palette entry not string"))
        })
        .collect::<Result<Vec<_>>>()?;
    let data: Option<Vec<i64>> = bio.get("data").and_then(|d| {
        d.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
                .collect()
        })
    });
    SectionBiomes::from_palette_nbt(&palette, data.as_deref())
}
