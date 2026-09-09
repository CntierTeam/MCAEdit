//! Offline random-tick approximations + scheduled-tick NBT stepping.
//!
//! Full vanilla behaviour (light, hydration, biome temperature, neighbour
//! updates, …) requires a live server. This module advances crop ages,
//! grows sugar cane / cactus / bamboo upward, spreads grass/mycelium onto
//! dirt, and decrements chunk `block_ticks` / `fluid_ticks` queues in NBT.
//!
//! Note: we intentionally avoid a full terrain-bridge chunk round-trip here —
//! rewriting through the vendored world crate can drop blocks that are not
//! fully representable in its in-memory chunk model.

use crate::action::BlockChange;
use crate::blockstate::BlockState;
use crate::chunk::ChunkData;
use crate::error::Result;
use crate::world::WorldView;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct OfflineTickStats {
    pub chunks: usize,
    pub samples: usize,
    pub candidates: usize,
    pub mutations: usize,
    pub due_block: usize,
    pub due_fluid: usize,
}

/// Plan offline random-tick mutations over a chunk AABB.
#[allow(clippy::too_many_arguments)]
pub fn plan_offline_random_ticks(
    world: &mut WorldView<'_>,
    from_cx: i32,
    from_cz: i32,
    to_cx: i32,
    to_cz: i32,
    rounds: u32,
    random_tick_speed: u32,
) -> Result<(Vec<BlockChange>, OfflineTickStats)> {
    let (min_cx, max_cx) = (from_cx.min(to_cx), from_cx.max(to_cx));
    let (min_cz, max_cz) = (from_cz.min(to_cz), from_cz.max(to_cz));
    let rounds = rounds.max(1);
    let speed = random_tick_speed.max(1);

    let mut dirty: BTreeMap<(i32, i32), ChunkData> = BTreeMap::new();
    let mut changes: BTreeMap<(i32, i32, i32), BlockChange> = BTreeMap::new();
    let mut stats = OfflineTickStats::default();

    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            if let std::collections::btree_map::Entry::Vacant(e) = dirty.entry((cx, cz)) {
                e.insert(world.load_chunk(cx, cz)?);
            }
            stats.chunks += 1;

            // Step scheduled tick queues in-place on the loaded chunk NBT.
            let (db, df) = step_scheduled_ticks(dirty.get_mut(&(cx, cz)).unwrap(), rounds);
            stats.due_block += db;
            stats.due_fluid += df;

            let mut section_ys = dirty.get(&(cx, cz)).unwrap().section_ys();
            if section_ys.is_empty() {
                section_ys = vec![3, 4, 5];
            }

            for _ in 0..rounds {
                for &sy in &section_ys {
                    let y_base = (sy as i32) * 16;
                    for _ in 0..speed {
                        let r = fastrand_u32();
                        let lx = (r & 0xF) as i32;
                        let ly = ((r >> 4) & 0xF) as i32;
                        let lz = ((r >> 8) & 0xF) as i32;
                        let x = (cx << 4) + lx;
                        let y = y_base + ly;
                        let z = (cz << 4) + lz;
                        stats.samples += 1;

                        let before = block_at(&dirty, &changes, x, y, z)?;
                        if !is_random_tickable(&before) {
                            continue;
                        }
                        stats.candidates += 1;

                        if let Some(extra) =
                            grow_at(world, &mut dirty, &mut changes, x, y, z, &before)?
                        {
                            for ch in extra {
                                let key = (ch.x, ch.y, ch.z);
                                let ck = (ch.x >> 4, ch.z >> 4);
                                if let std::collections::btree_map::Entry::Vacant(e) =
                                    dirty.entry(ck)
                                {
                                    e.insert(world.load_chunk(ck.0, ck.1)?);
                                }
                                let _ = dirty.get_mut(&ck).unwrap().set_block(
                                    ch.x,
                                    ch.y,
                                    ch.z,
                                    ch.after.clone(),
                                );
                                changes.insert(key, ch);
                                stats.mutations += 1;
                            }
                        }
                    }
                }
            }

            // Persist scheduled-tick NBT updates even when no block mutations.
            world.save_chunk(dirty.get(&(cx, cz)).unwrap())?;
        }
    }

    Ok((changes.into_values().collect(), stats))
}

/// Decrement `block_ticks` / `fluid_ticks` wait counters for `rounds` steps.
/// Entries that reach wait<=0 are removed (due). Full block behaviour for due
/// ticks is not simulated offline.
fn step_scheduled_ticks(chunk: &mut ChunkData, rounds: u32) -> (usize, usize) {
    let mut due_block = 0usize;
    let mut due_fluid = 0usize;
    for _ in 0..rounds {
        due_block += step_tick_list(chunk, "block_ticks");
        due_fluid += step_tick_list(chunk, "fluid_ticks");
        // Also accept legacy / alternate keys if present.
        due_block += step_tick_list(chunk, "TileTicks");
        due_fluid += step_tick_list(chunk, "FluidTicks");
    }
    (due_block, due_fluid)
}

fn step_tick_list(chunk: &mut ChunkData, key: &str) -> usize {
    let Some(list) = chunk.root.get_mut(key).and_then(|v| v.as_array_mut()) else {
        return 0;
    };
    let mut due = 0usize;
    let mut kept = Vec::with_capacity(list.len());
    for entry in list.drain(..) {
        let mut wait = entry
            .get("t")
            .or_else(|| entry.get("Ticks"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        wait -= 1;
        if wait < 0 {
            due += 1;
            continue;
        }
        let mut e = entry;
        if let Some(obj) = e.as_object_mut() {
            if obj.contains_key("t") {
                obj.insert("t".into(), JsonValue::from(wait as i32));
            } else if obj.contains_key("Ticks") {
                obj.insert("Ticks".into(), JsonValue::from(wait as i32));
            } else {
                obj.insert("t".into(), JsonValue::from(wait as i32));
            }
        }
        kept.push(e);
    }
    *list = kept;
    due
}

pub fn is_random_tickable(block: &BlockState) -> bool {
    matches!(
        strip_ns(&block.name),
        "wheat"
            | "carrots"
            | "potatoes"
            | "beetroots"
            | "nether_wart"
            | "sweet_berry_bush"
            | "cocoa"
            | "sugar_cane"
            | "cactus"
            | "kelp"
            | "bamboo"
            | "bamboo_sapling"
            | "melon_stem"
            | "pumpkin_stem"
            | "grass_block"
            | "mycelium"
            | "farmland"
            | "chorus_flower"
    )
}

fn strip_ns(name: &str) -> &str {
    name.rsplit_once(':').map(|(_, n)| n).unwrap_or(name)
}

fn age_max(name: &str) -> Option<u8> {
    match strip_ns(name) {
        "wheat" | "carrots" | "potatoes" | "melon_stem" | "pumpkin_stem" => Some(7),
        "beetroots" | "nether_wart" | "sweet_berry_bush" => Some(3),
        "cocoa" => Some(2),
        "kelp" => Some(25),
        "chorus_flower" => Some(5),
        _ => None,
    }
}

fn block_at(
    dirty: &BTreeMap<(i32, i32), ChunkData>,
    changes: &BTreeMap<(i32, i32, i32), BlockChange>,
    x: i32,
    y: i32,
    z: i32,
) -> Result<BlockState> {
    if let Some(c) = changes.get(&(x, y, z)) {
        return Ok(c.after.clone());
    }
    dirty
        .get(&(x >> 4, z >> 4))
        .ok_or_else(|| crate::error::Error::msg("chunk missing"))?
        .get_block(x, y, z)
}

fn peek(
    world: &WorldView<'_>,
    dirty: &BTreeMap<(i32, i32), ChunkData>,
    changes: &BTreeMap<(i32, i32, i32), BlockChange>,
    x: i32,
    y: i32,
    z: i32,
) -> Result<BlockState> {
    if let Some(c) = changes.get(&(x, y, z)) {
        return Ok(c.after.clone());
    }
    if let Some(ch) = dirty.get(&(x >> 4, z >> 4)) {
        return ch.get_block(x, y, z);
    }
    world.get_block(x, y, z)
}

fn grow_at(
    world: &WorldView<'_>,
    dirty: &mut BTreeMap<(i32, i32), ChunkData>,
    changes: &mut BTreeMap<(i32, i32, i32), BlockChange>,
    x: i32,
    y: i32,
    z: i32,
    before: &BlockState,
) -> Result<Option<Vec<BlockChange>>> {
    let name = strip_ns(&before.name);

    if let Some(max_age) = age_max(&before.name) {
        let age = prop_u8(before, "age").unwrap_or(0);
        if age < max_age && (fastrand_u32() % 2) == 0 {
            let mut after = before.clone();
            after
                .properties
                .insert("age".into(), (age + 1).to_string());
            return Ok(Some(vec![BlockChange {
                x,
                y,
                z,
                before: before.clone(),
                after,
            }]));
        }
        return Ok(None);
    }

    match name {
        "sugar_cane" | "cactus" | "bamboo" => {
            let above = peek(world, dirty, changes, x, y + 1, z)?;
            if !above.is_air_like() {
                return Ok(None);
            }
            let mut h = 1i32;
            let mut yy = y - 1;
            while h < 3 {
                let b = peek(world, dirty, changes, x, yy, z)?;
                if strip_ns(&b.name) != name {
                    break;
                }
                h += 1;
                yy -= 1;
            }
            if h >= 3 || (fastrand_u32() % 3) != 0 {
                return Ok(None);
            }
            let place = BlockState::parse(&format!("minecraft:{name}")).unwrap();
            Ok(Some(vec![BlockChange {
                x,
                y: y + 1,
                z,
                before: above,
                after: place,
            }]))
        }
        "bamboo_sapling" => {
            let above = peek(world, dirty, changes, x, y + 1, z)?;
            if above.is_air_like() && (fastrand_u32() % 3) == 0 {
                return Ok(Some(vec![
                    BlockChange {
                        x,
                        y,
                        z,
                        before: before.clone(),
                        after: BlockState::parse("minecraft:bamboo").unwrap(),
                    },
                    BlockChange {
                        x,
                        y: y + 1,
                        z,
                        before: above,
                        after: BlockState::parse("minecraft:bamboo").unwrap(),
                    },
                ]));
            }
            Ok(None)
        }
        "grass_block" | "mycelium" => {
            let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            let (dx, dz) = dirs[(fastrand_u32() as usize) % 4];
            let tx = x + dx;
            let tz = z + dz;
            let target = peek(world, dirty, changes, tx, y, tz)?;
            if strip_ns(&target.name) == "dirt" && (fastrand_u32() % 3) == 0 {
                let ck = (tx >> 4, tz >> 4);
                if let std::collections::btree_map::Entry::Vacant(e) = dirty.entry(ck) {
                    e.insert(world.load_chunk(ck.0, ck.1)?);
                }
                return Ok(Some(vec![BlockChange {
                    x: tx,
                    y,
                    z: tz,
                    before: target,
                    after: before.clone(),
                }]));
            }
            Ok(None)
        }
        "farmland" => {
            let moisture = prop_u8(before, "moisture").unwrap_or(0);
            if moisture > 0 && (fastrand_u32() % 4) == 0 {
                let mut after = before.clone();
                after
                    .properties
                    .insert("moisture".into(), (moisture - 1).to_string());
                return Ok(Some(vec![BlockChange {
                    x,
                    y,
                    z,
                    before: before.clone(),
                    after,
                }]));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn prop_u8(block: &BlockState, key: &str) -> Option<u8> {
    block.properties.get(key)?.parse().ok()
}

fn fastrand_u32() -> u32 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = const { Cell::new(0x9e37_79b9_7f4a_7c15u64) };
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

/// Deterministic helper for tests: force-advance crop age by `steps` (clamped).
pub fn force_age_steps(block: &BlockState, steps: u8) -> Option<BlockState> {
    let max = age_max(&block.name)?;
    let age = prop_u8(block, "age").unwrap_or(0);
    let next = (age.saturating_add(steps)).min(max);
    if next == age {
        return None;
    }
    let mut after = block.clone();
    after.properties.insert("age".into(), next.to_string());
    Some(after)
}
