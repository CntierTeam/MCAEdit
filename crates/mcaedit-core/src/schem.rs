//! Sponge Schematic (`.schem`) — v2 write; v1/v2/v3 read.
//!
//! WorldEdit / FAWE wrap fields under a root `Schematic` compound; MCAEdit accepts
//! both wrapped and flat roots. Classic MCEdit `.schematic` is rejected with a
//! clear error (not Sponge).

use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::mc_version::{ResolvedVersion, DEFAULT_DATA_VERSION};
use crate::template::{Template, TemplateEntity};
use crate::world::WorldView;
use fastnbt::Value;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indexmap::IndexSet;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// How many block ids to list in `schem info` summaries.
pub const INFO_TOP_BLOCKS: usize = 16;

/// Export AABB from world to Sponge Schematic v2 (`.schem`, gzip NBT).
#[allow(clippy::too_many_arguments)]
pub fn export_aabb(
    world: &WorldView<'_>,
    x1: i32,
    y1: i32,
    z1: i32,
    x2: i32,
    y2: i32,
    z2: i32,
    out: &Path,
) -> Result<SchemInfo> {
    let tpl = Template::capture(world, "schem", x1, y1, z1, x2, y2, z2)?;
    let dv = world
        .session
        .meta
        .data_version
        .unwrap_or(DEFAULT_DATA_VERSION);
    write_template_v2(&tpl, out, dv)?;
    info(out)
}

pub fn export_template(tpl: &Template, out: &Path) -> Result<SchemInfo> {
    write_template_v2(tpl, out, DEFAULT_DATA_VERSION)?;
    info(out)
}

pub fn export_template_with_version(
    tpl: &Template,
    out: &Path,
    data_version: i32,
) -> Result<SchemInfo> {
    write_template_v2(tpl, out, data_version)?;
    info(out)
}

/// Import `.schem` into a Template (blocks + relative entities when present).
pub fn import_to_template(path: &Path, name: &str) -> Result<Template> {
    let root = read_schem_root(path)?;
    template_from_schem(&root, name)
}

/// Paste `.schem` at origin into the session world (history via template paste).
/// Applies schematic `Offset` per Sponge spec (`at + Offset`).
pub fn import_paste(
    world: &mut WorldView<'_>,
    path: &Path,
    ox: i32,
    oy: i32,
    oz: i32,
) -> Result<crate::action::Action> {
    let root = read_schem_root(path)?;
    let offset = read_offset(&root);
    let tpl = template_from_schem(&root, "schem-import")?;
    tpl.paste_into(world, ox + offset[0], oy + offset[1], oz + offset[2])
}

#[derive(Clone, Debug)]
pub struct SchemInfo {
    pub version: i32,
    pub data_version: Option<i32>,
    /// Known release alias for DataVersion when mapped (e.g. `26.2`, `1.20.1`).
    pub mc: Option<String>,
    pub width: u16,
    pub height: u16,
    pub length: u16,
    pub offset: [i32; 3],
    pub palette_n: usize,
    pub volume: usize,
    pub entities: usize,
    pub block_entities: usize,
    /// Top block states by count (includes air).
    pub top_blocks: Vec<(String, usize)>,
    pub path: String,
}

impl SchemInfo {
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("schem={}", self.path),
            format!("version={}", self.version),
            format!(
                "DataVersion={}",
                self.data_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!("mc={}", self.mc.as_deref().unwrap_or("-")),
            format!(
                "size={}x{}x{}",
                self.width, self.height, self.length
            ),
            format!(
                "offset={},{},{}",
                self.offset[0], self.offset[1], self.offset[2]
            ),
            format!("volume={}", self.volume),
            format!("palette_n={}", self.palette_n),
            format!("entities={}", self.entities),
            format!("block_entities={}", self.block_entities),
        ];
        if !self.top_blocks.is_empty() {
            let summary = self
                .top_blocks
                .iter()
                .map(|(name, n)| format!("{name}={n}"))
                .collect::<Vec<_>>()
                .join(",");
            out.push(format!("blocks_top={summary}"));
        }
        out
    }
}

/// Parse-only metadata + block histogram (no world session required).
pub fn info(path: &Path) -> Result<SchemInfo> {
    let root = read_schem_root(path)?;
    info_from_root(&root, path)
}

fn info_from_root(root: &Value, path: &Path) -> Result<SchemInfo> {
    let version = compound_i32(root, "Version").unwrap_or(2);
    let data_version = compound_i32(root, "DataVersion");
    let mc = data_version.map(|dv| ResolvedVersion::from_data_version(dv).name);
    let (w, h, l, palette_n) = dims_from_root(root)?;
    let offset = read_offset(root);
    let volume = (w as usize)
        .checked_mul(h as usize)
        .and_then(|v| v.checked_mul(l as usize))
        .ok_or_else(|| Error::msg("schem volume overflow"))?;
    let entities = list_len(root, "Entities");
    let block_entities = block_entities_len(root, version);
    let top_blocks = block_histogram(root, version, volume, INFO_TOP_BLOCKS)?;

    Ok(SchemInfo {
        version,
        data_version,
        mc,
        width: w,
        height: h,
        length: l,
        offset,
        palette_n,
        volume,
        entities,
        block_entities,
        top_blocks,
        path: path.display().to_string(),
    })
}

fn write_template_v2(tpl: &Template, out: &Path, data_version: i32) -> Result<()> {
    let [w, h, l] = tpl.size;
    if w > u16::MAX as u32 || h > u16::MAX as u32 || l > u16::MAX as u32 {
        return Err(Error::msg("schem dimensions exceed u16"));
    }
    let mut palette = HashMap::new();
    for (i, name) in tpl.palette.iter().enumerate() {
        palette.insert(name.clone(), Value::Int(i as i32));
    }
    let mut block_data = Vec::new();
    for &id in &tpl.blocks {
        write_varint(&mut block_data, id as u32);
    }

    let mut schem = HashMap::new();
    schem.insert("Version".into(), Value::Int(2));
    schem.insert("DataVersion".into(), Value::Int(data_version));
    schem.insert("Width".into(), Value::Short(w as i16));
    schem.insert("Height".into(), Value::Short(h as i16));
    schem.insert("Length".into(), Value::Short(l as i16));
    schem.insert(
        "Offset".into(),
        Value::IntArray(fastnbt::IntArray::new(vec![0, 0, 0])),
    );
    schem.insert(
        "Palette".into(),
        Value::Compound(palette.into_iter().collect()),
    );
    schem.insert("PaletteMax".into(), Value::Int(tpl.palette.len() as i32));
    schem.insert(
        "BlockData".into(),
        Value::ByteArray(fastnbt::ByteArray::new(
            block_data.into_iter().map(|b| b as i8).collect(),
        )),
    );
    schem.insert("BlockEntities".into(), Value::List(Vec::new()));

    if !tpl.entities.is_empty() {
        let mut ents = Vec::new();
        for ent in &tpl.entities {
            let mut map = HashMap::new();
            map.insert(
                "Pos".into(),
                Value::List(vec![
                    Value::Double(ent.dx),
                    Value::Double(ent.dy),
                    Value::Double(ent.dz),
                ]),
            );
            if let Some(id) = ent.nbt.get("id").and_then(|v| v.as_str()) {
                map.insert("Id".into(), Value::String(id.to_string()));
            }
            if let Ok(nbt) = crate::chunk_nbt::json_to_nbt(&ent.nbt) {
                map.insert("Data".into(), nbt);
            }
            ents.push(Value::Compound(map.into_iter().collect()));
        }
        schem.insert("Entities".into(), Value::List(ents));
    }

    // WE/FAWE expect a named `Schematic` compound under the root.
    let mut root = HashMap::new();
    root.insert(
        "Schematic".into(),
        Value::Compound(schem.into_iter().collect()),
    );

    let bytes = fastnbt::to_bytes(&Value::Compound(root.into_iter().collect()))?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(out)?;
    let mut enc = GzEncoder::new(file, Compression::default());
    enc.write_all(&bytes)?;
    enc.finish()?;
    Ok(())
}

fn read_schem_root(path: &Path) -> Result<Value> {
    let raw = fs::read(path)?;
    let mut buf = Vec::new();
    let bytes = match GzDecoder::new(&raw[..]).read_to_end(&mut buf) {
        Ok(_) if !buf.is_empty() => buf,
        _ => raw,
    };
    let value: Value = fastnbt::from_bytes(&bytes).map_err(|e| Error::msg(e.to_string()))?;
    let root = unwrap_schematic(value)?;
    reject_classic_schematic(&root)?;
    Ok(root)
}

/// Accept flat Sponge root or WE/FAWE `{ Schematic: { … } }` wrapper.
fn unwrap_schematic(value: Value) -> Result<Value> {
    let Value::Compound(map) = value else {
        return Err(Error::msg("schem root is not a compound"));
    };
    if map.contains_key("Width") || map.contains_key("Version") || map.contains_key("Blocks") {
        return Ok(Value::Compound(map));
    }
    if let Some(inner) = map.get("Schematic").cloned() {
        if matches!(&inner, Value::Compound(_)) {
            return Ok(inner);
        }
    }
    // Case-insensitive fallback
    for (k, v) in &map {
        if k.eq_ignore_ascii_case("Schematic") {
            if matches!(v, Value::Compound(_)) {
                return Ok(v.clone());
            }
        }
    }
    Err(Error::msg(
        "schem missing Schematic compound (or Width/Version); not a Sponge .schem?",
    ))
}

fn reject_classic_schematic(root: &Value) -> Result<()> {
    // Classic MCEdit: Materials + Blocks byte[] without Sponge Palette / Version.
    let has_materials = compound_child(root, "Materials").is_some();
    let has_classic_blocks = matches!(
        compound_child(root, "Blocks"),
        Some(Value::ByteArray(_))
    );
    let has_palette = find_palette(root).is_some()
        || compound_child(root, "Blocks")
            .and_then(|b| find_palette(b))
            .is_some();
    let has_version = compound_i32(root, "Version").is_some();
    if has_materials && has_classic_blocks && !has_palette && !has_version {
        return Err(Error::msg(
            "classic MCEdit .schematic is not supported; convert to Sponge .schem (v1–v3) with WorldEdit/FAWE",
        ));
    }
    Ok(())
}

fn dims_from_root(root: &Value) -> Result<(u16, u16, u16, usize)> {
    let w = compound_u16(root, "Width").ok_or_else(|| Error::msg("schem missing Width"))?;
    let h = compound_u16(root, "Height").ok_or_else(|| Error::msg("schem missing Height"))?;
    let l = compound_u16(root, "Length").ok_or_else(|| Error::msg("schem missing Length"))?;
    let palette_n = match palette_value(root)? {
        Some(Value::Compound(m)) => m.len(),
        Some(Value::List(v)) => v.len(),
        _ => 0,
    };
    Ok((w, h, l, palette_n))
}

fn palette_value(root: &Value) -> Result<Option<&Value>> {
    let version = compound_i32(root, "Version").unwrap_or(2);
    if version >= 3 {
        if let Some(blocks) = compound_child(root, "Blocks") {
            return Ok(find_palette(blocks));
        }
        // Tolerate flat v3-ish files
        return Ok(find_palette(root));
    }
    Ok(find_palette(root))
}

fn template_from_schem(root: &Value, name: &str) -> Result<Template> {
    let version = compound_i32(root, "Version").unwrap_or(2);
    let (w, h, l, _) = dims_from_root(root)?;
    let (palette_map, block_bytes) = block_payload(root, version)?;

    let mut ordered: Vec<String> = vec![String::new(); palette_map.len()];
    for (name, idx) in &palette_map {
        if *idx >= ordered.len() {
            ordered.resize(idx + 1, String::new());
        }
        ordered[*idx] = name.clone();
    }
    for slot in ordered.iter_mut() {
        if slot.is_empty() {
            *slot = BlockState::air().to_compact();
        }
    }

    let volume = (w as usize) * (h as usize) * (l as usize);
    let ids = read_varints(&block_bytes, volume)?;
    if ids.len() != volume {
        return Err(Error::msg(format!(
            "schem BlockData length {} != volume {volume}",
            ids.len()
        )));
    }

    let mut dense: IndexSet<String> = IndexSet::new();
    dense.insert(BlockState::air().to_compact());
    let mut blocks = Vec::with_capacity(volume);
    for id in ids {
        let name = ordered
            .get(id as usize)
            .cloned()
            .unwrap_or_else(|| BlockState::air().to_compact());
        let (idx, _) = dense.insert_full(normalize_block_name(&name));
        blocks.push(idx as u16);
    }

    let mut entities = Vec::new();
    if let Some(Value::List(list)) = compound_child(root, "Entities") {
        for ent in list {
            let Value::Compound(map) = ent else { continue };
            let (dx, dy, dz) = entity_pos(map);
            let nbt = if let Some(data) = map.get("Data") {
                crate::chunk_nbt::nbt_to_json(data.clone()).unwrap_or(serde_json::json!({}))
            } else {
                let mut obj = serde_json::Map::new();
                if let Some(Value::String(id)) = map.get("Id").or_else(|| map.get("id")) {
                    obj.insert("id".into(), serde_json::Value::String(id.clone()));
                }
                serde_json::Value::Object(obj)
            };
            entities.push(TemplateEntity { dx, dy, dz, nbt });
        }
    }

    Ok(Template {
        name: name.to_string(),
        size: [w as u32, h as u32, l as u32],
        palette: dense.into_iter().collect(),
        blocks,
        entities,
    })
}

fn block_payload(root: &Value, version: i32) -> Result<(HashMap<String, usize>, Vec<u8>)> {
    if version >= 3 {
        let blocks = compound_child(root, "Blocks")
            .ok_or_else(|| Error::msg("v3 schem missing Blocks"))?;
        let palette = find_palette(blocks).ok_or_else(|| Error::msg("v3 missing Palette"))?;
        let data = compound_bytes(blocks, "Data")
            .or_else(|| compound_bytes(blocks, "BlockData"))
            .ok_or_else(|| Error::msg("v3 missing Block Data"))?;
        Ok((palette_to_index_map(palette)?, data))
    } else {
        // v1 / v2
        let palette = find_palette(root).ok_or_else(|| Error::msg("schem missing Palette"))?;
        let data = compound_bytes(root, "BlockData")
            .ok_or_else(|| Error::msg("schem missing BlockData"))?;
        Ok((palette_to_index_map(palette)?, data))
    }
}

fn block_histogram(
    root: &Value,
    version: i32,
    volume: usize,
    top_n: usize,
) -> Result<Vec<(String, usize)>> {
    if volume == 0 {
        return Ok(Vec::new());
    }
    let (palette_map, block_bytes) = match block_payload(root, version) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    let mut ordered: Vec<String> = vec![String::new(); palette_map.len()];
    for (name, idx) in &palette_map {
        if *idx >= ordered.len() {
            ordered.resize(idx + 1, String::new());
        }
        ordered[*idx] = name.clone();
    }
    let ids = read_varints(&block_bytes, volume)?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for id in ids {
        let name = ordered
            .get(id as usize)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| BlockState::air().to_compact());
        let name = normalize_block_name(&name);
        *counts.entry(name).or_insert(0) += 1;
    }
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs.truncate(top_n);
    Ok(pairs)
}

fn normalize_block_name(s: &str) -> String {
    BlockState::parse(s)
        .map(|b| b.to_compact())
        .unwrap_or_else(|_| {
            if s.contains(':') {
                s.to_string()
            } else {
                format!("minecraft:{s}")
            }
        })
}

fn palette_to_index_map(palette: &Value) -> Result<HashMap<String, usize>> {
    match palette {
        Value::Compound(map) => {
            let mut out = HashMap::new();
            for (k, v) in map {
                let idx = match v {
                    Value::Int(i) => *i as usize,
                    Value::Short(i) => *i as usize,
                    Value::Byte(i) => *i as usize,
                    Value::Long(i) => *i as usize,
                    _ => continue,
                };
                out.insert(k.clone(), idx);
            }
            Ok(out)
        }
        Value::List(list) => {
            let mut out = HashMap::new();
            for (i, v) in list.iter().enumerate() {
                if let Value::String(s) = v {
                    out.insert(s.clone(), i);
                }
            }
            Ok(out)
        }
        _ => Err(Error::msg("unsupported Palette type")),
    }
}

fn find_palette(root: &Value) -> Option<&Value> {
    compound_child(root, "Palette").or_else(|| compound_child(root, "palette"))
}

fn read_offset(root: &Value) -> [i32; 3] {
    match compound_child(root, "Offset") {
        Some(Value::IntArray(arr)) => {
            let x = arr.first().copied().unwrap_or(0);
            let y = arr.get(1).copied().unwrap_or(0);
            let z = arr.get(2).copied().unwrap_or(0);
            [x, y, z]
        }
        Some(Value::List(list)) => {
            let x = list.first().and_then(num_i32).unwrap_or(0);
            let y = list.get(1).and_then(num_i32).unwrap_or(0);
            let z = list.get(2).and_then(num_i32).unwrap_or(0);
            [x, y, z]
        }
        _ => [0, 0, 0],
    }
}

fn list_len(root: &Value, key: &str) -> usize {
    match compound_child(root, key) {
        Some(Value::List(v)) => v.len(),
        _ => 0,
    }
}

fn block_entities_len(root: &Value, version: i32) -> usize {
    if version >= 3 {
        if let Some(blocks) = compound_child(root, "Blocks") {
            let n = list_len(blocks, "BlockEntities");
            if n > 0 {
                return n;
            }
            return list_len(blocks, "TileEntities");
        }
    }
    let n = list_len(root, "BlockEntities");
    if n > 0 {
        return n;
    }
    list_len(root, "TileEntities") // v1 name
}

fn compound_child<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    match root {
        Value::Compound(m) => m.get(key),
        _ => None,
    }
}

fn compound_i32(root: &Value, key: &str) -> Option<i32> {
    match compound_child(root, key)? {
        Value::Int(v) => Some(*v),
        Value::Short(v) => Some(*v as i32),
        Value::Byte(v) => Some(*v as i32),
        Value::Long(v) => Some(*v as i32),
        _ => None,
    }
}

/// Sponge Width/Height/Length are unsigned shorts stored as NBT Short.
fn compound_u16(root: &Value, key: &str) -> Option<u16> {
    match compound_child(root, key)? {
        Value::Short(v) => Some(*v as u16),
        Value::Int(v) => Some(*v as u16),
        Value::Byte(v) => Some(*v as u16),
        Value::Long(v) => Some(*v as u16),
        _ => None,
    }
}

fn compound_bytes(root: &Value, key: &str) -> Option<Vec<u8>> {
    match compound_child(root, key)? {
        Value::ByteArray(arr) => Some(arr.iter().map(|b| *b as u8).collect()),
        Value::List(list) => {
            let mut out = Vec::with_capacity(list.len());
            for v in list {
                match v {
                    Value::Byte(b) => out.push(*b as u8),
                    Value::Int(i) => out.push(*i as u8),
                    _ => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

fn entity_pos(map: &HashMap<String, Value>) -> (f64, f64, f64) {
    if let Some(Value::List(pos)) = map.get("Pos") {
        let x = num_f64(pos.first()).unwrap_or(0.0);
        let y = num_f64(pos.get(1)).unwrap_or(0.0);
        let z = num_f64(pos.get(2)).unwrap_or(0.0);
        return (x, y, z);
    }
    (0.0, 0.0, 0.0)
}

fn num_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Double(d) => Some(*d),
        Value::Float(f) => Some(*f as f64),
        Value::Int(i) => Some(*i as f64),
        Value::Long(i) => Some(*i as f64),
        _ => None,
    }
}

fn num_i32(v: &Value) -> Option<i32> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Short(i) => Some(*i as i32),
        Value::Byte(i) => Some(*i as i32),
        Value::Long(i) => Some(*i as i32),
        _ => None,
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varints(data: &[u8], max: usize) -> Result<Vec<u32>> {
    let mut out = Vec::with_capacity(max);
    let mut i = 0;
    while i < data.len() && out.len() < max {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            if i >= data.len() {
                return Err(Error::msg("truncated varint in schem BlockData"));
            }
            let byte = data[i];
            i += 1;
            value |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 35 {
                return Err(Error::msg("varint too long"));
            }
        }
        out.push(value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use tempfile::TempDir;

    fn write_gzip_nbt(path: &Path, root: Value) {
        let bytes = fastnbt::to_bytes(&root).unwrap();
        let file = fs::File::create(path).unwrap();
        let mut enc = GzEncoder::new(file, Compression::default());
        enc.write_all(&bytes).unwrap();
        enc.finish().unwrap();
    }

    fn minimal_v2_flat(offset: [i32; 3]) -> Value {
        let mut palette = HashMap::new();
        palette.insert("minecraft:air".into(), Value::Int(0));
        palette.insert("minecraft:stone".into(), Value::Int(1));
        // 2x1x1 volume: stone, air
        let mut data = Vec::new();
        write_varint(&mut data, 1);
        write_varint(&mut data, 0);
        let mut root = HashMap::new();
        root.insert("Version".into(), Value::Int(2));
        root.insert("DataVersion".into(), Value::Int(3465));
        root.insert("Width".into(), Value::Short(2));
        root.insert("Height".into(), Value::Short(1));
        root.insert("Length".into(), Value::Short(1));
        root.insert(
            "Offset".into(),
            Value::IntArray(fastnbt::IntArray::new(offset.to_vec())),
        );
        root.insert(
            "Palette".into(),
            Value::Compound(palette.into_iter().collect()),
        );
        root.insert(
            "BlockData".into(),
            Value::ByteArray(fastnbt::ByteArray::new(
                data.into_iter().map(|b| b as i8).collect(),
            )),
        );
        Value::Compound(root.into_iter().collect())
    }

    fn wrap_schematic(inner: Value) -> Value {
        let mut root = HashMap::new();
        root.insert("Schematic".into(), inner);
        Value::Compound(root.into_iter().collect())
    }

    #[test]
    fn roundtrip_schem_v2() {
        let tmp = TempDir::new().unwrap();
        let world = tmp.path().join("world");
        fs::create_dir_all(world.join("region")).unwrap();
        fs::create_dir_all(world.join("entities")).unwrap();
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("s".into()), None).unwrap();
        let mut session = Session::open(&cwd, "s").unwrap();
        let mut wv = WorldView::new(&mut session);
        wv.fill(
            0,
            64,
            0,
            2,
            65,
            1,
            BlockState::parse("minecraft:stone").unwrap(),
        )
        .unwrap();
        let out = tmp.path().join("t.schem");
        export_aabb(&wv, 0, 64, 0, 2, 65, 1, &out).unwrap();
        let meta = info(&out).unwrap();
        assert_eq!(meta.version, 2);
        assert!(meta.data_version.is_some());
        assert_eq!(meta.offset, [0, 0, 0]);
        assert!(meta.volume > 0);
        assert!(!meta.top_blocks.is_empty());
        let tpl = import_to_template(&out, "t").unwrap();
        assert_eq!(tpl.size, [3, 2, 2]);
        assert!(tpl.palette.iter().any(|p| p.contains("stone")));
    }

    #[test]
    fn parse_we_wrapped_schematic() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("wrapped.schem");
        write_gzip_nbt(&path, wrap_schematic(minimal_v2_flat([1, -2, 3])));
        let meta = info(&path).unwrap();
        assert_eq!(meta.version, 2);
        assert_eq!(meta.data_version, Some(3465));
        assert_eq!(meta.mc.as_deref(), Some("1.20.1"));
        assert_eq!(meta.width, 2);
        assert_eq!(meta.height, 1);
        assert_eq!(meta.length, 1);
        assert_eq!(meta.offset, [1, -2, 3]);
        assert_eq!(meta.volume, 2);
        assert_eq!(meta.palette_n, 2);
        assert!(meta
            .top_blocks
            .iter()
            .any(|(n, c)| n.contains("stone") && *c == 1));
        let tpl = import_to_template(&path, "w").unwrap();
        assert_eq!(tpl.size, [2, 1, 1]);
    }

    #[test]
    fn parse_v3_blocks_compound() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v3.schem");
        let mut palette = HashMap::new();
        palette.insert("minecraft:dirt".into(), Value::Int(0));
        let mut data = Vec::new();
        write_varint(&mut data, 0);
        let mut blocks = HashMap::new();
        blocks.insert(
            "Palette".into(),
            Value::Compound(palette.into_iter().collect()),
        );
        blocks.insert(
            "Data".into(),
            Value::ByteArray(fastnbt::ByteArray::new(
                data.into_iter().map(|b| b as i8).collect(),
            )),
        );
        blocks.insert("BlockEntities".into(), Value::List(Vec::new()));
        let mut schem = HashMap::new();
        schem.insert("Version".into(), Value::Int(3));
        schem.insert("DataVersion".into(), Value::Int(4903));
        schem.insert("Width".into(), Value::Short(1));
        schem.insert("Height".into(), Value::Short(1));
        schem.insert("Length".into(), Value::Short(1));
        schem.insert(
            "Offset".into(),
            Value::IntArray(fastnbt::IntArray::new(vec![0, 0, 0])),
        );
        schem.insert(
            "Blocks".into(),
            Value::Compound(blocks.into_iter().collect()),
        );
        write_gzip_nbt(&path, wrap_schematic(Value::Compound(schem.into_iter().collect())));
        let meta = info(&path).unwrap();
        assert_eq!(meta.version, 3);
        assert_eq!(meta.data_version, Some(4903));
        assert_eq!(meta.mc.as_deref(), Some("26.2"));
        assert_eq!(meta.volume, 1);
        let tpl = import_to_template(&path, "v3").unwrap();
        assert!(tpl.palette.iter().any(|p| p.contains("dirt")));
    }

    #[test]
    fn paste_applies_offset() {
        let tmp = TempDir::new().unwrap();
        let world = tmp.path().join("world");
        fs::create_dir_all(world.join("region")).unwrap();
        fs::create_dir_all(world.join("entities")).unwrap();
        let cwd = tmp.path().to_path_buf();
        Session::create(&cwd, &world, "overworld", Some("s".into()), None).unwrap();
        let path = tmp.path().join("off.schem");
        write_gzip_nbt(&path, wrap_schematic(minimal_v2_flat([4, 0, 0])));

        let mut session = Session::open(&cwd, "s").unwrap();
        let mut wv = WorldView::new(&mut session);
        import_paste(&mut wv, &path, 10, 64, 10).unwrap();
        // at(10,64,10) + offset(4,0,0) → stone at x=14
        assert_eq!(
            wv.get_block(14, 64, 10).unwrap().to_compact(),
            "minecraft:stone"
        );
    }

    #[test]
    fn reject_classic_mcedit_schematic() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("old.schematic");
        let mut root = HashMap::new();
        root.insert("Materials".into(), Value::String("Alpha".into()));
        root.insert("Width".into(), Value::Short(1));
        root.insert("Height".into(), Value::Short(1));
        root.insert("Length".into(), Value::Short(1));
        root.insert(
            "Blocks".into(),
            Value::ByteArray(fastnbt::ByteArray::new(vec![1])),
        );
        root.insert(
            "Data".into(),
            Value::ByteArray(fastnbt::ByteArray::new(vec![0])),
        );
        write_gzip_nbt(&path, Value::Compound(root.into_iter().collect()));
        let err = info(&path).unwrap_err().to_string();
        assert!(err.contains("classic MCEdit"), "{err}");
    }
}
