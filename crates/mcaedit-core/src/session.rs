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
    /// Collaboration label (agent / user)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Preferred DataVersion for newly created empty chunks (from level.dat / bootstrap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_version: Option<i32>,
}

/// Options for `session create` when bootstrapping an empty world.
#[derive(Clone, Debug, Default)]
pub struct CreateSessionOpts {
    pub bootstrap: bool,
    pub force_level: bool,
    pub level_name: Option<String>,
    pub seed: Option<i64>,
    pub version: Option<crate::mc_version::ResolvedVersion>,
    pub generator: Option<crate::level::GeneratorKind>,
    pub region_format: Option<crate::level::RegionFormat>,
    pub data_version: Option<i32>,
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
    /// One-shot note from `create --bootstrap` (not persisted).
    pub bootstrap_note: Option<String>,
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
        Self::create_with_opts(
            cwd,
            source_world,
            dim,
            id,
            label,
            CreateSessionOpts::default(),
        )
    }

    pub fn create_with_opts(
        cwd: &Path,
        source_world: &Path,
        dim: &str,
        id: Option<String>,
        label: Option<String>,
        opts: CreateSessionOpts,
    ) -> Result<Self> {
        let source_world = if source_world.exists() {
            source_world
                .canonicalize()
                .map_err(|e| Error::msg(format!("world path: {e}")))?
        } else if opts.bootstrap {
            fs::create_dir_all(source_world)?;
            source_world
                .canonicalize()
                .map_err(|e| Error::msg(format!("world path: {e}")))?
        } else {
            return Err(Error::msg(format!(
                "world path missing: {} (pass --bootstrap to create)",
                source_world.display()
            )));
        };

        let region_dir = dim_region_dir(&source_world, dim);
        let mut data_version = opts.data_version;
        let mut bootstrap_note: Option<String> = None;
        let level_path = source_world.join("level.dat");
        let want_skeleton = opts.bootstrap || crate::level::needs_bootstrap(&source_world);
        if want_skeleton {
            if level_path.exists() && !opts.force_level {
                // --bootstrap with existing level.dat: ensure dirs only, never overwrite.
                crate::level::ensure_world_dirs(
                    &source_world,
                    dim,
                    opts.region_format
                        .unwrap_or(crate::level::RegionFormat::Anvil),
                )?;
                if let Ok(info) = crate::level::info(&source_world) {
                    data_version = data_version.or(info.data_version);
                }
                bootstrap_note = Some(format!(
                    "bootstrap=existing level.dat={} (dirs ensured; not overwritten)",
                    level_path.display()
                ));
            } else if !region_dir.exists() || opts.bootstrap {
                let mut create = crate::level::WorldCreateOptions {
                    path: source_world.clone(),
                    level_name: opts.level_name.clone().unwrap_or_else(|| {
                        source_world
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("world")
                            .to_string()
                    }),
                    force: opts.force_level || !level_path.exists(),
                    ..Default::default()
                };
                if let Some(seed) = opts.seed {
                    create.seed = seed;
                }
                if let Some(v) = opts.version.clone() {
                    create.version = v;
                }
                if let Some(g) = opts.generator {
                    create.generator = g;
                }
                if let Some(fmt) = opts.region_format {
                    create.region_format = fmt;
                }
                let info = crate::level::create_world(&create)?;
                data_version = data_version.or(info.data_version);
                bootstrap_note = Some("bootstrap=created level.dat + region skeleton".into());
            }
        }
        if !region_dir.exists() {
            return Err(Error::msg(format!(
                "region dir missing: {} (use session create --bootstrap)",
                region_dir.display()
            )));
        }
        if data_version.is_none() {
            if let Ok(info) = crate::level::info(&source_world) {
                data_version = info.data_version;
            }
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
            data_version,
        };
        fs::write(root.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
        fs::write(root.join("HEAD"), "0\n")?;
        let history = History::open(root.join("history"))?;
        Ok(Self {
            root,
            meta,
            history,
            bootstrap_note,
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
            bootstrap_note: None,
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
                if name.ends_with(".mca") || name.ends_with(".linear") {
                    // Linear sources are converted lazily via copy_region_if_needed;
                    // still seed work copy so list/sync reports files.
                    if name.ends_with(".linear") {
                        // Convert immediately so work tree is Anvil-editable.
                        let bytes = fs::read(entry.path())?;
                        if crate::linear::is_linear_file(&bytes) {
                            let mut linear = crate::linear::read_linear(&bytes)?;
                            let (rx, rz) = crate::region::parse_region_name(name)?;
                            linear.region_x = rx;
                            linear.region_z = rz;
                            let mca = crate::linear::linear_to_mca_bytes(&linear)?;
                            let mca_name = crate::region::region_file_name(rx, rz);
                            crate::region::atomic_write(&work.join(&mca_name), &mca)?;
                            crate::region::write_linear_source_marker(
                                &work,
                                rx,
                                rz,
                                linear.version,
                            )?;
                            n += 1;
                            continue;
                        }
                    }
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
                    "world locked by session={} label={} pid={} (another agent is committing; retry after it finishes, or sync)",
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

/// Soft AABB claim for multi-agent coordination (not a hard lock).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionLease {
    pub session: String,
    pub label: Option<String>,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub updated_at: String,
}

impl RegionLease {
    pub fn aabb(&self) -> (i32, i32, i32, i32, i32, i32) {
        (
            self.from[0].min(self.to[0]),
            self.from[1].min(self.to[1]),
            self.from[2].min(self.to[2]),
            self.from[0].max(self.to[0]),
            self.from[1].max(self.to[1]),
            self.from[2].max(self.to[2]),
        )
    }

    pub fn overlaps(&self, other: &RegionLease) -> bool {
        let (ax0, ay0, az0, ax1, ay1, az1) = self.aabb();
        let (bx0, by0, bz0, bx1, by1, bz1) = other.aabb();
        ax0 <= bx1 && ax1 >= bx0 && ay0 <= by1 && ay1 >= by0 && az0 <= bz1 && az1 >= bz0
    }
}

impl Session {
    pub fn leases_dir(cwd: &Path) -> PathBuf {
        Self::sessions_root(cwd).join("leases")
    }

    pub fn lease_path(cwd: &Path, session_id: &str) -> PathBuf {
        Self::leases_dir(cwd).join(format!("{session_id}.json"))
    }

    /// Claim / refresh an AABB lease under `.mcaedit/leases/`.
    pub fn set_lease(
        cwd: &Path,
        session: &Session,
        from: [i32; 3],
        to: [i32; 3],
    ) -> Result<(RegionLease, Vec<String>)> {
        let lease = RegionLease {
            session: session.meta.id.clone(),
            label: session.meta.label.clone(),
            from,
            to,
            updated_at: chrono_like_now(),
        };
        let dir = Self::leases_dir(cwd);
        fs::create_dir_all(&dir)?;
        let warnings = list_lease_conflicts(cwd, &lease)?;
        fs::write(
            Self::lease_path(cwd, &session.meta.id),
            serde_json::to_string_pretty(&lease)?,
        )?;
        Ok((lease, warnings))
    }

    pub fn clear_lease(cwd: &Path, session_id: &str) -> Result<bool> {
        let p = Self::lease_path(cwd, session_id);
        if p.exists() {
            fs::remove_file(p)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn list_leases(cwd: &Path) -> Result<Vec<RegionLease>> {
        let dir = Self::leases_dir(cwd);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for e in fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&p) {
                if let Ok(lease) = serde_json::from_str::<RegionLease>(&raw) {
                    out.push(lease);
                }
            }
        }
        out.sort_by(|a, b| a.session.cmp(&b.session));
        Ok(out)
    }
}

pub fn list_lease_conflicts(cwd: &Path, mine: &RegionLease) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    for other in Session::list_leases(cwd)? {
        if other.session == mine.session {
            continue;
        }
        if mine.overlaps(&other) {
            warnings.push(format!(
                "lease conflict: session={} label={} aabb=({},{},{})..({},{},{}) overlaps ours",
                other.session,
                other.label.as_deref().unwrap_or("-"),
                other.from[0],
                other.from[1],
                other.from[2],
                other.to[0],
                other.to[1],
                other.to[2],
            ));
        }
    }
    Ok(warnings)
}

/// Warn when other sessions' work-region files share region names with ours and look newer.
pub fn commit_conflict_hints(cwd: &Path, session: &Session) -> Result<Vec<String>> {
    let mut hints = Vec::new();
    let our_regions = match list_work_region_names(session) {
        Ok(v) => v,
        Err(_) => return Ok(hints),
    };
    if our_regions.is_empty() {
        return Ok(hints);
    }
    for other in Session::list(cwd)? {
        if other.id == session.meta.id {
            continue;
        }
        let other_root = Session::path_for(cwd, &other.id);
        let other_region = other_root.join("world/region");
        if !other_region.exists() {
            continue;
        }
        for name in &our_regions {
            let ours = session.work_region_dir().join(name);
            let theirs = other_region.join(name);
            if !theirs.exists() || !ours.exists() {
                continue;
            }
            let om = fs::metadata(&ours)?.modified().ok();
            let tm = fs::metadata(&theirs)?.modified().ok();
            if let (Some(o), Some(t)) = (om, tm) {
                if t > o {
                    hints.push(format!(
                        "region conflict hint: {name} also dirty in session={} label={} (their mtime newer; sync/coordinate before commit)",
                        other.id,
                        other.label.as_deref().unwrap_or("-")
                    ));
                }
            }
        }
    }
    // Lease overlaps
    if let Ok(Some(mine)) = read_own_lease(cwd, &session.meta.id) {
        for w in list_lease_conflicts(cwd, &mine)? {
            hints.push(w);
        }
    }
    Ok(hints)
}

fn read_own_lease(cwd: &Path, id: &str) -> Result<Option<RegionLease>> {
    let p = Session::lease_path(cwd, id);
    if !p.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(p)?)?))
}

fn list_work_region_names(session: &Session) -> Result<Vec<String>> {
    let dir = session.work_region_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.ends_with(".mca") || name.ends_with(".linear") {
            names.push(name.to_string());
        }
    }
    Ok(names)
}
