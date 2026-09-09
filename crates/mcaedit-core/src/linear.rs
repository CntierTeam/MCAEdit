//! Linear region file format (`.linear`) — v1 (xymb) and v2 (bucket grid).
//!
//! Spec references:
//! - v1: <https://github.com/xymb-endcrystalme/LinearRegionFileFormatTools>
//! - v2: <https://gist.github.com/Aaron2550/5701519671253d4c6190bde6706f9f98>

use crate::error::{Error, Result};
use bytes::{Buf, BufMut};
use std::io::Read;
use xxhash_rust::xxh64::xxh64;

pub const LINEAR_SIGNATURE: u64 = 0xc3ff_1318_3cca_9d9a;
pub const CHUNK_COUNT: usize = 32 * 32;
const V1_HEADER_LEN: usize = 32; // signature..hash
const DEFAULT_V1_COMPRESSION: i8 = 6;
const DEFAULT_V2_GRID: u8 = 2;
const VALID_V2_GRIDS: &[u8] = &[1, 2, 4, 8, 16, 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinearVersion {
    V1,
    V2,
}

#[derive(Clone, Debug)]
pub struct LinearRegion {
    pub version: LinearVersion,
    pub region_x: i32,
    pub region_z: i32,
    pub newest_timestamp: u64,
    /// Per-chunk uncompressed NBT bytes (Anvil chunk payload, no length/compression header).
    pub chunks: Vec<Option<Vec<u8>>>,
    pub timestamps: Vec<u32>,
    /// v2 only
    pub grid_size: u8,
}

impl LinearRegion {
    pub fn empty(region_x: i32, region_z: i32) -> Self {
        Self {
            version: LinearVersion::V1,
            region_x,
            region_z,
            newest_timestamp: 0,
            chunks: vec![None; CHUNK_COUNT],
            timestamps: vec![0; CHUNK_COUNT],
            grid_size: DEFAULT_V2_GRID,
        }
    }

    pub fn chunk_index(local_x: u8, local_z: u8) -> usize {
        (local_z as usize) * 32 + (local_x as usize)
    }

    pub fn get(&self, local_x: u8, local_z: u8) -> Option<&[u8]> {
        self.chunks[Self::chunk_index(local_x, local_z)].as_deref()
    }

    pub fn set(&mut self, local_x: u8, local_z: u8, nbt: Option<Vec<u8>>, timestamp: u32) {
        let i = Self::chunk_index(local_x, local_z);
        if let Some(ref bytes) = nbt {
            self.newest_timestamp = self.newest_timestamp.max(timestamp as u64);
            self.timestamps[i] = timestamp;
            self.chunks[i] = Some(bytes.clone());
        } else {
            self.timestamps[i] = 0;
            self.chunks[i] = None;
        }
    }

    pub fn present_count(&self) -> usize {
        self.chunks.iter().filter(|c| c.is_some()).count()
    }
}

/// Detect Linear version from raw file bytes (signature + version byte).
pub fn detect_linear_version(bytes: &[u8]) -> Result<LinearVersion> {
    if bytes.len() < 9 {
        return Err(Error::msg("linear: file too short"));
    }
    let sig = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    if sig != LINEAR_SIGNATURE {
        return Err(Error::msg("linear: bad signature"));
    }
    match bytes[8] {
        1 => Ok(LinearVersion::V1),
        2 => Ok(LinearVersion::V2),
        v => Err(Error::msg(format!("linear: unsupported version {v}"))),
    }
}

pub fn is_linear_file(bytes: &[u8]) -> bool {
    detect_linear_version(bytes).is_ok()
}

pub fn read_linear(bytes: &[u8]) -> Result<LinearRegion> {
    match detect_linear_version(bytes)? {
        LinearVersion::V1 => read_linear_v1(bytes),
        LinearVersion::V2 => read_linear_v2(bytes),
    }
}

pub fn write_linear(region: &LinearRegion) -> Result<Vec<u8>> {
    match region.version {
        LinearVersion::V1 => write_linear_v1(region, DEFAULT_V1_COMPRESSION),
        LinearVersion::V2 => write_linear_v2(region),
    }
}

/// Linear v1: whole-region zstd blob after 32-byte header.
pub fn read_linear_v1(bytes: &[u8]) -> Result<LinearRegion> {
    if bytes.len() < V1_HEADER_LEN + 8 {
        return Err(Error::msg("linear v1: truncated"));
    }
    let sig = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    if sig != LINEAR_SIGNATURE {
        return Err(Error::msg("linear v1: bad header signature"));
    }
    let version = bytes[8];
    if version != 1 {
        return Err(Error::msg(format!("linear v1: unexpected version {version}")));
    }
    let newest_timestamp = u64::from_be_bytes(bytes[9..17].try_into().unwrap());
    let _compression_level = bytes[17] as i8;
    let chunk_count = i16::from_be_bytes(bytes[18..20].try_into().unwrap());
    let compressed_len = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    // bytes[24..32] = reserved hash (usually 0)
    let footer = u64::from_be_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
    if footer != LINEAR_SIGNATURE {
        return Err(Error::msg("linear v1: bad footer signature"));
    }
    let compressed = &bytes[V1_HEADER_LEN..bytes.len() - 8];
    if compressed.len() != compressed_len {
        return Err(Error::msg(format!(
            "linear v1: compressed length mismatch (header={compressed_len} actual={})",
            compressed.len()
        )));
    }
    let decompressed = zstd::decode_all(compressed)
        .map_err(|e| Error::msg(format!("linear v1: zstd decompress: {e}")))?;

    let header_size = CHUNK_COUNT * 8;
    if decompressed.len() < header_size {
        return Err(Error::msg("linear v1: decompressed too short"));
    }
    let mut sizes = Vec::with_capacity(CHUNK_COUNT);
    let mut timestamps = Vec::with_capacity(CHUNK_COUNT);
    let mut real_count = 0usize;
    let mut total_size = 0usize;
    for i in 0..CHUNK_COUNT {
        let off = i * 8;
        let size = u32::from_be_bytes(decompressed[off..off + 4].try_into().unwrap()) as usize;
        let ts = u32::from_be_bytes(decompressed[off + 4..off + 8].try_into().unwrap());
        if size > 0 {
            real_count += 1;
        }
        total_size += size;
        sizes.push(size);
        timestamps.push(ts);
    }
    if header_size + total_size != decompressed.len() {
        return Err(Error::msg("linear v1: decompressed size invalid"));
    }
    if real_count != chunk_count as usize {
        return Err(Error::msg(format!(
            "linear v1: chunk count mismatch (header={chunk_count} actual={real_count})"
        )));
    }

    let mut chunks = vec![None; CHUNK_COUNT];
    let mut cursor = header_size;
    for i in 0..CHUNK_COUNT {
        let size = sizes[i];
        if size > 0 {
            chunks[i] = Some(decompressed[cursor..cursor + size].to_vec());
            cursor += size;
        }
    }

    Ok(LinearRegion {
        version: LinearVersion::V1,
        region_x: 0, // filled by caller from filename when needed
        region_z: 0,
        newest_timestamp,
        chunks,
        timestamps,
        grid_size: DEFAULT_V2_GRID,
    })
}

pub fn write_linear_v1(region: &LinearRegion, compression_level: i8) -> Result<Vec<u8>> {
    let mut inside = Vec::with_capacity(CHUNK_COUNT * 8);
    let mut body = Vec::new();
    let mut newest = 0u64;
    let mut count = 0i16;
    for i in 0..CHUNK_COUNT {
        match &region.chunks[i] {
            Some(data) => {
                inside.extend_from_slice(&(data.len() as u32).to_be_bytes());
                inside.extend_from_slice(&region.timestamps[i].to_be_bytes());
                newest = newest.max(region.timestamps[i] as u64);
                count += 1;
                body.extend_from_slice(data);
            }
            None => {
                inside.extend_from_slice(&0u32.to_be_bytes());
                inside.extend_from_slice(&0u32.to_be_bytes());
            }
        }
    }
    let mut uncompressed = inside;
    uncompressed.extend_from_slice(&body);
    let level = compression_level.clamp(1, 22) as i32;
    let compressed = zstd::encode_all(uncompressed.as_slice(), level)
        .map_err(|e| Error::msg(format!("linear v1: zstd compress: {e}")))?;

    let mut out = Vec::with_capacity(V1_HEADER_LEN + compressed.len() + 8);
    out.extend_from_slice(&LINEAR_SIGNATURE.to_be_bytes());
    out.push(1); // version
    out.extend_from_slice(&newest.to_be_bytes());
    out.push(compression_level as u8);
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes()); // reserved hash
    out.extend_from_slice(&compressed);
    out.extend_from_slice(&LINEAR_SIGNATURE.to_be_bytes());
    Ok(out)
}

/// Linear v2: grid of independently zstd-compressed buckets.
pub fn read_linear_v2(bytes: &[u8]) -> Result<LinearRegion> {
    let mut buf = bytes;
    if buf.len() < 26 {
        return Err(Error::msg("linear v2: truncated superblock"));
    }
    let sig = u64::from_be_bytes(buf[0..8].try_into().unwrap());
    if sig != LINEAR_SIGNATURE {
        return Err(Error::msg("linear v2: bad header signature"));
    }
    if buf[8] != 2 {
        return Err(Error::msg("linear v2: bad version"));
    }
    let newest_timestamp = u64::from_be_bytes(buf[9..17].try_into().unwrap());
    let grid_size = buf[17];
    let region_x = i32::from_be_bytes(buf[18..22].try_into().unwrap());
    let region_z = i32::from_be_bytes(buf[22..26].try_into().unwrap());
    if !VALID_V2_GRIDS.contains(&grid_size) {
        return Err(Error::msg(format!("linear v2: bad grid_size {grid_size}")));
    }
    buf = &buf[26..];

    if buf.len() < 128 {
        return Err(Error::msg("linear v2: truncated bitmap"));
    }
    buf = &buf[128..]; // existence bitmap (unreliable; ignore)

    // NBT features dict
    loop {
        if buf.is_empty() {
            return Err(Error::msg("linear v2: truncated features"));
        }
        let key_len = buf[0] as usize;
        buf = &buf[1..];
        if key_len == 0 {
            break;
        }
        if buf.len() < key_len + 4 {
            return Err(Error::msg("linear v2: truncated feature entry"));
        }
        buf = &buf[key_len + 4..];
    }

    let bucket_count = (grid_size as usize) * (grid_size as usize);
    let mut bucket_sizes = Vec::with_capacity(bucket_count);
    let mut bucket_hashes = Vec::with_capacity(bucket_count);
    for _ in 0..bucket_count {
        if buf.len() < 13 {
            return Err(Error::msg("linear v2: truncated bucket meta"));
        }
        let size = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
        let _level = buf[4] as i8;
        let hash = u64::from_be_bytes(buf[5..13].try_into().unwrap());
        buf = &buf[13..];
        bucket_sizes.push(size);
        bucket_hashes.push(hash);
    }

    let total_compressed: usize = bucket_sizes.iter().sum();
    if buf.len() < total_compressed + 8 {
        return Err(Error::msg("linear v2: truncated buckets/footer"));
    }
    let footer = u64::from_be_bytes(buf[total_compressed..total_compressed + 8].try_into().unwrap());
    if footer != LINEAR_SIGNATURE {
        return Err(Error::msg("linear v2: bad footer signature"));
    }

    let mut chunks = vec![None; CHUNK_COUNT];
    let mut timestamps = vec![0u32; CHUNK_COUNT];
    let mut cursor = buf;
    for (bucket_idx, &size) in bucket_sizes.iter().enumerate() {
        let compressed = &cursor[..size];
        let actual = xxh64(compressed, 0);
        if actual != bucket_hashes[bucket_idx] {
            return Err(Error::msg(format!(
                "linear v2: xxhash mismatch bucket {bucket_idx}"
            )));
        }
        let decompressed = decompress_zstd_ruzstd_or_zstd(compressed)?;
        cursor = &cursor[size..];

        let mut b = bytes::Bytes::from(decompressed);
        let cpb = CHUNK_COUNT / bucket_count;
        for local in 0..cpb {
            if b.remaining() < 12 {
                return Err(Error::msg("linear v2: truncated chunk record"));
            }
            let chunk_size = b.get_u32() as usize;
            let ts = b.get_u64();
            let chunk_index = global_chunk_index(bucket_idx, local, grid_size);
            timestamps[chunk_index] = ts as u32;
            if chunk_size == 0 {
                continue;
            }
            if b.remaining() < chunk_size {
                return Err(Error::msg("linear v2: truncated chunk data"));
            }
            let data = b.split_to(chunk_size);
            chunks[chunk_index] = Some(data.to_vec());
        }
    }

    Ok(LinearRegion {
        version: LinearVersion::V2,
        region_x,
        region_z,
        newest_timestamp,
        chunks,
        timestamps,
        grid_size,
    })
}

pub fn write_linear_v2(region: &LinearRegion) -> Result<Vec<u8>> {
    let grid_size = if VALID_V2_GRIDS.contains(&region.grid_size) {
        region.grid_size
    } else {
        DEFAULT_V2_GRID
    };
    let bucket_count = (grid_size as usize) * (grid_size as usize);
    let cpb = CHUNK_COUNT / bucket_count;

    let mut compressed_buckets = Vec::with_capacity(bucket_count);
    let mut bucket_meta = Vec::with_capacity(bucket_count * 13);
    for bucket_idx in 0..bucket_count {
        let mut raw = Vec::new();
        for local in 0..cpb {
            let idx = global_chunk_index(bucket_idx, local, grid_size);
            match &region.chunks[idx] {
                None => {
                    raw.put_u32(0);
                    raw.put_u64(region.timestamps[idx] as u64);
                }
                Some(data) => {
                    raw.put_u32(data.len() as u32);
                    raw.put_u64(region.timestamps[idx] as u64);
                    raw.extend_from_slice(data);
                }
            }
        }
        let compressed = zstd::encode_all(raw.as_slice(), 1)
            .map_err(|e| Error::msg(format!("linear v2: zstd: {e}")))?;
        let hash = xxh64(&compressed, 0);
        bucket_meta.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        bucket_meta.push(1); // compression level
        bucket_meta.extend_from_slice(&hash.to_be_bytes());
        compressed_buckets.push(compressed);
    }

    let mut bitmap = [0u8; 128];
    for (i, c) in region.chunks.iter().enumerate() {
        if c.is_some() {
            bitmap[i / 8] |= 1 << (i % 8);
        }
    }

    let mut header = Vec::new();
    header.extend_from_slice(&LINEAR_SIGNATURE.to_be_bytes());
    header.push(2);
    header.extend_from_slice(&region.newest_timestamp.to_be_bytes());
    header.push(grid_size);
    header.extend_from_slice(&region.region_x.to_be_bytes());
    header.extend_from_slice(&region.region_z.to_be_bytes());
    header.extend_from_slice(&bitmap);
    header.push(0); // empty NBT features
    header.extend_from_slice(&bucket_meta);

    let mut out = header;
    for b in compressed_buckets {
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&LINEAR_SIGNATURE.to_be_bytes());
    Ok(out)
}

fn global_chunk_index(bucket_idx: usize, local: usize, grid_size: u8) -> usize {
    let stride = 32 / grid_size as usize;
    let bucket_row = bucket_idx / grid_size as usize;
    let bucket_col = bucket_idx % grid_size as usize;
    let local_row = local / stride;
    let local_col = local % stride;
    let cz = bucket_row * stride + local_row;
    let cx = bucket_col * stride + local_col;
    cz * 32 + cx
}

fn decompress_zstd_ruzstd_or_zstd(compressed: &[u8]) -> Result<Vec<u8>> {
    // Prefer libzstd for compatibility with pyzstd / Linear v1 tooling.
    zstd::decode_all(compressed).or_else(|_| {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed)
            .map_err(|e| Error::msg(format!("linear: zstd decoder: {e}")))?;
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| Error::msg(format!("linear: zstd read: {e}")))?;
        Ok(out)
    })
}

/// Convert a Linear region into Anvil `.mca` bytes (zlib-compressed chunks).
pub fn linear_to_mca_bytes(linear: &LinearRegion) -> Result<Vec<u8>> {
    use mca::{Compression, RegionWriter};
    let mut writer = RegionWriter::new();
    for z in 0..32u8 {
        for x in 0..32u8 {
            if let Some(nbt) = linear.get(x, z) {
                writer.set_chunk(x, z, nbt.to_vec(), Compression::default())?;
            }
        }
    }
    let mut out = Vec::new();
    writer.write(&mut out)?;
    Ok(out)
}

/// Build a Linear v1 region from Anvil `.mca` bytes.
pub fn mca_to_linear_v1(mca_bytes: &[u8], region_x: i32, region_z: i32) -> Result<LinearRegion> {
    use mca::RegionReader;
    let mut region = LinearRegion::empty(region_x, region_z);
    region.version = LinearVersion::V1;
    if mca_bytes.len() < 8192 {
        return Ok(region);
    }
    let mut reader = RegionReader::new(mca_bytes)?;
    for z in 0..32u8 {
        for x in 0..32u8 {
            if let Some(raw) = reader.chunk(x, z)? {
                region.set(x, z, Some(raw.to_vec()), 0);
            }
        }
    }
    Ok(region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_v1_roundtrip_empty_and_one_chunk() {
        let mut region = LinearRegion::empty(1, -2);
        region.set(3, 5, Some(b"\x0a\x00hello-chunk-nbt\x00".to_vec()), 42);
        let bytes = write_linear_v1(&region, 3).unwrap();
        assert!(is_linear_file(&bytes));
        let back = read_linear_v1(&bytes).unwrap();
        assert_eq!(back.present_count(), 1);
        assert_eq!(
            back.get(3, 5).unwrap(),
            b"\x0a\x00hello-chunk-nbt\x00".as_slice()
        );
        assert_eq!(back.timestamps[LinearRegion::chunk_index(3, 5)], 42);
    }

    #[test]
    fn linear_v2_roundtrip() {
        let mut region = LinearRegion::empty(0, 0);
        region.version = LinearVersion::V2;
        region.grid_size = 2;
        region.set(0, 0, Some(vec![1, 2, 3, 4, 5],), 7);
        region.set(31, 31, Some(vec![9, 8, 7],), 8);
        let bytes = write_linear_v2(&region).unwrap();
        let back = read_linear_v2(&bytes).unwrap();
        assert_eq!(back.version, LinearVersion::V2);
        assert_eq!(back.get(0, 0).unwrap(), &[1, 2, 3, 4, 5]);
        assert_eq!(back.get(31, 31).unwrap(), &[9, 8, 7]);
        assert!(back.get(1, 0).is_none());
    }

    #[test]
    fn mca_linear_mca_roundtrip() {
        use mca::{Compression, RegionWriter};
        let mut w = RegionWriter::new();
        // Minimal valid-ish NBT compound: empty root tag is not enough for ChunkData,
        // but RegionWriter stores opaque bytes — roundtrip at region layer only.
        let payload = b"\x0a\x00\x00".to_vec(); // TAG_Compound "" end
        w.set_chunk(1, 2, payload.clone(), Compression::default())
            .unwrap();
        let mut mca = Vec::new();
        w.write(&mut mca).unwrap();

        let linear = mca_to_linear_v1(&mca, 0, 0).unwrap();
        assert_eq!(linear.present_count(), 1);
        let mca2 = linear_to_mca_bytes(&linear).unwrap();
        let back = mca_to_linear_v1(&mca2, 0, 0).unwrap();
        assert_eq!(back.get(1, 2).unwrap(), payload.as_slice());
    }
}
