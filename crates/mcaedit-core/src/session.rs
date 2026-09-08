use crate::error::{Error, Result};
use crate::history::History;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const SESSION_ROOT_NAME: &str = ".mcaedit";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub source_world: PathBuf,
    /// Dimension folder relative to source_world, usually "."
    pub dim: String,
    pub created_at: String,
    pub dirty: bool,
    pub next_action_hint: u64,
    /// Collaboration label (e.g. agent / user name)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub label: Option<String>,
    pub world: PathBuf,
    pub dim: String,
    pub dirty: bool,
    pub history: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct Session {
    pub root: PathBuf,
    pub meta: SessionMeta,
    pub history: History,
}

impl Session {
    pub fn sessions_root(cwd: &Path) -> PathBuf {
        cwd.join(SESSION_ROOT_NAME)
    }

    pub fn path_for(cwd: &Path, id: &str) -> PathBuf {
        Self::sessions_root(cwd).join(id)
    }

    pub fn create(
        cwd: &Path,
        source_world: &Path,
        dim: &str,
        id: Option<String>,
        label: Option<String>,
    ) -> Result<Self> {
        let source_world = source_world
            .canonicalize()
            .map_err(|e| Error::msg(format!("world path: {e}")))?;
        let region_dir = dim_region_dir(&source_world, dim);
        if !region_dir.exists() {
            return Err(Error::msg(format!(
                "region dir missing: {}",
                region_dir.display()
            )));
        }
        let id = id.unwrap_or_else(|| Uuid::new_v4().to_string()[..8].to_string());
        let root = Self::path_for(cwd, &id);
        if root.exists() {
            return Err(Error::msg(format!("session `{id}` already exists")));
        }
        fs::create_dir_all(root.join("world/region"))?;
        fs::create_dir_all(root.join("world/entities"))?;
        fs::create_dir_all(root.join("history"))?;
        let meta = SessionMeta {
            id: id.clone(),
            source_world,
            dim: dim.to_string(),
            created_at: chrono_like_now(),
            dirty: false,
            next_action_hint: 1,
            label,
        };
        fs::write(root.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        fs::write(root.join("HEAD"), "0\n")?;
        let history = History::open(root.join("history"))?;
        Ok(Self {
            root,
            meta,
            history,
        })
    }

    pub fn open(cwd: &Path, id_or_path: &str) -> Result<Self> {
        let root = resolve_session_path(cwd, id_or_path)?;
        let meta: SessionMeta = serde_json::from_str(&fs::read_to_string(root.join("meta.json"))?)?;
        let history = History::open(root.join("history"))?;
        Ok(Self {
            root,
            meta,
            history,
        })
    }

    pub fn open_default(cwd: &Path) -> Result<Self> {
        let summaries = Self::list(cwd)?;
        let id = summaries
            .last()
            .map(|s| s.id.clone())
            .ok_or_else(|| Error::SessionNotFound("(none)".into()))?;
        Self::open(cwd, &id)
    }

    pub fn list(cwd: &Path) -> Result<Vec<SessionSummary>> {
        let root = Self::sessions_root(cwd);
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut ids: Vec<_> = fs::read_dir(&root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("meta.json").exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            let s = Self::open(cwd, &id)?;
            out.push(SessionSummary {
                id: s.meta.id.clone(),
                label: s.meta.label.clone(),
                world: s.meta.source_world.clone(),
                dim: s.meta.dim.clone(),
                dirty: s.meta.dirty,
                history: format!("{}/{}", s.history.position(), s.history.size()),
                path: s.root.clone(),
            });
        }
        Ok(out)
    }

    pub fn save_meta(&self) -> Result<()> {
        fs::write(
            self.root.join("meta.json"),
            serde_json::to_string_pretty(&self.meta)?,
        )?;
        Ok(())
    }

    pub fn mark_dirty(&mut self) -> Result<()> {
        self.meta.dirty = true;
        self.save_meta()
    }

    pub fn discard(self) -> Result<()> {
        fs::remove_dir_all(&self.root)?;
        Ok(())
    }

    /// Refresh working region/entity copies from source.
    /// Refuses if dirty unless `force`.
    pub fn sync_from_source(&mut self, force: bool) -> Result<Vec<String>> {
        if self.meta.dirty && !force {
            return Err(Error::msg(
                "session dirty; commit/discard or pass --force to overwrite work copy",
            ));
        }
        let mut report = Vec::new();
        for (work, source, label) in [
            (self.work_region_dir(), self.source_region_dir(), "region"),
            (
                self.work_entities_dir(),
                self.source_entities_dir(),
                "entities",
            ),
        ] {
            if work.exists() {
                fs::remove_dir_all(&work)?;
            }
            fs::create_dir_all(&work)?;
            if !source.exists() {
                report.push(format!("{label}: source missing, left empty"));
                continue;
            }
            let mut n = 0usize;
            for entry in fs::read_dir(&source)? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.ends_with(".mca") {
                    fs::copy(entry.path(), work.join(name))?;
                    n += 1;
                }
            }
            report.push(format!("{label}: copied {n} files"));
        }
        self.meta.dirty = false;
        self.save_meta()?;
        Ok(report)
    }

    pub fn work_region_dir(&self) -> PathBuf {
        self.root.join("world/region")
    }

    pub fn work_entities_dir(&self) -> PathBuf {
        self.root.join("world/entities")
    }

    pub fn source_region_dir(&self) -> PathBuf {
        dim_region_dir(&self.meta.source_world, &self.meta.dim)
    }

    pub fn source_entities_dir(&self) -> PathBuf {
        dim_entities_dir(&self.meta.source_world, &self.meta.dim)
    }

    pub fn world_lock_path(cwd: &Path, world: &Path) -> PathBuf {
        let key = world_lock_key(world);
        Self::sessions_root(cwd).join("locks").join(format!("{key}.lock"))
    }

    pub fn status_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("session={}", self.meta.id),
            format!("world={}", self.meta.source_world.display()),
            format!("dim={}", self.meta.dim),
            format!("dirty={}", self.meta.dirty),
            format!(
                "history={}/{}",
                self.history.position(),
                self.history.size()
            ),
            format!("path={}", self.root.display()),
        ];
        if let Some(label) = &self.meta.label {
            lines.insert(1, format!("label={label}"));
        }
        lines
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldLock {
    pub session: String,
    pub label: Option<String>,
    pub pid: u32,
    pub since: String,
}

/// Exclusive lock for committing to a shared world (multi-session).
pub struct CommitLock {
    path: PathBuf,
}

impl CommitLock {
    pub fn acquire(cwd: &Path, session: &Session) -> Result<Self> {
        let path = Session::world_lock_path(cwd, &session.meta.source_world);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            let existing: WorldLock = serde_json::from_str(&fs::read_to_string(&path)?)?;
            if existing.session != session.meta.id && process_alive(existing.pid) {
                return Err(Error::msg(format!(
                    "world locked by session={} label={} pid={}",
                    existing.session,
                    existing.label.as_deref().unwrap_or("-"),
                    existing.pid
                )));
            }
            let _ = fs::remove_file(&path);
        }
        let lock = WorldLock {
            session: session.meta.id.clone(),
            label: session.meta.label.clone(),
            pid: std::process::id(),
            since: chrono_like_now(),
        };
        // create_new = exclusive
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::msg("world lock race; retry commit")
                } else {
                    Error::from(e)
                }
            })?;
        drop(f);
        fs::write(&path, serde_json::to_string_pretty(&lock)?)?;
        Ok(Self { path })
    }
}

impl Drop for CommitLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn world_lock_key(world: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    world.to_string_lossy().hash(&mut h);
    format!("{:x}", h.finish())
}

fn resolve_session_path(cwd: &Path, id_or_path: &str) -> Result<PathBuf> {
    let as_path = PathBuf::from(id_or_path);
    if as_path.join("meta.json").exists() {
        return Ok(as_path);
    }
    let candidate = Session::path_for(cwd, id_or_path);
    if candidate.join("meta.json").exists() {
        return Ok(candidate);
    }
    Err(Error::SessionNotFound(id_or_path.to_string()))
}

pub fn dim_region_dir(world: &Path, dim: &str) -> PathBuf {
    match dim {
        "." | "" | "overworld" => world.join("region"),
        "nether" | "the_nether" => world.join("DIM-1/region"),
        "end" | "the_end" => world.join("DIM1/region"),
        other => {
            let p = world.join(other).join("region");
            if p.exists() {
                p
            } else if world.join("region").exists() {
                world.join("region")
            } else {
                p
            }
        }
    }
}

pub fn dim_entities_dir(world: &Path, dim: &str) -> PathBuf {
    let region = dim_region_dir(world, dim);
    region
        .parent()
        .map(|p| p.join("entities"))
        .unwrap_or_else(|| world.join("entities"))
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
