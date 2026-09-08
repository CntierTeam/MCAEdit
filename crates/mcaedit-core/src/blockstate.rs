use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Named block state as stored in MCA section palettes.
/// Example: `minecraft:oak_log[axis=y]` or `minecraft:stone`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockState {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

impl BlockState {
    pub const AIR: &'static str = "minecraft:air";
    /// Skip-cell sentinel for sparse section diffs (leave existing block).
    pub const VOID_AIR: &'static str = "minecraft:void_air";

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            properties: BTreeMap::new(),
        }
    }

    pub fn air() -> Self {
        Self::new(Self::AIR)
    }

    pub fn void_air() -> Self {
        Self::new(Self::VOID_AIR)
    }

    pub fn is_empty_sentinel(&self) -> bool {
        self.name == Self::VOID_AIR && self.properties.is_empty()
    }

    pub fn is_air_like(&self) -> bool {
        matches!(
            self.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ) && self.properties.is_empty()
    }

    /// Parse `namespace:path[key=value,...]`.
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("empty block state".into());
        }
        if let Some((name, rest)) = input.split_once('[') {
            let rest = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("missing ']' in `{input}`"))?;
            let mut properties = BTreeMap::new();
            if !rest.is_empty() {
                for part in rest.split(',') {
                    let (k, v) = part
                        .split_once('=')
                        .ok_or_else(|| format!("bad property `{part}` in `{input}`"))?;
                    properties.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
            Ok(Self {
                name: name.trim().to_string(),
                properties,
            })
        } else {
            Ok(Self::new(input))
        }
    }

    pub fn to_compact(&self) -> String {
        if self.properties.is_empty() {
            self.name.clone()
        } else {
            let props: Vec<String> = self
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("{}[{}]", self.name, props.join(","))
        }
    }

    /// Short label for LLM slice views.
    pub fn short_label(&self) -> String {
        let base = self
            .name
            .rsplit_once(':')
            .map(|(_, n)| n)
            .unwrap_or(self.name.as_str());
        if self.properties.is_empty() {
            base.to_string()
        } else {
            format!("{base}*")
        }
    }
}

impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_compact())
    }
}

impl From<&str> for BlockState {
    fn from(value: &str) -> Self {
        Self::parse(value).unwrap_or_else(|_| Self::new(value))
    }
}
