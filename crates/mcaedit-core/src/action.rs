use crate::blockstate::BlockState;
use crate::palette::SectionDiff;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockChange {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub before: BlockState,
    pub after: BlockState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BiomeChange {
    /// World coords of biome cell origin (aligned to 4).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpawnedEntity {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub entity: JsonValue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionPayload {
    SetBlocks {
        changes: Vec<BlockChange>,
    },
    SetBiomes {
        changes: Vec<BiomeChange>,
    },
    SetSection {
        cx: i32,
        cy: i32,
        cz: i32,
        before: SectionDiff,
        after: SectionDiff,
    },
    EntitySpawn {
        chunk_x: i32,
        chunk_z: i32,
        entity: JsonValue,
    },
    EntitySet {
        chunk_x: i32,
        chunk_z: i32,
        uuid: String,
        before: JsonValue,
        after: JsonValue,
    },
    EntityRemove {
        chunk_x: i32,
        chunk_z: i32,
        entity: JsonValue,
    },
    /// Template paste: blocks + spawned entities as one history entry.
    PasteTemplate {
        name: String,
        origin: [i32; 3],
        changes: Vec<BlockChange>,
        spawned: Vec<SpawnedEntity>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: u64,
    pub description: String,
    pub payload: ActionPayload,
}

impl Action {
    pub fn changed_count(&self) -> usize {
        match &self.payload {
            ActionPayload::SetBlocks { changes } => changes.len(),
            ActionPayload::SetBiomes { changes } => changes.len(),
            ActionPayload::SetSection { after, .. } => after
                .cells
                .values()
                .filter(|s| !s.is_empty_sentinel())
                .count(),
            ActionPayload::EntitySpawn { .. }
            | ActionPayload::EntitySet { .. }
            | ActionPayload::EntityRemove { .. } => 1,
            ActionPayload::PasteTemplate {
                changes, spawned, ..
            } => changes.len() + spawned.len(),
        }
    }
}
