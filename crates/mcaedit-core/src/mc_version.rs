//! Minecraft release ↔ DataVersion mapping for world / chunk / level.dat.

/// Default target when creating new worlds (Java Edition 26.2).
pub const DEFAULT_MC: &str = "26.2";
pub const DEFAULT_DATA_VERSION: i32 = 4903;
/// Anvil level format id (TAG `version` inside Data).
pub const ANVIL_LEVEL_VERSION: i32 = 19133;

/// Known release aliases MCAEdit can target when creating worlds.
/// Exact integers are Java Edition release DataVersions (minecraft.wiki).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McRelease {
    pub alias: &'static str,
    pub name: &'static str,
    pub data_version: i32,
}

pub const KNOWN_RELEASES: &[McRelease] = &[
    McRelease {
        alias: "26.2",
        name: "26.2",
        data_version: 4903,
    },
    McRelease {
        alias: "1.21.11",
        name: "1.21.11",
        data_version: 4671,
    },
    McRelease {
        alias: "1.21.10",
        name: "1.21.10",
        data_version: 4556,
    },
    McRelease {
        alias: "1.21.4",
        name: "1.21.4",
        data_version: 4189,
    },
    McRelease {
        alias: "1.21.1",
        name: "1.21.1",
        data_version: 3955,
    },
    McRelease {
        alias: "1.21",
        name: "1.21",
        data_version: 3738,
    },
    McRelease {
        alias: "1.20.4",
        name: "1.20.4",
        data_version: 3700,
    },
    McRelease {
        alias: "1.20.1",
        name: "1.20.1",
        data_version: 3465,
    },
    McRelease {
        alias: "1.20",
        name: "1.20",
        data_version: 3463,
    },
    McRelease {
        alias: "1.19.4",
        name: "1.19.4",
        data_version: 3337,
    },
    McRelease {
        alias: "1.18.2",
        name: "1.18.2",
        data_version: 2975,
    },
];

/// From 1.21.9 onward, Mojang moved several fields out of `level.dat` into
/// `data/minecraft/*.dat` (world_gen_settings, game_rules, …).
pub const MODERN_LEVEL_DAT_MIN: i32 = 4435;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub name: String,
    pub data_version: i32,
    pub modern_layout: bool,
}

impl ResolvedVersion {
    pub fn default_latest() -> Self {
        Self::from_mc(DEFAULT_MC).expect("default mc alias")
    }

    pub fn from_data_version(data_version: i32) -> Self {
        let name = KNOWN_RELEASES
            .iter()
            .find(|r| r.data_version == data_version)
            .map(|r| r.name.to_string())
            .unwrap_or_else(|| format!("data-{data_version}"));
        Self {
            name,
            data_version,
            modern_layout: data_version >= MODERN_LEVEL_DAT_MIN,
        }
    }

    /// Parse `--mc 26.2|1.21.4|…` or a raw DataVersion integer string.
    pub fn from_mc(spec: &str) -> crate::error::Result<Self> {
        let spec = spec.trim();
        if let Ok(dv) = spec.parse::<i32>() {
            return Ok(Self::from_data_version(dv));
        }
        let lower = spec.to_ascii_lowercase();
        for r in KNOWN_RELEASES {
            if r.alias.eq_ignore_ascii_case(&lower) || r.name.eq_ignore_ascii_case(&lower) {
                return Ok(Self {
                    name: r.name.to_string(),
                    data_version: r.data_version,
                    modern_layout: r.data_version >= MODERN_LEVEL_DAT_MIN,
                });
            }
        }
        Err(crate::error::Error::msg(format!(
            "unknown --mc `{spec}`; known: {} (or raw DataVersion int)",
            KNOWN_RELEASES
                .iter()
                .map(|r| r.alias)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    pub fn resolve(mc: Option<&str>, data_version: Option<i32>) -> crate::error::Result<Self> {
        match (mc, data_version) {
            (Some(mc), Some(dv)) => {
                let mut v = Self::from_mc(mc)?;
                v.data_version = dv;
                v.modern_layout = dv >= MODERN_LEVEL_DAT_MIN;
                Ok(v)
            }
            (Some(mc), None) => Self::from_mc(mc),
            (None, Some(dv)) => Ok(Self::from_data_version(dv)),
            (None, None) => Ok(Self::default_latest()),
        }
    }
}

/// Human-readable list for docs / `--help`.
pub fn supported_mc_summary() -> String {
    KNOWN_RELEASES
        .iter()
        .map(|r| format!("{}={}", r.alias, r.data_version))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_to_26_2() {
        let v = ResolvedVersion::resolve(None, None).unwrap();
        assert_eq!(v.data_version, 4903);
        assert_eq!(v.name, "26.2");
        assert!(v.modern_layout);
    }

    #[test]
    fn resolve_old_anvil() {
        let v = ResolvedVersion::from_mc("1.18.2").unwrap();
        assert_eq!(v.data_version, 2975);
        assert!(!v.modern_layout);
    }

    #[test]
    fn resolve_raw_int() {
        let v = ResolvedVersion::from_mc("3465").unwrap();
        assert_eq!(v.data_version, 3465);
        assert_eq!(v.name, "1.20.1");
    }
}
