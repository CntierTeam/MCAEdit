//! Minecraft client jar texture discovery and loading (block atlas for view).
//!
//! Reads `assets/minecraft/textures/block/*.png` from a client/assets jar
//! (Minecraft **26.2** by default). Missing jar → callers keep palette colors.

use crate::error::{Error, Result};
use image::RgbaImage;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Default client version folder / jar stem for auto-detect.
pub const DEFAULT_MC_VERSION: &str = "26.2";

const BLOCK_TEX_PREFIX: &str = "assets/minecraft/textures/block/";

/// Face of a unit cube (world axes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CubeFace {
    NegX,
    PosX,
    NegY,
    PosY,
    NegZ,
    PosZ,
}

impl CubeFace {
    pub fn is_top(self) -> bool {
        matches!(self, Self::PosY)
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, Self::NegY)
    }
}

/// Lazy block-texture cache backed by a Minecraft client jar (zip).
#[derive(Debug)]
pub struct BlockTextureAtlas {
    path: PathBuf,
    /// Texture path stem → decoded first-frame RGBA (None = confirmed missing).
    cache: HashMap<String, Option<RgbaImage>>,
}

impl BlockTextureAtlas {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open an explicit jar path (must look like a client assets jar).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = normalize_jar_path(path.as_ref())
            .ok_or_else(|| Error::msg("minecraft/assets jar path not found"))?;
        if !jar_has_block_textures(&path) {
            return Err(Error::msg(format!(
                "jar lacks block textures: {}",
                path.display()
            )));
        }
        Ok(Self {
            path,
            cache: HashMap::new(),
        })
    }

    /// Try CLI paths, env vars, then auto-detect `version` (default 26.2).
    ///
    /// Priority:
    /// 1. `assets_jar` argument
    /// 2. `minecraft` argument (jar file or `versions/<ver>/` directory)
    /// 3. `MCAEDIT_ASSETS_JAR`
    /// 4. `MCAEDIT_MINECRAFT_JAR`
    /// 5. common launcher paths for `version`
    pub fn resolve(
        minecraft: Option<&Path>,
        assets_jar: Option<&Path>,
        version: &str,
    ) -> Option<Self> {
        let mut tried = Vec::new();
        if let Some(p) = assets_jar {
            tried.push(p.to_path_buf());
        }
        if let Some(p) = minecraft {
            tried.push(p.to_path_buf());
        }
        if let Ok(p) = std::env::var("MCAEDIT_ASSETS_JAR") {
            if !p.is_empty() {
                tried.push(PathBuf::from(p));
            }
        }
        if let Ok(p) = std::env::var("MCAEDIT_MINECRAFT_JAR") {
            if !p.is_empty() {
                tried.push(PathBuf::from(p));
            }
        }
        for c in candidate_jar_paths(version) {
            tried.push(c);
        }

        let mut seen = std::collections::HashSet::new();
        for raw in tried {
            if !seen.insert(raw.clone()) {
                continue;
            }
            if let Some(norm) = normalize_jar_path(&raw) {
                if let Ok(atlas) = Self::open(&norm) {
                    return Some(atlas);
                }
            }
        }
        None
    }

    /// Auto-detect only (no CLI/env). Useful for tests / docs.
    pub fn discover(version: &str) -> Option<Self> {
        Self::resolve(None, None, version)
    }

    /// Load (and cache) a block texture by stem, e.g. `"stone"` or `"grass_block_top"`.
    /// Animated vertical strips use the **first frame** only.
    pub fn load_block_texture(&mut self, stem: &str) -> Option<&RgbaImage> {
        if !self.cache.contains_key(stem) {
            let entry = format!("{BLOCK_TEX_PREFIX}{stem}.png");
            let img = read_png_from_jar(&self.path, &entry).map(first_animation_frame);
            self.cache.insert(stem.to_string(), img);
        }
        self.cache.get(stem).and_then(|o| o.as_ref())
    }

    /// Resolve a cube-face texture for a block id (`minecraft:stone` or `stone`).
    /// Returns owned image clone for mesh packing (caller caches by stem).
    pub fn image_for_block_face(&mut self, block_name: &str, face: CubeFace) -> Option<RgbaImage> {
        let stem = block_stem(block_name);
        for cand in face_texture_candidates(stem, face) {
            if let Some(img) = self.load_block_texture(&cand) {
                return Some(img.clone());
            }
        }
        None
    }
}

/// Resolve jar path the same way as [`BlockTextureAtlas::resolve`], returning the path only.
pub fn resolve_assets_jar_path(
    minecraft: Option<&Path>,
    assets_jar: Option<&Path>,
    version: &str,
) -> Option<PathBuf> {
    BlockTextureAtlas::resolve(minecraft, assets_jar, version).map(|a| a.path().to_path_buf())
}

fn block_stem(block_name: &str) -> &str {
    block_name
        .rsplit_once(':')
        .map(|(_, n)| n)
        .unwrap_or(block_name)
}

/// Heuristic face → texture stem list (no full blockstate models in v1).
pub fn face_texture_candidates(stem: &str, face: CubeFace) -> Vec<String> {
    let mut out = Vec::new();
    match stem {
        "grass_block" => {
            if face.is_top() {
                out.push("grass_block_top".into());
            } else if face.is_bottom() {
                out.push("dirt".into());
            } else {
                out.push("grass_block_side".into());
            }
        }
        "mycelium" => {
            if face.is_top() {
                out.push("mycelium_top".into());
            } else if face.is_bottom() {
                out.push("dirt".into());
            } else {
                out.push("mycelium_side".into());
            }
        }
        "podzol" => {
            if face.is_top() {
                out.push("podzol_top".into());
            } else if face.is_bottom() {
                out.push("dirt".into());
            } else {
                out.push("podzol_side".into());
            }
        }
        "crimson_nylium" | "warped_nylium" => {
            if face.is_top() {
                out.push(stem.to_string());
            } else if face.is_bottom() {
                out.push("netherrack".into());
            } else {
                out.push(format!("{stem}_side"));
            }
        }
        "water" => {
            out.push("water_still".into());
            out.push("water".into());
        }
        "lava" => {
            out.push("lava_still".into());
            out.push("lava".into());
        }
        "snow_block" => {
            out.push("snow".into());
        }
        "magma_block" => {
            out.push("magma".into());
        }
        name if name.ends_with("_log")
            || name.ends_with("_wood")
            || name.ends_with("_stem")
            || name.ends_with("_hyphae") =>
        {
            if face.is_top() || face.is_bottom() {
                out.push(format!("{name}_top"));
                out.push(name.to_string());
            } else {
                out.push(name.to_string());
            }
        }
        name => {
            if face.is_top() {
                out.push(format!("{name}_top"));
            } else if face.is_bottom() {
                out.push(format!("{name}_bottom"));
                out.push(format!("{name}_top"));
            } else {
                out.push(format!("{name}_side"));
            }
            out.push(name.to_string());
        }
    }
    out
}

/// If `path` is a version dir, map to `<dir>/<name>.jar`; if file, keep.
pub fn normalize_jar_path(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    if path.is_dir() {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            let named = path.join(format!("{name}.jar"));
            if named.is_file() {
                return Some(named);
            }
        }
        // Any single *.jar in the directory.
        if let Ok(rd) = std::fs::read_dir(path) {
            let jars: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jar"))
                .collect();
            if jars.len() == 1 {
                return Some(jars[0].clone());
            }
            // Prefer `<dirname>.jar` already handled; else first jar with block textures.
            for j in jars {
                if jar_has_block_textures(&j) {
                    return Some(j);
                }
            }
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Common launcher locations for `versions/<ver>/<ver>.jar`.
pub fn candidate_jar_paths(version: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let rel = format!("versions/{version}/{version}.jar");
    // buildTest / shared Minecraft layout on this fleet
    out.push(PathBuf::from(format!("/other/Minecraft/.minecraft/{rel}")));
    out.push(PathBuf::from(format!("/other/Minecraft/minecraft/{rel}")));
    out.push(PathBuf::from(format!("/Minecraft/.minecraft/{rel}")));
    if let Some(home) = home_dir() {
        out.push(home.join(".minecraft").join(&rel));
        out.push(
            home.join(".var/app/com.mojang.Minecraft/.minecraft")
                .join(&rel),
        );
        out.push(
            home.join("Library/Application Support/minecraft")
                .join(&rel),
        );
        out.push(home.join(".hmcl").join(&rel));
        out.push(home.join(".minecraft_hmcl").join(&rel));
        for launcher in [
            "PrismLauncher",
            "PolyMC",
            "MultiMC",
            "pollymc",
            "xmcl",
        ] {
            let instances = home.join(format!(".local/share/{launcher}/instances"));
            push_instance_jars(&instances, version, &mut out);
            let alt = home.join(format!(".local/share/{launcher}"));
            if alt != instances {
                push_instance_jars(&alt.join("instances"), version, &mut out);
            }
        }
        // Flatpak Prism
        push_instance_jars(
            &home.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances"),
            version,
            &mut out,
        );
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        out.push(PathBuf::from(appdata).join(".minecraft").join(&rel));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        for launcher in ["PrismLauncher", "PolyMC", "MultiMC"] {
            push_instance_jars(
                &PathBuf::from(&xdg).join(launcher).join("instances"),
                version,
                &mut out,
            );
        }
    }
    out
}

fn push_instance_jars(instances: &Path, version: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(instances) else {
        return;
    };
    for ent in rd.flatten() {
        let base = ent.path();
        out.push(
            base.join("minecraft")
                .join("versions")
                .join(version)
                .join(format!("{version}.jar")),
        );
        out.push(
            base.join(".minecraft")
                .join("versions")
                .join(version)
                .join(format!("{version}.jar")),
        );
    }
}

fn jar_has_block_textures(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return false;
    };
    // Prefer a cheap existence check for a ubiquitous texture.
    for probe in [
        "assets/minecraft/textures/block/stone.png",
        "assets/minecraft/textures/block/dirt.png",
        "assets/minecraft/textures/block/oak_planks.png",
    ] {
        if zip.by_name(probe).is_ok() {
            return true;
        }
    }
    // Fallback: any block texture entry.
    for i in 0..zip.len().min(4096) {
        if let Ok(e) = zip.by_index(i) {
            let name = e.name();
            if name.starts_with(BLOCK_TEX_PREFIX) && name.ends_with(".png") {
                return true;
            }
        }
    }
    false
}

fn read_png_from_jar(jar: &Path, entry: &str) -> Option<RgbaImage> {
    let file = File::open(jar).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut zf = zip.by_name(entry).ok()?;
    let mut buf = Vec::with_capacity(zf.size() as usize);
    zf.read_to_end(&mut buf).ok()?;
    image::load_from_memory(&buf).ok().map(|i| i.to_rgba8())
}

/// Animated MC textures are vertical frame strips (`height = width * frames`).
fn first_animation_frame(img: RgbaImage) -> RgbaImage {
    let w = img.width();
    let h = img.height();
    if w > 0 && h > w && h.is_multiple_of(w) {
        image::imageops::crop_imm(&img, 0, 0, w, w).to_image()
    } else {
        img
    }
}

/// Whether this block's grayscale overlay textures need palette tint (grass/leaves).
pub fn needs_biome_tint(block_name: &str) -> bool {
    let n = block_stem(block_name);
    n.contains("grass") || n.contains("leaves") || n.contains("vine") || n == "fern" || n == "large_fern"
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn write_solid_png(path: &Path, color: [u8; 4], w: u32, h: u32) {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = Rgba(color);
        }
        img.save(path).unwrap();
    }

    fn build_fixture_jar(dir: &Path) -> PathBuf {
        let jar = dir.join("fake-26.2.jar");
        let stone_png = dir.join("stone.png");
        let dirt_png = dir.join("dirt.png");
        let anim_png = dir.join("water_still.png");
        write_solid_png(&stone_png, [0x80, 0x80, 0x80, 0xFF], 2, 2);
        write_solid_png(&dirt_png, [0x82, 0x5A, 0x3A, 0xFF], 2, 2);
        // 2x4 = two animation frames
        write_solid_png(&anim_png, [0x3F, 0x76, 0xE4, 0xFF], 2, 4);

        let file = File::create(&jar).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, src) in [
            ("assets/minecraft/textures/block/stone.png", &stone_png),
            ("assets/minecraft/textures/block/dirt.png", &dirt_png),
            ("assets/minecraft/textures/block/water_still.png", &anim_png),
            (
                "assets/minecraft/textures/block/grass_block_top.png",
                &stone_png,
            ),
            (
                "assets/minecraft/textures/block/grass_block_side.png",
                &dirt_png,
            ),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(&std::fs::read(src).unwrap()).unwrap();
        }
        zip.finish().unwrap();
        jar
    }

    #[test]
    fn open_fixture_jar_and_load_stone() {
        let dir = tempfile::tempdir().unwrap();
        let jar = build_fixture_jar(dir.path());
        let mut atlas = BlockTextureAtlas::open(&jar).unwrap();
        let stone = atlas.load_block_texture("stone").unwrap();
        assert_eq!(stone.width(), 2);
        assert_eq!(stone.get_pixel(0, 0).0[0], 0x80);
    }

    #[test]
    fn animation_uses_first_frame_only() {
        let dir = tempfile::tempdir().unwrap();
        let jar = build_fixture_jar(dir.path());
        let mut atlas = BlockTextureAtlas::open(&jar).unwrap();
        let water = atlas.load_block_texture("water_still").unwrap();
        assert_eq!(water.width(), 2);
        assert_eq!(water.height(), 2);
    }

    #[test]
    fn grass_block_face_candidates() {
        let top = face_texture_candidates("grass_block", CubeFace::PosY);
        assert_eq!(top[0], "grass_block_top");
        let side = face_texture_candidates("grass_block", CubeFace::PosX);
        assert_eq!(side[0], "grass_block_side");
        let bot = face_texture_candidates("grass_block", CubeFace::NegY);
        assert_eq!(bot[0], "dirt");
    }

    #[test]
    fn normalize_version_dir_to_jar() {
        let dir = tempfile::tempdir().unwrap();
        let ver_dir = dir.path().join("26.2");
        std::fs::create_dir_all(&ver_dir).unwrap();
        let jar = build_fixture_jar(dir.path());
        let dest = ver_dir.join("26.2.jar");
        std::fs::copy(&jar, &dest).unwrap();
        let norm = normalize_jar_path(&ver_dir).unwrap();
        assert_eq!(norm, dest);
    }

    #[test]
    fn resolve_prefers_assets_jar_arg() {
        let dir = tempfile::tempdir().unwrap();
        let jar = build_fixture_jar(dir.path());
        let atlas = BlockTextureAtlas::resolve(None, Some(&jar), "26.2").unwrap();
        assert_eq!(atlas.path(), jar.as_path());
    }

    #[test]
    fn open_missing_jar_fails_gracefully() {
        let err = BlockTextureAtlas::open("/no/such/minecraft-26.2.jar");
        assert!(err.is_err());
    }
}
