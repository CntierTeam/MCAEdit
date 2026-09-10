//! Vanilla Minecraft structure template (`.nbt`) import / export / place.
//!
//! Distinct from Sponge `.schem`. Structure files live under
//! `generated/<ns>/structures/` or datapack `data/<ns>/structure/`.
//! Placement reuses the Template paste pipeline (history / undo).

use crate::action::Action;
use crate::blockstate::BlockState;
use crate::chunk_nbt::{json_to_nbt, nbt_to_json};
use crate::error::{Error, Result};
use crate::mc_version::DEFAULT_DATA_VERSION;
use crate::template::{Template, TemplateEntity};
use crate::world::WorldView;
use fastnbt::Value;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indexmap::IndexSet;
use serde_json::{json, Map, Value as JsonValue};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StructureInfo {
    pub path: String,
    pub size: [i32; 3],
    pub blocks: usize,
    pub entities: usize,
    pub palette: usize,
    pub data_version: Option<i32>,
    pub author: Option<String>,
}

impl StructureInfo {
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("structure={}", self.path),
            format!("size={}x{}x{}", self.size[0], self.size[1], self.size[2]),
            format!("blocks={}", self.blocks),
            format!("entities={}", self.entities),
            format!("palette={}", self.palette),
            format!(
                "DataVersion={}",
                self.data_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!("author={}", self.author.as_deref().unwrap_or("-")),
        ]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceOptions {
    /// Yaw degrees: 0 / 90 / 180 / 270 (clockwise looking down).
    pub rotation: i32,
    /// Mirror axis: None / Some('x') / Some('z') (vanilla LEFT_RIGHT≈z, FRONT_BACK≈x).
    pub mirror: Option<char>,
    pub include_entities: bool,
}

/// Read structure NBT metadata.
pub fn info(path: &Path) -> Result<StructureInfo> {
    let root = read_structure_root(path)?;
    let root_j = nbt_to_json(root)?;
    let size = size_from_json(&root_j)?;
    let blocks = root_j
        .get("blocks")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let entities = root_j
        .get("entities")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let palette = palette_len(&root_j);
    Ok(StructureInfo {
        path: path.display().to_string(),
        size,
        blocks,
        entities,
        palette,
        data_version: root_j
            .get("DataVersion")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        author: root_j
            .get("author")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// List `.nbt` files under common world / datapack structure directories.
pub fn list_in_world(world: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let candidates = [
        world.join("generated"),
        world.join("datapacks"),
        world.join("structures"),
    ];
    for root in candidates {
        if root.exists() {
            walk_nbt(&root, &mut out)?;
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk_nbt(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_nbt(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("nbt"))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Import structure → Template (dense AABB; missing positions = air).
pub fn import_to_template(path: &Path, name: &str) -> Result<Template> {
    let root = read_structure_root(path)?;
    let root_j = nbt_to_json(root)?;
    template_from_structure(&root_j, name)
}

/// Export AABB as vanilla structure `.nbt`.
pub fn export_aabb(
    world: &WorldView<'_>,
    from: [i32; 3],
    to: [i32; 3],
    out: &Path,
    data_version: i32,
) -> Result<StructureInfo> {
    let [x1, y1, z1] = from;
    let [x2, y2, z2] = to;
    let tpl = Template::capture(world, "structure", x1, y1, z1, x2, y2, z2)?;
    write_template_structure(&tpl, out, data_version)?;
    info(out)
}

/// Paste structure at origin (optional rotate / mirror). History via template paste.
pub fn place(
    world: &mut WorldView<'_>,
    path: &Path,
    origin_x: i32,
    origin_y: i32,
    origin_z: i32,
    opts: PlaceOptions,
) -> Result<Action> {
    let mut tpl = import_to_template(path, "structure-place")?;
    if !opts.include_entities {
        tpl.entities.clear();
    }
    if let Some(axis) = opts.mirror {
        tpl.flip(axis)?;
    }
    if opts.rotation != 0 {
        tpl.rotate_yaw(opts.rotation)?;
    }
    let action = tpl.paste_into(world, origin_x, origin_y, origin_z)?;
    let desc = format!(
        "structure place {} @ {},{},{} rot={} mirror={}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("structure.nbt"),
        origin_x,
        origin_y,
        origin_z,
        opts.rotation,
        opts.mirror
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".into())
    );
    if let Some(mut cur) = world.session.history.current()? {
        cur.description = desc;
        world.session.history.replace_current(cur.clone())?;
        Ok(cur)
    } else {
        Ok(action)
    }
}

fn template_from_structure(root: &JsonValue, name: &str) -> Result<Template> {
    let size = size_from_json(root)?;
    let [sx, sy, sz] = size;
    if sx <= 0 || sy <= 0 || sz <= 0 {
        return Err(Error::msg("structure size must be positive"));
    }
    if (sx as i64) * (sy as i64) * (sz as i64) > 64 * 64 * 64 {
        return Err(Error::msg("structure too large (max 64^3 dense volume)"));
    }

    let palette = load_palette(root)?;
    let mut palette_set: IndexSet<String> = IndexSet::new();
    palette_set.insert(BlockState::air().to_compact());
    let mut blocks = vec![0u16; (sx * sy * sz) as usize];

    let block_list = root
        .get("blocks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::msg("structure missing blocks"))?;

    for b in block_list {
        let state_i = b
            .get("state")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::msg("block missing state"))? as usize;
        let pos = b
            .get("pos")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::msg("block missing pos"))?;
        if pos.len() != 3 {
            return Err(Error::msg("block pos must be [x,y,z]"));
        }
        let x = pos[0].as_i64().unwrap_or(0) as i32;
        let y = pos[1].as_i64().unwrap_or(0) as i32;
        let z = pos[2].as_i64().unwrap_or(0) as i32;
        if x < 0 || y < 0 || z < 0 || x >= sx || y >= sy || z >= sz {
            continue;
        }
        let compact = palette
            .get(state_i)
            .ok_or_else(|| Error::msg(format!("palette index {state_i} out of range")))?;
        let (id, _) = palette_set.insert_full(compact.clone());
        let idx = ((y * sz + z) * sx + x) as usize;
        blocks[idx] = id as u16;
    }

    let mut entities = Vec::new();
    if let Some(ents) = root.get("entities").and_then(|v| v.as_array()) {
        for ent in ents {
            let nbt = ent.get("nbt").cloned().unwrap_or_else(|| json!({}));
            let pos = ent
                .get("pos")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_else(|| vec![json!(0.0), json!(0.0), json!(0.0)]);
            let dx = pos.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
            let dy = pos.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let dz = pos.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            entities.push(TemplateEntity { dx, dy, dz, nbt });
        }
    }

    Ok(Template {
        name: name.to_string(),
        size: [sx as u32, sy as u32, sz as u32],
        palette: palette_set.into_iter().collect(),
        blocks,
        entities,
    })
}

fn write_template_structure(tpl: &Template, out: &Path, data_version: i32) -> Result<()> {
    let [sx, sy, sz] = tpl.size;
    let mut palette_json = Vec::new();
    for name in &tpl.palette {
        palette_json.push(blockstate_to_palette_entry(name)?);
    }

    let mut blocks = Vec::new();
    let mut i = 0usize;
    for y in 0..sy as i32 {
        for z in 0..sz as i32 {
            for x in 0..sx as i32 {
                let id = *tpl
                    .blocks
                    .get(i)
                    .ok_or_else(|| Error::msg("template blocks truncated"))?;
                i += 1;
                let name = tpl
                    .palette
                    .get(id as usize)
                    .ok_or_else(|| Error::msg("bad palette id"))?;
                if BlockState::parse(name).map(|b| b.is_air_like()).unwrap_or(false) {
                    continue; // vanilla structures omit pure air
                }
                blocks.push(json!({
                    "state": id as i32,
                    "pos": [x, y, z]
                }));
            }
        }
    }

    let mut entities = Vec::new();
    for ent in &tpl.entities {
        entities.push(json!({
            "pos": [ent.dx, ent.dy, ent.dz],
            "blockPos": [ent.dx.floor() as i32, ent.dy.floor() as i32, ent.dz.floor() as i32],
            "nbt": ent.nbt
        }));
    }

    let root = json!({
        "size": [sx as i32, sy as i32, sz as i32],
        "palette": palette_json,
        "blocks": blocks,
        "entities": entities,
        "DataVersion": data_version
    });

    let nbt = json_to_nbt(&root)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fastnbt::to_bytes(&nbt)?;
    let file = fs::File::create(out)?;
    let mut enc = GzEncoder::new(file, Compression::default());
    enc.write_all(&bytes)?;
    enc.finish()?;
    Ok(())
}

fn blockstate_to_palette_entry(compact: &str) -> Result<JsonValue> {
    let bs = BlockState::parse(compact).map_err(Error::msg)?;
    let mut obj = Map::new();
    obj.insert("Name".into(), json!(bs.name));
    if !bs.properties.is_empty() {
        let mut props = Map::new();
        for (k, v) in &bs.properties {
            props.insert(k.clone(), json!(v));
        }
        obj.insert("Properties".into(), JsonValue::Object(props));
    }
    Ok(JsonValue::Object(obj))
}

fn load_palette(root: &JsonValue) -> Result<Vec<String>> {
    if let Some(arr) = root.get("palette").and_then(|v| v.as_array()) {
        return arr.iter().map(palette_entry_to_compact).collect();
    }
    // Multi-palette (shipwrecks): use first.
    if let Some(palettes) = root.get("palettes").and_then(|v| v.as_array()) {
        if let Some(first) = palettes.first().and_then(|v| v.as_array()) {
            return first.iter().map(palette_entry_to_compact).collect();
        }
    }
    Err(Error::msg("structure missing palette/palettes"))
}

fn palette_entry_to_compact(v: &JsonValue) -> Result<String> {
    let name = v
        .get("Name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| Error::msg("palette entry missing Name"))?;
    let mut bs = BlockState::parse(name).map_err(Error::msg)?;
    if let Some(props) = v.get("Properties").and_then(|p| p.as_object()) {
        for (k, val) in props {
            if let Some(s) = val.as_str() {
                bs.properties.insert(k.clone(), s.to_string());
            }
        }
    }
    Ok(bs.to_compact())
}

fn palette_len(root: &JsonValue) -> usize {
    if let Some(arr) = root.get("palette").and_then(|v| v.as_array()) {
        return arr.len();
    }
    root.get("palettes")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn size_from_json(root: &JsonValue) -> Result<[i32; 3]> {
    let size = root
        .get("size")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::msg("structure missing size"))?;
    if size.len() != 3 {
        return Err(Error::msg("structure size must have 3 ints"));
    }
    Ok([
        size[0].as_i64().unwrap_or(0) as i32,
        size[1].as_i64().unwrap_or(0) as i32,
        size[2].as_i64().unwrap_or(0) as i32,
    ])
}

fn read_structure_root(path: &Path) -> Result<Value> {
    let raw = fs::read(path)?;
    let mut buf = Vec::new();
    let bytes = match GzDecoder::new(&raw[..]).read_to_end(&mut buf) {
        Ok(_) if !buf.is_empty() => buf,
        _ => raw,
    };
    Ok(fastnbt::from_bytes(&bytes)?)
}

/// Default DataVersion stamped into newly exported structures.
pub fn default_structure_data_version() -> i32 {
    DEFAULT_DATA_VERSION
}

// ── Chunk structure starts / references ───────────────────────────────────

/// Summarize `structures` / `Structures` compound on a chunk.
pub fn chunk_structure_summary(chunk: &crate::chunk::ChunkData) -> Vec<String> {
    let Some(st) = chunk
        .root
        .get("structures")
        .or_else(|| chunk.root.get("Structures"))
    else {
        return vec!["structures=(none)".into()];
    };
    let mut lines = Vec::new();
    if let Some(starts) = st.get("starts").and_then(|v| v.as_object()) {
        lines.push(format!("starts={}", starts.len()));
        for (k, v) in starts {
            let id = v
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("-");
            lines.push(format!("  start {k} id={id}"));
        }
    }
    if let Some(refs) = st
        .get("References")
        .or_else(|| st.get("references"))
        .and_then(|v| v.as_object())
    {
        lines.push(format!("References={}", refs.len()));
        for (k, v) in refs {
            let n = v.as_array().map(|a| a.len()).unwrap_or(0);
            lines.push(format!("  ref {k} chunks={n}"));
        }
    }
    if lines.is_empty() {
        lines.push("structures=(empty)".into());
    }
    lines
}

/// Clear structure starts and/or references intersecting an AABB (chunk-level).
pub fn clear_structures_in_aabb(
    world: &mut WorldView<'_>,
    from: [i32; 3],
    to: [i32; 3],
    clear_starts: bool,
    clear_refs: bool,
) -> Result<usize> {
    let [x1, y1, z1] = from;
    let [x2, y2, z2] = to;
    let (min_x, max_x) = (x1.min(x2), x1.max(x2));
    let (min_z, max_z) = (z1.min(z2), z1.max(z2));
    let _ = (y1, y2); // structure refs are XZ/chunk keyed
    let cx0 = min_x >> 4;
    let cx1 = max_x >> 4;
    let cz0 = min_z >> 4;
    let cz1 = max_z >> 4;
    let mut touched = 0usize;
    for cx in cx0..=cx1 {
        for cz in cz0..=cz1 {
            let mut chunk = world.load_chunk(cx, cz)?;
            let key = if chunk.root.get("structures").is_some() {
                "structures"
            } else if chunk.root.get("Structures").is_some() {
                "Structures"
            } else {
                continue;
            };
            let Some(st) = chunk.root.get_mut(key).and_then(|v| v.as_object_mut()) else {
                continue;
            };
            let mut changed = false;
            if clear_starts && st.remove("starts").is_some() {
                st.insert("starts".into(), json!({}));
                changed = true;
            }
            if clear_refs
                && (st.remove("References").is_some() || st.remove("references").is_some())
            {
                st.insert("References".into(), json!({}));
                changed = true;
            }
            if changed {
                world.save_chunk(&chunk)?;
                touched += 1;
            }
        }
    }
    if touched > 0 {
        world.session.mark_dirty()?;
    }
    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use tempfile::TempDir;

    fn make_session(tmp: &TempDir) -> (std::path::PathBuf, Session) {
        let world = tmp.path().join("world");
        fs::create_dir_all(world.join("region")).unwrap();
        fs::create_dir_all(world.join("entities")).unwrap();
        let cwd = tmp.path().to_path_buf();
        let s = Session::create(&cwd, &world, "overworld", Some("s1".into()), None).unwrap();
        (cwd, s)
    }

    #[test]
    fn structure_export_import_place_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let (cwd, mut session) = make_session(&tmp);
        {
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
            let out = tmp.path().join("hut.nbt");
            export_aabb(
                &wv,
                [0, 64, 0],
                [2, 65, 1],
                &out,
                DEFAULT_DATA_VERSION,
            )
            .unwrap();
            let meta = info(&out).unwrap();
            assert_eq!(meta.size, [3, 2, 2]);
            assert!(meta.blocks >= 1);

            place(
                &mut wv,
                &out,
                32,
                64,
                32,
                PlaceOptions {
                    rotation: 90,
                    mirror: None,
                    include_entities: true,
                },
            )
            .unwrap();
            assert_eq!(
                wv.get_block(32, 64, 32).unwrap().to_compact(),
                "minecraft:stone"
            );
        }
        let _ = cwd;
    }
}
