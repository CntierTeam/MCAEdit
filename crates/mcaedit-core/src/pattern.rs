//! Weighted / percent block patterns for fill, brush, replace-with.
//!
//! Examples:
//! - `minecraft:stone` (100%)
//! - `50%stone,50%dirt`
//! - `50%minecraft:stone,50%minecraft:dirt`
//! - `3*stone,1*dirt`

use crate::blockstate::BlockState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    entries: Vec<(BlockState, u32)>,
    total: u32,
}

impl Pattern {
    pub fn single(block: BlockState) -> Self {
        Self {
            entries: vec![(block, 1)],
            total: 1,
        }
    }

    pub fn entries(&self) -> &[(BlockState, u32)] {
        &self.entries
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("empty pattern".into());
        }
        // Single block (no comma / % / *) — keep simple path.
        if !input.contains(',') && !input.contains('%') && !input.contains('*') {
            return Ok(Self::single(parse_block_token(input)?));
        }

        let mut entries = Vec::new();
        for part in input.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (weight, block_s) = if let Some((w, rest)) = part.split_once('%') {
                let w: u32 = w
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad percent weight in `{part}`"))?;
                if w == 0 {
                    return Err(format!("zero weight in `{part}`"));
                }
                (w, rest.trim())
            } else if let Some((w, rest)) = part.split_once('*') {
                let w: u32 = w
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad star weight in `{part}`"))?;
                if w == 0 {
                    return Err(format!("zero weight in `{part}`"));
                }
                (w, rest.trim())
            } else {
                (1u32, part)
            };
            entries.push((parse_block_token(block_s)?, weight));
        }
        if entries.is_empty() {
            return Err("pattern has no entries".into());
        }
        let total: u32 = entries.iter().map(|(_, w)| *w).sum();
        if total == 0 {
            return Err("pattern total weight is 0".into());
        }
        Ok(Self { entries, total })
    }

    /// Deterministic pick from world coords (stable across runs).
    pub fn pick_at(&self, x: i32, y: i32, z: i32) -> &BlockState {
        if self.entries.len() == 1 {
            return &self.entries[0].0;
        }
        let mut slot = hash_coords(x, y, z) % self.total as u64;
        for (block, w) in &self.entries {
            if slot < *w as u64 {
                return block;
            }
            slot -= *w as u64;
        }
        &self.entries.last().unwrap().0
    }

    pub fn describe(&self) -> String {
        if self.entries.len() == 1 {
            return self.entries[0].0.to_compact();
        }
        self.entries
            .iter()
            .map(|(b, w)| {
                let pct = (*w as u64 * 100) / self.total as u64;
                format!("{pct}%{}", b.to_compact())
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn parse_block_token(s: &str) -> Result<BlockState, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("empty block in pattern".into());
    }
    let with_ns = if t.contains(':') {
        t.to_string()
    } else {
        format!("minecraft:{t}")
    };
    BlockState::parse(&with_ns)
}

fn hash_coords(x: i32, y: i32, z: i32) -> u64 {
    // SplitMix64-ish mix of coords (deterministic, not crypto).
    let mut n = (x as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add((y as u64).wrapping_mul(0xC2B2AE3D27D4EB4F))
        .wrapping_add((z as u64).wrapping_mul(0x165667B19E3779F9))
        .wrapping_add(0x85EBCA77C2B2AE63);
    n = (n ^ (n >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94D049BB133111EB);
    n ^ (n >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_percent_and_single() {
        let p = Pattern::parse("50%stone,50%dirt").unwrap();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.total, 100);
        let s = Pattern::parse("minecraft:glass").unwrap();
        assert_eq!(s.entries.len(), 1);
    }

    #[test]
    fn pick_is_deterministic() {
        let p = Pattern::parse("50%stone,50%dirt").unwrap();
        let a = p.pick_at(1, 2, 3).clone();
        let b = p.pick_at(1, 2, 3).clone();
        assert_eq!(a, b);
    }
}
