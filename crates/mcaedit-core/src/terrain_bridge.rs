//! Bridge to vendored `pumpkin-world` for terrain gen, lighting, and tick participation.
//!
//! MCAEdit is GPL-3.0 because it links this GPL code.

use crate::error::{Error, Result};
use crate::region::{region_coords, region_file_name, RegionStore};
use crate::world::WorldView;
use bytes::Bytes;
use pumpkin_config::lighting::LightingEngineConfig;
use pumpkin_data::block_properties::has_random_ticks;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::{Block, BlockState, BlockStateId, Mirror, Rotation};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::world_seed::Seed;
use pumpkin_world::chunk::format::anvil::SingleChunkDataSerializer;
use pumpkin_world::chunk::palette::has_random_ticking_fluid;
use pumpkin_world::chunk::{ChunkData as PumpkinChunk, ChunkSections};
use pumpkin_world::chunk_system::{generate_single_chunk, Cache, Chunk, StagedChunkEnum};
use pumpkin_world::generation::get_world_gen;
use pumpkin_world::generation::proto_chunk::GenerationCache;
use pumpkin_world::lighting::LightEngine;
use pumpkin_world::world::WorldPortalExt;
use std::path::Path;
use std::sync::atomic::Ordering;

pub struct PortalStub;

impl WorldPortalExt for PortalStub {
    fn can_place_at(
        &self,
        _block: &Block,
        _state: &BlockState,
        _block_accessor: &dyn pumpkin_world::world::BlockAccessor,
        _block_pos: &BlockPos,
    ) -> bool {
        true
    }

    fn mirror(&self, block: &Block, state_id: BlockStateId, mirror: Mirror) -> &'static BlockState {
        block.mirror(state_id, mirror)
    }

    fn rotate(
        &self,
        block: &Block,
        state_id: BlockStateId,
        rotation: Rotation,
    ) -> &'static BlockState {
        block.rotate(state_id, rotation)
    }

    fn spawn_mobs_for_chunk_generation(
        &self,
        _cache: &mut dyn GenerationCache,
        _biome: &'static pumpkin_data::chunk::Biome,
        _chunk_x: i32,
        _chunk_z: i32,
    ) {
    }
}

pub fn parse_dimension(name: &str) -> Result<&'static Dimension> {
    match name.trim().to_ascii_lowercase().as_str() {
        "overworld" | "minecraft:overworld" => Ok(&Dimension::OVERWORLD),
        "nether" | "the_nether" | "minecraft:the_nether" => Ok(&Dimension::THE_NETHER),
        "end" | "the_end" | "minecraft:the_end" => Ok(&Dimension::THE_END),
        other => Err(Error::msg(format!("unknown dimension `{other}`"))),
    }
}

fn write_pumpkin_chunk(region_dir: &Path, chunk: &PumpkinChunk) -> Result<()> {
    let (rx, rz) = region_coords(chunk.x, chunk.z);
    let path = region_dir.join(region_file_name(rx, rz));
    let mut store = RegionStore::open(path);
    let nbt = chunk
        .to_bytes()
        .map_err(|e| Error::msg(format!("pumpkin chunk serialize: {e}")))?
        .to_vec();
    store.write_raw_chunk_nbt(chunk.x, chunk.z, nbt)?;
    Ok(())
}

fn read_pumpkin_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<PumpkinChunk>> {
    let (rx, rz) = region_coords(cx, cz);
    let path = region_dir.join(region_file_name(rx, rz));
    let store = RegionStore::open(path);
    let Some(raw) = store.read_raw_chunk_nbt(cx, cz)? else {
        return Ok(None);
    };
    let chunk = PumpkinChunk::from_bytes(&Bytes::from(raw), Vector2::new(cx, cz))
        .map_err(|e| Error::msg(format!("pumpkin chunk parse ({cx},{cz}): {e}")))?;
    Ok(Some(chunk))
}

/// Generate vanilla-compatible terrain into the session work `region/`.
pub fn generate_chunks(
    world: &mut WorldView<'_>,
    seed: u64,
    dim: &str,
    from_cx: i32,
    from_cz: i32,
    to_cx: i32,
    to_cz: i32,
) -> Result<Vec<String>> {
    let dimension = parse_dimension(dim)?;
    let gen = get_world_gen(Seed(seed), dimension.clone(), false, Vec::new(), String::new());
    let registry = PortalStub;
    let region_dir = world.session.work_region_dir();
    std::fs::create_dir_all(&region_dir)?;

    let (min_cx, max_cx) = (from_cx.min(to_cx), from_cx.max(to_cx));
    let (min_cz, max_cz) = (from_cz.min(to_cz), from_cz.max(to_cz));
    let mut lines = Vec::new();
    let mut n = 0usize;
    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            let chunk = generate_single_chunk(&gen, &registry, cx, cz, StagedChunkEnum::Full);
            let Chunk::Level(data) = chunk else {
                return Err(Error::msg(format!(
                    "generate ({cx},{cz}): expected Level chunk"
                )));
            };
            write_pumpkin_chunk(&region_dir, &data)?;
            n += 1;
            lines.push(format!("gen chunk={cx},{cz}"));
        }
    }
    world.session.mark_dirty()?;
    lines.push(format!(
        "generated={n} seed={seed} dim={} range=({min_cx},{min_cz})..({max_cx},{max_cz})",
        dim
    ));
    Ok(lines)
}

fn copy_level_into_proto(
    level: &PumpkinChunk,
    gen: &pumpkin_world::generation::generator::WorldGenerator,
) -> pumpkin_world::ProtoChunk {
    let mut proto = pumpkin_world::ProtoChunk::new(level.x, level.z, gen);
    let min_y = level.section.min_y;
    let height = (level.section.count as i32) * 16;
    let base_x = level.x * 16;
    let base_z = level.z * 16;
    for y in min_y..(min_y + height) {
        for z in 0..16usize {
            for x in 0..16usize {
                let id = level
                    .section
                    .get_block_absolute_y(x, y, z)
                    .unwrap_or(Block::AIR.default_state.id);
                if id == Block::AIR.default_state.id {
                    continue;
                }
                let state = BlockState::from_id(id);
                proto.set_block_state(base_x + x as i32, y, base_z + z as i32, state);
            }
        }
    }
    // Force lighting stage to run (initialize_light skips stage >= Lighting).
    proto.stage = StagedChunkEnum::Features;
    proto
}

/// Recalculate sky/block light for chunk AABB.
pub fn fix_light(
    world: &mut WorldView<'_>,
    from_cx: i32,
    from_cz: i32,
    to_cx: i32,
    to_cz: i32,
    seed: u64,
    dim: &str,
) -> Result<Vec<String>> {
    let dimension = parse_dimension(dim)?;
    let gen = get_world_gen(Seed(seed), dimension.clone(), false, Vec::new(), String::new());
    let region_dir = world.session.work_region_dir();

    let (min_cx, max_cx) = (from_cx.min(to_cx), from_cx.max(to_cx));
    let (min_cz, max_cz) = (from_cz.min(to_cz), from_cz.max(to_cz));
    let mut lines = Vec::new();
    let mut n = 0usize;
    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            let radius = 1;
            let mut local = Cache::new(cx - radius, cz - radius, radius * 2 + 1);
            for ddz in -radius..=radius {
                for ddx in -radius..=radius {
                    let nx = cx + ddx;
                    let nz = cz + ddz;
                    if let Some(level) = read_pumpkin_chunk(&region_dir, nx, nz)? {
                        let proto = copy_level_into_proto(&level, &gen);
                        local.chunks.push(Chunk::Proto(Box::new(proto)));
                    } else {
                        let mut proto = pumpkin_world::ProtoChunk::new(nx, nz, &gen);
                        proto.stage = StagedChunkEnum::Features;
                        local.chunks.push(Chunk::Proto(Box::new(proto)));
                    }
                }
            }
            let mut engine = LightEngine::new();
            engine.initialize_light(&mut local, &LightingEngineConfig::Default);
            // Mid index is center
            let mid = ((local.size * local.size) >> 1) as usize;
            local.chunks[mid].upgrade_to_level_chunk(dimension, &LightingEngineConfig::Default);
            let Chunk::Level(data) = &local.chunks[mid] else {
                return Err(Error::msg("fix-light: upgrade failed"));
            };
            write_pumpkin_chunk(&region_dir, data)?;
            n += 1;
            lines.push(format!("fix-light chunk={cx},{cz}"));
        }
    }
    world.session.mark_dirty()?;
    lines.push(format!(
        "fix_light={n} range=({min_cx},{min_cz})..({max_cx},{max_cz})"
    ));
    Ok(lines)
}

/// Rebuild random-tick participation masks and advance scheduled block/fluid ticks
/// via the vendored world crate (may be lossy on minimal MCAEdit-authored chunks).
///
/// Prefer [`crate::world::WorldView::tick_offline`] / `edit tick` for safe offline
/// growth + NBT scheduled-tick stepping.
///
/// Full block random-tick *behaviour* lives in the server crate; this path:
/// 1. rebuilds `randomly_ticking_mask` / section caches
/// 2. samples random-tick candidates (`--rounds`)
/// 3. steps scheduled tick queues (`block_ticks` / `fluid_ticks`) for `--rounds`
pub fn tick_participate(
    world: &mut WorldView<'_>,
    from_cx: i32,
    from_cz: i32,
    to_cx: i32,
    to_cz: i32,
    rounds: u32,
    random_tick_speed: u32,
) -> Result<Vec<String>> {
    let region_dir = world.session.work_region_dir();
    let (min_cx, max_cx) = (from_cx.min(to_cx), from_cx.max(to_cx));
    let (min_cz, max_cz) = (from_cz.min(to_cz), from_cz.max(to_cz));
    let rounds = rounds.max(1);
    let speed = random_tick_speed.max(1);

    let mut lines = Vec::new();
    let mut samples = 0usize;
    let mut due_block = 0usize;
    let mut due_fluid = 0usize;
    let mut chunks_n = 0usize;

    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            let Some(chunk) = read_pumpkin_chunk(&region_dir, cx, cz)? else {
                continue;
            };
            chunks_n += 1;

            // Rebuild random-tick section participation from current blocks.
            {
                let sections = chunk
                    .section
                    .block_sections
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let (cache, mask) = ChunkSections::build_random_tick_sections_cache(&sections);
                drop(sections);
                *chunk
                    .section
                    .random_tick_sections
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = cache;
                chunk
                    .section
                    .randomly_ticking_mask
                    .store(mask, Ordering::Relaxed);
            }

            let section_count = chunk.section.count;
            let min_y = chunk.section.min_y;
            let mask = chunk
                .section
                .randomly_ticking_mask
                .load(std::sync::atomic::Ordering::Relaxed);

            for _ in 0..rounds {
                if mask != 0 {
                    let sections = chunk
                        .section
                        .block_sections
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for i in 0..section_count {
                        if (mask & (1 << i)) == 0 {
                            continue;
                        }
                        let y_base = min_y + (i as i32 * 16);
                        for _ in 0..speed {
                            let r = fastrand_u32();
                            let x_offset = (r & 0xF) as usize;
                            let z_offset = ((r >> 8) & 0xF) as usize;
                            let y_in_section = ((r >> 4) & 0xF) as usize;
                            let block_state_id =
                                sections[i].get(x_offset, y_in_section, z_offset);
                            if has_random_ticks(block_state_id)
                                || has_random_ticking_fluid(block_state_id)
                            {
                                samples += 1;
                            }
                            let _ = y_base; // position reserved for future behaviour hooks
                        }
                    }
                }
                due_block += chunk.block_ticks.step_tick().len();
                due_fluid += chunk.fluid_ticks.step_tick().len();
            }

            chunk.dirty.store(true, Ordering::Relaxed);
            write_pumpkin_chunk(&region_dir, &chunk)?;
            lines.push(format!("tick-participate chunk={cx},{cz}"));
        }
    }

    world.session.mark_dirty()?;
    lines.push(format!(
        "tick_participate chunks={chunks_n} rounds={rounds} speed={speed} random_samples={samples} due_block={due_block} due_fluid={due_fluid}"
    ));
    Ok(lines)
}

fn fastrand_u32() -> u32 {
    // Cheap LCG without new dep; fine for offline sampling.
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = const { Cell::new(0x4d59_5a6b_cde1u64) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x as u32
    })
}
