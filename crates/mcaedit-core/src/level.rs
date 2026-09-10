//! Create / read / write `level.dat` and empty world skeletons.
//!
//! Supports classic Anvil `level.dat` (WorldGenSettings inside `Data`, 1.18–~1.21.8)
//! and modern layout (seed / gen settings also mirrored under `data/minecraft/`, 1.21.9+ / 26.2).

use crate::chunk_nbt::{json_to_nbt, nbt_to_json};
use crate::error::{Error, Result};
use crate::mc_version::{
    ResolvedVersion, ANVIL_LEVEL_VERSION, DEFAULT_DATA_VERSION, MODERN_LEVEL_DAT_MIN,
};
use fastnbt::Value;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value as JsonValue};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorKind {
    Noise,
    Flat,
}

impl GeneratorKind {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "noise" | "default" | "normal" => Ok(Self::Noise),
            "flat" | "superflat" => Ok(Self::Flat),
            other => Err(Error::msg(format!(
                "unknown generator `{other}` (noise|flat)"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionFormat {
    Anvil,
    Linear,
}

impl RegionFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anvil" | "mca" => Ok(Self::Anvil),
            "linear" => Ok(Self::Linear),
            other => Err(Error::msg(format!(
                "unknown region format `{other}` (anvil|linear)"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorldCreateOptions {
    pub path: PathBuf,
    pub level_name: String,
    pub seed: i64,
    pub spawn: [i32; 3],
    pub game_type: i32,
    pub generator: GeneratorKind,
    pub version: ResolvedVersion,
    pub region_format: RegionFormat,
    /// Create DIM-1 / DIM1 region dirs as well.
    pub all_dims: bool,
    /// Overwrite existing level.dat if present.
    pub force: bool,
}

impl Default for WorldCreateOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::from("world"),
            level_name: "world".into(),
            seed: 0,
            spawn: [0, 64, 0],
            game_type: 1, // creative — friendlier for offline edit
            generator: GeneratorKind::Noise,
            version: ResolvedVersion::default_latest(),
            region_format: RegionFormat::Anvil,
            all_dims: false,
            force: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LevelInfo {
    pub path: PathBuf,
    pub level_name: String,
    pub seed: Option<i64>,
    pub spawn: [i32; 3],
    pub game_type: Option<i32>,
    pub data_version: Option<i32>,
    pub version_name: Option<String>,
    pub last_played: Option<i64>,
    pub generator: Option<String>,
    pub modern_layout: bool,
}

impl LevelInfo {
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("level.dat={}", self.path.display()),
            format!("LevelName={}", self.level_name),
            format!(
                "Seed={}",
                self.seed
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!(
                "Spawn={},{},{}",
                self.spawn[0], self.spawn[1], self.spawn[2]
            ),
            format!(
                "GameType={}",
                self.game_type
                    .map(|g| g.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!(
                "DataVersion={}",
                self.data_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!(
                "Version.Name={}",
                self.version_name.as_deref().unwrap_or("-")
            ),
            format!(
                "LastPlayed={}",
                self.last_played
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!(
                "generator={}",
                self.generator.as_deref().unwrap_or("-")
            ),
            format!("modern_layout={}", self.modern_layout),
        ]
    }
}

/// Create an empty world directory tree + `level.dat` (+ modern sidecars when needed).
pub fn create_world(opts: &WorldCreateOptions) -> Result<LevelInfo> {
    let world = &opts.path;
    if world.exists() {
        let level = world.join("level.dat");
        let region = world.join("region");
        if level.exists() && !opts.force {
            return Err(Error::msg(format!(
                "world already has level.dat at {}; pass --force to overwrite",
                level.display()
            )));
        }
        if !region.exists() {
            fs::create_dir_all(&region)?;
        }
    } else {
        fs::create_dir_all(world)?;
    }

    ensure_dim_dirs(world, ".", opts.region_format)?;
    if opts.all_dims {
        ensure_dim_dirs(world, "DIM-1", opts.region_format)?;
        ensure_dim_dirs(world, "DIM1", opts.region_format)?;
    }

    write_level_dat(world, opts)?;
    if opts.version.modern_layout {
        write_modern_sidecars(world, opts)?;
    }

    info(world)
}

/// Ensure overworld (or named dim) region/entities/poi dirs exist without touching level.dat.
pub fn ensure_world_dirs(world: &Path, dim: &str, format: RegionFormat) -> Result<()> {
    let rel = match dim {
        "overworld" | "." | "" => ".",
        "nether" | "DIM-1" => "DIM-1",
        "end" | "DIM1" => "DIM1",
        other => other,
    };
    ensure_dim_dirs(world, rel, format)
}

/// Ensure region (+ entities) dirs exist; for Linear, leave empty (first write creates files).
pub fn ensure_dim_dirs(world: &Path, rel: &str, format: RegionFormat) -> Result<()> {
    let base = if rel == "." {
        world.to_path_buf()
    } else {
        world.join(rel)
    };
    fs::create_dir_all(base.join("region"))?;
    fs::create_dir_all(base.join("entities"))?;
    fs::create_dir_all(base.join("poi"))?;
    let _ = format; // Linear uses same dirs; files appear on commit/edit
    Ok(())
}

/// Write / update `level.dat` only (keeps other world files).
pub fn write_level_dat(world: &Path, opts: &WorldCreateOptions) -> Result<()> {
    let data = build_data_compound(opts);
    let mut root = Map::new();
    root.insert("Data".into(), JsonValue::Object(data));
    let nbt = json_to_nbt(&JsonValue::Object(root))?;
    write_gzip_nbt(world.join("level.dat"), &nbt)?;
    // Backup mirror for tools that look for level.dat_old after first save.
    if !world.join("level.dat_old").exists() {
        let _ = fs::copy(world.join("level.dat"), world.join("level.dat_old"));
    }
    if opts.version.modern_layout {
        write_modern_sidecars(world, opts)?;
    }
    Ok(())
}

fn write_modern_sidecars(world: &Path, opts: &WorldCreateOptions) -> Result<()> {
    let dir = world.join("data/minecraft");
    fs::create_dir_all(&dir)?;
    let wgs = world_gen_settings_json(opts);
    let mut root = Map::new();
    root.insert("DataVersion".into(), json!(opts.version.data_version));
    root.insert("data".into(), wgs);
    write_gzip_nbt(dir.join("world_gen_settings.dat"), &json_to_nbt(&JsonValue::Object(root))?)?;
    Ok(())
}

fn build_data_compound(opts: &WorldCreateOptions) -> Map<String, JsonValue> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut data = Map::new();
    data.insert("DataVersion".into(), json!(opts.version.data_version));
    data.insert("version".into(), json!(ANVIL_LEVEL_VERSION));
    data.insert("LevelName".into(), json!(opts.level_name));
    data.insert("GameType".into(), json!(opts.game_type));
    data.insert("Difficulty".into(), json!(2));
    data.insert("hardcore".into(), json!(0));
    data.insert("allowCommands".into(), json!(1));
    data.insert("initialized".into(), json!(1));
    data.insert("SpawnX".into(), json!(opts.spawn[0]));
    data.insert("SpawnY".into(), json!(opts.spawn[1]));
    data.insert("SpawnZ".into(), json!(opts.spawn[2]));
    data.insert("SpawnAngle".into(), json!(0.0));
    data.insert("LastPlayed".into(), json!(now));
    data.insert("Time".into(), json!(0));
    data.insert("DayTime".into(), json!(1000));
    data.insert("rainTime".into(), json!(0));
    data.insert("thunderTime".into(), json!(0));
    data.insert("raining".into(), json!(0));
    data.insert("thundering".into(), json!(0));
    data.insert("GameRules".into(), default_game_rules());
    data.insert(
        "Version".into(),
        json!({
            "Id": opts.version.data_version,
            "Name": opts.version.name,
            "Snapshot": false,
            "Series": "main"
        }),
    );
    data.insert(
        "DataPacks".into(),
        json!({
            "Disabled": [],
            "Enabled": ["vanilla"]
        }),
    );
    data.insert("WasModded".into(), json!(0));

    // Always embed WorldGenSettings for classic clients / older tools.
    // Modern layout ALSO writes data/minecraft/world_gen_settings.dat.
    data.insert("WorldGenSettings".into(), world_gen_settings_json(opts));
    // Legacy field still read by some editors.
    data.insert("RandomSeed".into(), json!(opts.seed));

    data
}

fn world_gen_settings_json(opts: &WorldCreateOptions) -> JsonValue {
    let overworld_gen = match opts.generator {
        GeneratorKind::Noise => json!({
            "type": "minecraft:noise",
            "settings": "minecraft:overworld",
            "biome_source": {
                "type": "minecraft:multi_noise",
                "preset": "minecraft:overworld"
            }
        }),
        GeneratorKind::Flat => json!({
            "type": "minecraft:flat",
            "settings": {
                "biome": "minecraft:plains",
                "lakes": false,
                "features": false,
                "layers": [
                    {"block": "minecraft:bedrock", "height": 1},
                    {"block": "minecraft:dirt", "height": 2},
                    {"block": "minecraft:grass_block", "height": 1}
                ],
                "structure_overrides": []
            }
        }),
    };
    json!({
        "seed": opts.seed,
        "generate_features": true,
        "bonus_chest": false,
        "dimensions": {
            "minecraft:overworld": {
                "type": "minecraft:overworld",
                "generator": overworld_gen
            },
            "minecraft:the_nether": {
                "type": "minecraft:the_nether",
                "generator": {
                    "type": "minecraft:noise",
                    "settings": "minecraft:nether",
                    "biome_source": {
                        "type": "minecraft:multi_noise",
                        "preset": "minecraft:nether"
                    }
                }
            },
            "minecraft:the_end": {
                "type": "minecraft:the_end",
                "generator": {
                    "type": "minecraft:noise",
                    "settings": "minecraft:end",
                    "biome_source": {
                        "type": "minecraft:the_end"
                    }
                }
            }
        }
    })
}

fn default_game_rules() -> JsonValue {
    json!({
        "doDaylightCycle": "true",
        "doMobSpawning": "true",
        "doFireTick": "true",
        "mobGriefing": "true",
        "keepInventory": "false",
        "doWeatherCycle": "true",
        "commandBlockOutput": "true",
        "naturalRegeneration": "true",
        "doImmediateRespawn": "false",
        "spawnRadius": "10"
    })
}

fn write_gzip_nbt(path: PathBuf, nbt: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fastnbt::to_bytes(nbt)?;
    let file = fs::File::create(&path)?;
    let mut enc = GzEncoder::new(file, Compression::default());
    enc.write_all(&bytes)?;
    enc.finish()?;
    Ok(())
}

pub fn read_gzip_nbt(path: &Path) -> Result<Value> {
    let raw = fs::read(path)?;
    let mut buf = Vec::new();
    let bytes = match GzDecoder::new(&raw[..]).read_to_end(&mut buf) {
        Ok(_) if !buf.is_empty() => buf,
        _ => raw,
    };
    Ok(fastnbt::from_bytes(&bytes)?)
}

/// Read summary from an existing world folder.
pub fn info(world: &Path) -> Result<LevelInfo> {
    let path = world.join("level.dat");
    if !path.exists() {
        return Err(Error::msg(format!("missing {}", path.display())));
    }
    let root = read_gzip_nbt(&path)?;
    let root_json = nbt_to_json(root)?;
    let data = root_json
        .get("Data")
        .cloned()
        .ok_or_else(|| Error::msg("level.dat missing Data"))?;

    let level_name = data
        .get("LevelName")
        .and_then(|v| v.as_str())
        .unwrap_or("world")
        .to_string();
    let spawn = [
        data.get("SpawnX").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        data.get("SpawnY").and_then(|v| v.as_i64()).unwrap_or(64) as i32,
        data.get("SpawnZ").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
    ];
    let data_version = data.get("DataVersion").and_then(|v| v.as_i64()).map(|v| v as i32);
    let modern_layout = data_version.unwrap_or(0) >= MODERN_LEVEL_DAT_MIN
        || world
            .join("data/minecraft/world_gen_settings.dat")
            .exists();

    let mut seed = data
        .get("WorldGenSettings")
        .and_then(|w| w.get("seed"))
        .and_then(|v| v.as_i64())
        .or_else(|| data.get("RandomSeed").and_then(|v| v.as_i64()));

    if seed.is_none() {
        if let Ok(sidecar) = read_gzip_nbt(&world.join("data/minecraft/world_gen_settings.dat")) {
            if let Ok(sj) = nbt_to_json(sidecar) {
                seed = sj
                    .pointer("/data/seed")
                    .or_else(|| sj.get("seed"))
                    .and_then(|v| v.as_i64());
            }
        }
    }

    let generator = data
        .pointer("/WorldGenSettings/dimensions/minecraft:overworld/generator/type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let version_name = data
        .pointer("/Version/Name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(LevelInfo {
        path,
        level_name,
        seed,
        spawn,
        game_type: data.get("GameType").and_then(|v| v.as_i64()).map(|v| v as i32),
        data_version,
        version_name,
        last_played: data.get("LastPlayed").and_then(|v| v.as_i64()),
        generator,
        modern_layout,
    })
}

/// Fields to patch on an existing `level.dat` (all optional; unknown tags preserved).
#[derive(Clone, Debug, Default)]
pub struct LevelPatchOptions<'a> {
    pub level_name: Option<&'a str>,
    pub seed: Option<i64>,
    pub spawn: Option<[i32; 3]>,
    pub game_type: Option<i32>,
    pub data_version: Option<i32>,
    pub version_name: Option<&'a str>,
    pub touch_last_played: bool,
}

/// Patch selected fields on an existing level.dat (preserves unknown tags).
pub fn update_level_dat(world: &Path, opts: &LevelPatchOptions<'_>) -> Result<LevelInfo> {
    let path = world.join("level.dat");
    let root_nbt = read_gzip_nbt(&path)?;
    let mut root = nbt_to_json(root_nbt)?;
    let data = root
        .get_mut("Data")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| Error::msg("level.dat missing Data"))?;

    if let Some(name) = opts.level_name {
        data.insert("LevelName".into(), json!(name));
    }
    if let Some(gt) = opts.game_type {
        data.insert("GameType".into(), json!(gt));
    }
    if let Some([x, y, z]) = opts.spawn {
        data.insert("SpawnX".into(), json!(x));
        data.insert("SpawnY".into(), json!(y));
        data.insert("SpawnZ".into(), json!(z));
    }
    if let Some(dv) = opts.data_version {
        data.insert("DataVersion".into(), json!(dv));
        if let Some(ver) = data.get_mut("Version").and_then(|v| v.as_object_mut()) {
            ver.insert("Id".into(), json!(dv));
            if let Some(n) = opts.version_name {
                ver.insert("Name".into(), json!(n));
            }
        } else {
            data.insert(
                "Version".into(),
                json!({
                    "Id": dv,
                    "Name": opts.version_name.unwrap_or("custom"),
                    "Snapshot": false,
                    "Series": "main"
                }),
            );
        }
    } else if let Some(n) = opts.version_name {
        if let Some(ver) = data.get_mut("Version").and_then(|v| v.as_object_mut()) {
            ver.insert("Name".into(), json!(n));
        }
    }
    if let Some(s) = opts.seed {
        data.insert("RandomSeed".into(), json!(s));
        if let Some(wgs) = data.get_mut("WorldGenSettings").and_then(|v| v.as_object_mut()) {
            wgs.insert("seed".into(), json!(s));
        }
        // Update modern sidecar if present / required.
        let dv = data
            .get("DataVersion")
            .and_then(|v| v.as_i64())
            .unwrap_or(DEFAULT_DATA_VERSION as i64) as i32;
        if dv >= MODERN_LEVEL_DAT_MIN || world.join("data/minecraft/world_gen_settings.dat").exists()
        {
            let sidecar_path = world.join("data/minecraft/world_gen_settings.dat");
            if sidecar_path.exists() {
                if let Ok(sc) = read_gzip_nbt(&sidecar_path) {
                    if let Ok(mut sj) = nbt_to_json(sc) {
                        if let Some(obj) = sj.get_mut("data").and_then(|v| v.as_object_mut()) {
                            obj.insert("seed".into(), json!(s));
                        } else if let Some(obj) = sj.as_object_mut() {
                            obj.insert("seed".into(), json!(s));
                        }
                        write_gzip_nbt(sidecar_path, &json_to_nbt(&sj)?)?;
                    }
                }
            }
        }
    }
    if opts.touch_last_played {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        data.insert("LastPlayed".into(), json!(now));
    }

    write_gzip_nbt(path, &json_to_nbt(&root)?)?;
    info(world)
}

/// True if the world folder looks empty / missing region data (safe to bootstrap).
pub fn needs_bootstrap(world: &Path) -> bool {
    if !world.exists() {
        return true;
    }
    let region = world.join("region");
    if !region.exists() {
        return true;
    }
    let Ok(rd) = fs::read_dir(&region) else {
        return true;
    };
    !rd.filter_map(|e| e.ok()).any(|e| {
        let n = e.file_name();
        let s = n.to_string_lossy();
        s.ends_with(".mca") || s.ends_with(".linear")
    }) && !world.join("level.dat").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_roundtrip_26_2_and_1_20_1() {
        let tmp = TempDir::new().unwrap();
        for (mc, dv, modern) in [("26.2", 4903, true), ("1.20.1", 3465, false)] {
            let path = tmp.path().join(mc);
            let opts = WorldCreateOptions {
                path: path.clone(),
                level_name: format!("test-{mc}"),
                seed: 42,
                spawn: [8, 70, -8],
                game_type: 0,
                version: ResolvedVersion::from_mc(mc).unwrap(),
                generator: GeneratorKind::Flat,
                ..Default::default()
            };
            let info = create_world(&opts).unwrap();
            assert_eq!(info.data_version, Some(dv));
            assert_eq!(info.seed, Some(42));
            assert_eq!(info.spawn, [8, 70, -8]);
            assert_eq!(info.game_type, Some(0));
            assert_eq!(info.modern_layout, modern);
            assert!(path.join("region").is_dir());
            assert!(path.join("level.dat").is_file());
            if modern {
                assert!(path
                    .join("data/minecraft/world_gen_settings.dat")
                    .is_file());
            }
            // Roundtrip: rewrite LastPlayed / seed
            let again = update_level_dat(
                &path,
                &LevelPatchOptions {
                    level_name: Some("renamed"),
                    seed: Some(99),
                    touch_last_played: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(again.level_name, "renamed");
            assert_eq!(again.seed, Some(99));
            assert_eq!(again.data_version, Some(dv));
        }
    }
}
