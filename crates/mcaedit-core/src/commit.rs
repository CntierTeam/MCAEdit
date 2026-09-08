use crate::error::Result;
use crate::region::list_region_files;
use crate::session::{CommitLock, Session};
use std::fs;
use std::path::{Path, PathBuf};

/// Copy dirty working region/entity MCA files back to the source world.
/// Takes an exclusive world commit lock for multi-session safety.
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
            if !name.ends_with(".mca") {
                continue;
            }
            let src_file = entry.path();
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
    let tmp = to.with_extension("mca.tmp");
    fs::copy(from, &tmp)?;
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
                .map(|s| s.ends_with(".mca"))
                .unwrap_or(false)
            {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}
