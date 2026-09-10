use crate::error::Result;
use crate::region::{
    list_region_files, parse_region_name, read_linear_source_marker, work_mca_to_linear_bytes,
    RegionFormat,
};
use crate::session::{CommitLock, Session};
use std::fs;
use std::path::{Path, PathBuf};

/// Copy dirty working region/entity files back to the source world.
/// Takes an exclusive world commit lock for multi-session safety.
///
/// If a work region was converted from Linear (marker `r.X.Z.linear.source`),
/// commit writes `.linear` back to the source (and removes a stale `.mca` there
/// only when the source originally had Linear).
pub fn commit_session(
    cwd: &Path,
    session: &mut Session,
    dry_run: bool,
) -> Result<Vec<String>> {
    let _lock = if dry_run {
        None
    } else {
        Some(CommitLock::acquire(cwd, session)?)
    };

    let mut report = Vec::new();
    for hint in crate::session::commit_conflict_hints(cwd, session)? {
        report.push(format!("warn={hint}"));
    }
    let pairs = [
        (
            session.work_region_dir(),
            session.source_region_dir(),
            "region",
        ),
        (
            session.work_entities_dir(),
            session.source_entities_dir(),
            "entities",
        ),
    ];

    for (work, source, label) in pairs {
        if !work.exists() {
            continue;
        }
        for entry in fs::read_dir(&work)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.ends_with(".linear.source") {
                continue;
            }
            if !(name.ends_with(".mca") || name.ends_with(".linear")) {
                continue;
            }
            let src_file = entry.path();

            if name.ends_with(".mca") {
                let Ok((rx, rz)) = parse_region_name(name) else {
                    continue;
                };
                if let Some(ver) = read_linear_source_marker(&work, rx, rz) {
                    let linear_name = format!("r.{rx}.{rz}.linear");
                    let dst_file = source.join(&linear_name);
                    report.push(format!(
                        "{label}/{name} -> {} (linear {:?})",
                        dst_file.display(),
                        ver
                    ));
                    if !dry_run {
                        fs::create_dir_all(&source)?;
                        let bytes = work_mca_to_linear_bytes(&src_file, rx, rz, ver)?;
                        atomic_copy_bytes(&bytes, &dst_file, "linear.tmp")?;
                        // Prefer Linear as source of truth: drop anvil sibling if present.
                        let stale_mca = source.join(format!("r.{rx}.{rz}.mca"));
                        let _ = fs::remove_file(stale_mca);
                    }
                    continue;
                }
            }

            let dst_file = source.join(name);
            report.push(format!("{label}/{name} -> {}", dst_file.display()));
            if !dry_run {
                fs::create_dir_all(&source)?;
                atomic_copy(&src_file, &dst_file)?;
            }
        }
    }

    if !dry_run {
        session.meta.dirty = false;
        session.save_meta()?;
    }
    if report.is_empty() {
        report.push("nothing to commit".into());
    }
    Ok(report)
}

fn atomic_copy(from: &Path, to: &Path) -> Result<()> {
    let ext = to
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    let tmp = to.with_extension(format!("{ext}.tmp"));
    fs::copy(from, &tmp)?;
    fs::rename(&tmp, to)?;
    Ok(())
}

fn atomic_copy_bytes(data: &[u8], to: &Path, tmp_ext: &str) -> Result<()> {
    let tmp = to.with_extension(tmp_ext);
    fs::write(&tmp, data)?;
    fs::rename(&tmp, to)?;
    Ok(())
}

pub fn list_work_regions(session: &Session) -> Result<Vec<(i32, i32)>> {
    list_region_files(&session.work_region_dir())
}

pub fn session_work_files(session: &Session) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in [session.work_region_dir(), session.work_entities_dir()] {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .map(|s| s.ends_with(".mca") || s.ends_with(".linear"))
                .unwrap_or(false)
            {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

pub fn detect_source_region_format(session: &Session, rx: i32, rz: i32) -> Option<RegionFormat> {
    crate::region::find_source_region_file(&session.source_region_dir(), rx, rz).map(|(_, f)| f)
}
