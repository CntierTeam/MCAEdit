//! Sponge Schematic (.schem) v2 write / v2+v3 read.

use crate::blockstate::BlockState;
use crate::error::{Error, Result};
use crate::template::{Template, TemplateEntity};
use crate::world::WorldView;
use fastnbt::Value;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indexmap::IndexSet;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// Export AABB from world to Sponge Schematic v2 (.schem, gzip NBT).
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
    write_template_v2(&tpl, out)?;
    Ok(SchemInfo {
        version: 2,
        width: tpl.size[0] as u16,
        height: tpl.size[1] as u16,
        length: tpl.size[2] as u16,
        palette_n: tpl.palette.len(),
        path: out.display().to_string(),
    })
}

pub fn export_template(tpl: &Template, out: &Path) -> Result<SchemInfo> {
    write_template_v2(tpl, out)?;
    Ok(SchemInfo {
        version: 2,
        width: tpl.size[0] as u16,
        height: tpl.size[1] as u16,
        length: tpl.size[2] as u16,
        palette_n: tpl.palette.len(),
        path: out.display().to_string(),
    })
}

/// Import .schem into a Template (blocks + relative entities when present).
pub fn import_to_template(path: &Path, name: &str) -> Result<Template> {
    let root = read_schem_root(path)?;
    template_from_schem(&root, name)
}

/// Paste .schem at origin into the session world (history via template paste).
pub fn import_paste(
    world: &mut WorldView<'_>,
    path: &Path,
    ox: i32,
    oy: i32,
    oz: i32,
) -> Result<crate::action::Action> {
    let tpl = import_to_template(path, "schem-import")?;
    tpl.paste_into(world, ox, oy, oz)
}

#[derive(Clone, Debug)]
pub struct SchemInfo {
    pub version: i32,
    pub width: u16,
    pub height: u16,
    pub length: u16,
    pub palette_n: usize,
    pub path: String,
}

impl SchemInfo {
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("schem={}", self.path),
            format!("version={}", self.version),
            format!(
                "size={}x{}x{}",
                self.width, self.height, self.length
            ),
            format!("palette_n={}", self.palette_n),
        ]
    }
}

pub fn info(path: &Path) -> Result<SchemInfo> {
    let root = read_schem_root(path)?;
    let version = compound_i32(&root, "Version").unwrap_or(2);
    let (w, h, l, palette_n) = dims_from_root(&root)?;
    Ok(SchemInfo {
        version,
        width: w,
        height: h,
        length: l,
        palette_n,
        path: path.display().to_string(),
    })
}

fn write_template_v2(tpl: &Template, out: &Path) -> Result<()> {
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

    let mut root = HashMap::new();
    root.insert("Version".into(), Value::Int(2));
    root.insert("DataVersion".into(), Value::Int(3465));
    root.insert("Width".into(), Value::Short(w as i16));
    root.insert("Height".into(), Value::Short(h as i16));
    root.insert("Length".into(), Value::Short(l as i16));
    root.insert(
        "Offset".into(),
        Value::IntArray(fastnbt::IntArray::new(vec![0, 0, 0])),
    );
    root.insert("Palette".into(), Value::Compound(palette.into_iter().collect()));
    root.insert("PaletteMax".into(), Value::Int(tpl.palette.len() as i32));
    root.insert(
        "BlockData".into(),
        Value::ByteArray(fastnbt::ByteArray::new(
            block_data.into_iter().map(|b| b as i8).collect(),
        )),
    );
    root.insert("BlockEntities".into(), Value::List(Vec::new()));

    if !tpl.entities.is_empty() {
        let mut ents = Vec::new();
        for ent in &tpl.entities {
            let mut map = HashMap::new();
            // Relative Pos inside schematic
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
            // Keep full NBT under Data when possible
            if let Ok(nbt) = crate::chunk_nbt::json_to_nbt(&ent.nbt) {
                map.insert("Data".into(), nbt);
            }
            ents.push(Value::Compound(map.into_iter().collect()));
        }
        root.insert("Entities".into(), Value::List(ents));
    }

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
    Ok(value)
}

fn dims_from_root(root: &Value) -> Result<(u16, u16, u16, usize)> {
    let w = compound_i32(root, "Width").ok_or_else(|| Error::msg("schem missing Width"))? as u16;
    let h = compound_i32(root, "Height").ok_or_else(|| Error::msg("schem missing Height"))? as u16;
    let l = compound_i32(root, "Length").ok_or_else(|| Error::msg("schem missing Length"))? as u16;
    let palette_n = match find_palette(root) {
        Some(Value::Compound(m)) => m.len(),
        Some(Value::List(v)) => v.len(),
        _ => 0,
    };
    Ok((w, h, l, palette_n))
}

fn template_from_schem(root: &Value, name: &str) -> Result<Template> {
    let version = compound_i32(root, "Version").unwrap_or(2);
    let (w, h, l, _) = dims_from_root(root)?;
    let (palette_map, block_bytes) = if version >= 3 {
        // v3: Blocks compound
        let blocks = compound_child(root, "Blocks").ok_or_else(|| Error::msg("v3 schem missing Blocks"))?;
        let palette = find_palette(blocks).ok_or_else(|| Error::msg("v3 missing Palette"))?;
        let data = compound_bytes(blocks, "Data")
            .or_else(|| compound_bytes(blocks, "BlockData"))
            .ok_or_else(|| Error::msg("v3 missing Block Data"))?;
        (palette_to_index_map(palette)?, data)
    } else {
        let palette = find_palette(root).ok_or_else(|| Error::msg("schem missing Palette"))?;
        let data = compound_bytes(root, "BlockData")
            .ok_or_else(|| Error::msg("schem missing BlockData"))?;
        (palette_to_index_map(palette)?, data)
    };

    let mut ordered: Vec<String> = vec![String::new(); palette_map.len()];
    for (name, idx) in &palette_map {
        if *idx >= ordered.len() {
            ordered.resize(idx + 1, String::new());
        }
        ordered[*idx] = name.clone();
    }
    for (i, slot) in ordered.iter_mut().enumerate() {
        if slot.is_empty() {
            *slot = BlockState::air().to_compact();
            let _ = i;
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

    // Remap to dense palette for Template
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
            // Rare: list of names in order
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
        let tpl = import_to_template(&out, "t").unwrap();
        assert_eq!(tpl.size, [3, 2, 2]);
        assert!(tpl.palette.iter().any(|p| p.contains("stone")));
    }
}
