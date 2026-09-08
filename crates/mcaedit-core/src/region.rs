use crate::chunk::ChunkData;
use crate::error::{Error, Result};
use mca::{Compression, RegionReader, RegionWriter};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn region_coords(chunk_x: i32, chunk_z: i32) -> (i32, i32) {
    (chunk_x >> 5, chunk_z >> 5)
}

pub fn region_file_name(rx: i32, rz: i32) -> String {
    format!("r.{rx}.{rz}.mca")
}

pub fn local_chunk(chunk_x: i32, chunk_z: i32) -> (u8, u8) {
    ((chunk_x & 31) as u8, (chunk_z & 31) as u8)
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

    pub fn read_chunk(&self, chunk_x: i32, chunk_z: i32) -> Result<Option<ChunkData>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path)?;
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
        let (rx, rz) = region_coords(chunk.chunk_x, chunk.chunk_z);
        let expected = region_file_name(rx, rz);
        if self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            != Some(expected.as_str())
        {
            return Err(Error::msg(format!(
                "region path {} does not match chunk region {expected}",
                self.path.display()
            )));
        }

        let (lx, lz) = local_chunk(chunk.chunk_x, chunk.chunk_z);
        if self.path.exists() {
            let bytes = fs::read(&self.path)?;
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
        self.dirty.insert((chunk.chunk_x, chunk.chunk_z));
        Ok(())
    }

    pub fn list_present_chunks(&self) -> Result<Vec<(i32, i32)>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&self.path)?;
        let region = RegionReader::new(&bytes)?;
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::msg("bad region name"))?;
        let (rx, rz) = parse_region_name(name)?;
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

pub fn parse_region_name(name: &str) -> Result<(i32, i32)> {
    // r.X.Z.mca
    let stem = name.strip_suffix(".mca").unwrap_or(name);
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("mca.tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn copy_region_if_needed(src_dir: &Path, dst_dir: &Path, rx: i32, rz: i32) -> Result<PathBuf> {
    fs::create_dir_all(dst_dir)?;
    let name = region_file_name(rx, rz);
    let dst = dst_dir.join(&name);
    if dst.exists() {
        return Ok(dst);
    }
    let src = src_dir.join(&name);
    if src.exists() {
        fs::copy(&src, &dst)?;
    }
    Ok(dst)
}

pub fn list_region_files(dir: &Path) -> Result<Vec<(i32, i32)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".mca") {
            continue;
        }
        if let Ok(coords) = parse_region_name(name) {
            out.push(coords);
        }
    }
    Ok(out)
}
