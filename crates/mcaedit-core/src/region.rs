use crate::chunk::ChunkData;
use crate::error::{Error, Result};
use crate::linear::{
    is_linear_file, linear_to_mca_bytes, mca_to_linear_v1, read_linear, write_linear, LinearRegion,
    LinearVersion,
};
use mca::{Compression, RegionReader, RegionWriter};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn region_coords(chunk_x: i32, chunk_z: i32) -> (i32, i32) {
    (chunk_x >> 5, chunk_z >> 5)
}

pub fn region_file_name(rx: i32, rz: i32) -> String {
    format!("r.{rx}.{rz}.mca")
}

pub fn linear_file_name(rx: i32, rz: i32) -> String {
    format!("r.{rx}.{rz}.linear")
}

pub fn local_chunk(chunk_x: i32, chunk_z: i32) -> (u8, u8) {
    ((chunk_x & 31) as u8, (chunk_z & 31) as u8)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionFormat {
    Anvil,
    LinearV1,
    LinearV2,
}

impl RegionFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        if name.ends_with(".mca") {
            Some(Self::Anvil)
        } else if name.ends_with(".linear") {
            // Version determined when reading; default assume v1 until probed.
            Some(Self::LinearV1)
        } else {
            None
        }
    }

    pub fn is_linear(self) -> bool {
        matches!(self, Self::LinearV1 | Self::LinearV2)
    }
}

#[derive(Debug)]
pub struct RegionStore {
    path: PathBuf,
    /// Dirty chunk keys "x,z"
    dirty: HashSet<(i32, i32)>,
}

impl RegionStore {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            dirty: HashSet::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    fn format_of_path(&self) -> RegionFormat {
        if self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == "linear")
        {
            if self.path.exists() {
                if let Ok(bytes) = fs::read(&self.path) {
                    if let Ok(v) = crate::linear::detect_linear_version(&bytes) {
                        return match v {
                            LinearVersion::V1 => RegionFormat::LinearV1,
                            LinearVersion::V2 => RegionFormat::LinearV2,
                        };
                    }
                }
            }
            RegionFormat::LinearV1
        } else {
            RegionFormat::Anvil
        }
    }

    pub fn read_chunk(&self, chunk_x: i32, chunk_z: i32) -> Result<Option<ChunkData>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)?;
        if is_linear_file(&bytes) {
            let mut linear = read_linear(&bytes)?;
            fill_region_coords_from_path(&mut linear, &self.path)?;
            let (lx, lz) = local_chunk(chunk_x, chunk_z);
            return match linear.get(lx, lz) {
                Some(raw) => Ok(Some(ChunkData::from_bytes(raw)?)),
                None => Ok(None),
            };
        }
        if bytes.len() < 8192 {
            return Ok(None);
        }
        let mut region = RegionReader::new(&bytes)?;
        let (lx, lz) = local_chunk(chunk_x, chunk_z);
        let Some(raw) = region.chunk(lx, lz)? else {
            return Ok(None);
        };
        Ok(Some(ChunkData::from_bytes(raw)?))
    }

    pub fn write_chunk(&mut self, chunk: &ChunkData) -> Result<()> {
        let nbt = chunk.to_bytes()?;
        self.write_raw_chunk_nbt(chunk.chunk_x, chunk.chunk_z, nbt)
    }

    pub fn write_raw_chunk_nbt(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        nbt: Vec<u8>,
    ) -> Result<()> {
        let (rx, rz) = region_coords(chunk_x, chunk_z);
        self.ensure_path_matches_region(rx, rz)?;
        let (lx, lz) = local_chunk(chunk_x, chunk_z);
        let ts = now_secs() as u32;
        let fmt = self.format_of_path();

        match fmt {
            RegionFormat::Anvil => self.write_anvil_raw(lx, lz, nbt)?,
            RegionFormat::LinearV1 | RegionFormat::LinearV2 => {
                let version = match fmt {
                    RegionFormat::LinearV2 => LinearVersion::V2,
                    _ => LinearVersion::V1,
                };
                let mut linear = if self.path.exists() {
                    let bytes = fs::read(&self.path)?;
                    let mut r = read_linear(&bytes)?;
                    fill_region_coords_from_path(&mut r, &self.path)?;
                    r
                } else {
                    let mut r = LinearRegion::empty(rx, rz);
                    r.version = version;
                    r
                };
                linear.version = version;
                linear.region_x = rx;
                linear.region_z = rz;
                linear.set(lx, lz, Some(nbt), ts);
                let out = write_linear(&linear)?;
                atomic_write_ext(&self.path, &out, "linear.tmp")?;
            }
        }
        self.dirty.insert((chunk_x, chunk_z));
        Ok(())
    }

    fn write_anvil_raw(&self, lx: u8, lz: u8, nbt: Vec<u8>) -> Result<()> {
        if self.path.exists() {
            let bytes = fs::read(&self.path)?;
            if is_linear_file(&bytes) {
                // Convert in place to anvil when path says .mca but content was linear.
                let linear = read_linear(&bytes)?;
                let mca = linear_to_mca_bytes(&linear)?;
                let region = RegionReader::new(&mca)?;
                let mut writer = region.into_writer(())?;
                writer.set_chunk(lx, lz, nbt, Compression::default())?;
                let mut out = Vec::new();
                writer.write(&mut out)?;
                atomic_write(&self.path, &out)?;
                return Ok(());
            }
            let region = RegionReader::new(&bytes)?;
            let mut writer = region.into_writer(())?;
            writer.set_chunk(lx, lz, nbt, Compression::default())?;
            let mut out = Vec::new();
            writer.write(&mut out)?;
            atomic_write(&self.path, &out)?;
        } else {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut writer = RegionWriter::new();
            writer.set_chunk(lx, lz, nbt, Compression::default())?;
            let mut out = Vec::new();
            writer.write(&mut out)?;
            atomic_write(&self.path, &out)?;
        }
        Ok(())
    }

    fn ensure_path_matches_region(&self, rx: i32, rz: i32) -> Result<()> {
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::msg("bad region path"))?;
        let expected_mca = region_file_name(rx, rz);
        let expected_linear = linear_file_name(rx, rz);
        if name != expected_mca.as_str() && name != expected_linear.as_str() {
            return Err(Error::msg(format!(
                "region path {} does not match chunk region {expected_mca}/{expected_linear}",
                self.path.display()
            )));
        }
        Ok(())
    }

    pub fn read_raw_chunk_nbt(&self, chunk_x: i32, chunk_z: i32) -> Result<Option<Vec<u8>>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)?;
        let (lx, lz) = local_chunk(chunk_x, chunk_z);
        if is_linear_file(&bytes) {
            let linear = read_linear(&bytes)?;
            return Ok(linear.get(lx, lz).map(|b| b.to_vec()));
        }
        if bytes.len() < 8192 {
            return Ok(None);
        }
        let mut region = RegionReader::new(&bytes)?;
        Ok(region.chunk(lx, lz)?.map(|b| b.to_vec()))
    }

    pub fn list_present_chunks(&self) -> Result<Vec<(i32, i32)>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&self.path)?;
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::msg("bad region name"))?;
        let (rx, rz) = parse_region_name(name)?;
        if is_linear_file(&bytes) {
            let linear = read_linear(&bytes)?;
            let mut out = Vec::new();
            for z in 0..32u8 {
                for x in 0..32u8 {
                    if linear.get(x, z).is_some() {
                        let cx = (rx << 5) + x as i32;
                        let cz = (rz << 5) + z as i32;
                        out.push((cx, cz));
                    }
                }
            }
            return Ok(out);
        }
        let region = RegionReader::new(&bytes)?;
        let mut out = Vec::new();
        let mut iter = region.iter()?;
        while let Some(((lx, lz), _)) = iter.next_available_chunk()? {
            let cx = (rx << 5) + lx as i32;
            let cz = (rz << 5) + lz as i32;
            out.push((cx, cz));
        }
        Ok(out)
    }
}

fn fill_region_coords_from_path(linear: &mut LinearRegion, path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::msg("bad linear path"))?;
    let (rx, rz) = parse_region_name(name)?;
    linear.region_x = rx;
    linear.region_z = rz;
    Ok(())
}

pub fn parse_region_name(name: &str) -> Result<(i32, i32)> {
    let stem = name
        .strip_suffix(".mca")
        .or_else(|| name.strip_suffix(".linear"))
        .unwrap_or(name);
    let mut parts = stem.split('.');
    let r = parts.next();
    let x = parts.next();
    let z = parts.next();
    if r != Some("r") {
        return Err(Error::msg(format!("bad region name {name}")));
    }
    let rx: i32 = x
        .ok_or_else(|| Error::msg("missing rx"))?
        .parse()
        .map_err(|_| Error::msg("bad rx"))?;
    let rz: i32 = z
        .ok_or_else(|| Error::msg("missing rz"))?
        .parse()
        .map_err(|_| Error::msg("bad rz"))?;
    Ok((rx, rz))
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    atomic_write_ext(path, data, "mca.tmp")
}

pub fn atomic_write_ext(path: &Path, data: &[u8], tmp_ext: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(tmp_ext);
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Probe source dir for `.mca` or `.linear` for the given region coords.
pub fn find_source_region_file(src_dir: &Path, rx: i32, rz: i32) -> Option<(PathBuf, RegionFormat)> {
    let mca = src_dir.join(region_file_name(rx, rz));
    if mca.exists() {
        return Some((mca, RegionFormat::Anvil));
    }
    let linear = src_dir.join(linear_file_name(rx, rz));
    if linear.exists() {
        let fmt = if let Ok(bytes) = fs::read(&linear) {
            match crate::linear::detect_linear_version(&bytes) {
                Ok(LinearVersion::V2) => RegionFormat::LinearV2,
                _ => RegionFormat::LinearV1,
            }
        } else {
            RegionFormat::LinearV1
        };
        return Some((linear, fmt));
    }
    None
}

/// Ensure work copy has an editable region file.
///
/// Policy: work copy prefers `.mca`. If source is Linear-only, convert into work `.mca`
/// and leave a marker `r.X.Z.linear.source` so commit can write Linear back.
pub fn copy_region_if_needed(src_dir: &Path, dst_dir: &Path, rx: i32, rz: i32) -> Result<PathBuf> {
    fs::create_dir_all(dst_dir)?;
    let dst_mca = dst_dir.join(region_file_name(rx, rz));
    if dst_mca.exists() {
        return Ok(dst_mca);
    }
    let dst_linear = dst_dir.join(linear_file_name(rx, rz));
    if dst_linear.exists() {
        // Edit path normalizes to MCA for pumpkin bridge compatibility.
        let bytes = fs::read(&dst_linear)?;
        let mut linear = read_linear(&bytes)?;
        linear.region_x = rx;
        linear.region_z = rz;
        let mca = linear_to_mca_bytes(&linear)?;
        atomic_write(&dst_mca, &mca)?;
        write_linear_source_marker(dst_dir, rx, rz, linear.version)?;
        let _ = fs::remove_file(&dst_linear);
        return Ok(dst_mca);
    }

    match find_source_region_file(src_dir, rx, rz) {
        Some((src, RegionFormat::Anvil)) => {
            fs::copy(&src, &dst_mca)?;
            Ok(dst_mca)
        }
        Some((src, RegionFormat::LinearV1 | RegionFormat::LinearV2)) => {
            let bytes = fs::read(&src)?;
            let mut linear = read_linear(&bytes)?;
            linear.region_x = rx;
            linear.region_z = rz;
            let mca = linear_to_mca_bytes(&linear)?;
            atomic_write(&dst_mca, &mca)?;
            write_linear_source_marker(dst_dir, rx, rz, linear.version)?;
            Ok(dst_mca)
        }
        None => Ok(dst_mca),
    }
}

pub fn linear_source_marker_path(dir: &Path, rx: i32, rz: i32) -> PathBuf {
    dir.join(format!("r.{rx}.{rz}.linear.source"))
}

pub fn write_linear_source_marker(
    dir: &Path,
    rx: i32,
    rz: i32,
    version: LinearVersion,
) -> Result<()> {
    let v = match version {
        LinearVersion::V1 => "1",
        LinearVersion::V2 => "2",
    };
    fs::write(linear_source_marker_path(dir, rx, rz), format!("v{v}\n"))?;
    Ok(())
}

pub fn read_linear_source_marker(dir: &Path, rx: i32, rz: i32) -> Option<LinearVersion> {
    let p = linear_source_marker_path(dir, rx, rz);
    let Ok(s) = fs::read_to_string(p) else {
        return None;
    };
    match s.trim() {
        "v2" | "2" => Some(LinearVersion::V2),
        _ => Some(LinearVersion::V1),
    }
}

/// Convert work `.mca` back to `.linear` for commit when source was Linear.
pub fn work_mca_to_linear_bytes(
    mca_path: &Path,
    rx: i32,
    rz: i32,
    version: LinearVersion,
) -> Result<Vec<u8>> {
    let mca = fs::read(mca_path)?;
    let mut linear = mca_to_linear_v1(&mca, rx, rz)?;
    linear.version = version;
    if version == LinearVersion::V2 {
        linear.grid_size = 2;
    }
    write_linear(&linear)
}

pub fn list_region_files(dir: &Path) -> Result<Vec<(i32, i32)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.ends_with(".mca") || name.ends_with(".linear")) {
            continue;
        }
        if let Ok(coords) = parse_region_name(name) {
            if seen.insert(coords) {
                out.push(coords);
            }
        }
    }
    Ok(out)
}

/// Convert `src` Linear file into Anvil bytes (helper for tests / CLI).
pub fn convert_linear_file_to_mca(src: &Path, dst: &Path) -> Result<()> {
    let bytes = fs::read(src)?;
    let linear = read_linear(&bytes)?;
    let mca = linear_to_mca_bytes(&linear)?;
    atomic_write(dst, &mca)
}

/// Convert Anvil `.mca` into Linear v1 `.linear`.
pub fn convert_mca_file_to_linear(src: &Path, dst: &Path, version: LinearVersion) -> Result<()> {
    let (rx, rz) = parse_region_name(
        src.file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::msg("bad mca name"))?,
    )?;
    let mca = fs::read(src)?;
    let mut linear = mca_to_linear_v1(&mca, rx, rz)?;
    linear.version = version;
    let out = write_linear(&linear)?;
    atomic_write_ext(dst, &out, "linear.tmp")
}
