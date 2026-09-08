use crate::action::Action;
use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Disk-backed history with undo/redo cursor.
#[derive(Debug)]
pub struct History {
    dir: PathBuf,
    /// Number of committed actions on disk (max id written while at tip).
    size: usize,
    /// Cursor: next action index to apply for redo; undo goes to position-1.
    /// After N pushes with no undo: position == size.
    position: usize,
}

impl History {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let head_path = dir.join("../HEAD");
        let position = if head_path.exists() {
            fs::read_to_string(&head_path)?
                .trim()
                .parse::<usize>()
                .unwrap_or(0)
        } else {
            0
        };
        let size = count_actions(&dir)?;
        let position = position.min(size);
        Ok(Self {
            dir,
            size,
            position,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn list(&self) -> Result<Vec<Action>> {
        let mut out = Vec::new();
        for i in 1..=self.size {
            if let Some(a) = self.load(i)? {
                out.push(a);
            }
        }
        Ok(out)
    }

    pub fn load(&self, id: usize) -> Result<Option<Action>> {
        let path = self.action_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&text)?))
    }

    pub fn push(&mut self, mut action: Action) -> Result<Action> {
        // Drop redo tail
        if self.position < self.size {
            for i in (self.position + 1)..=self.size {
                let _ = fs::remove_file(self.action_path(i));
            }
            self.size = self.position;
        }
        let id = self.size + 1;
        action.id = id as u64;
        let path = self.action_path(id);
        fs::write(&path, serde_json::to_string_pretty(&action)?)?;
        self.size = id;
        self.position = id;
        self.write_head()?;
        Ok(action)
    }

    pub fn undo_target(&self) -> Result<Action> {
        if self.position == 0 {
            return Err(Error::NothingToUndo);
        }
        self.load(self.position)?
            .ok_or(Error::NothingToUndo)
    }

    pub fn redo_target(&self) -> Result<Action> {
        if self.position >= self.size {
            return Err(Error::NothingToRedo);
        }
        self.load(self.position + 1)?
            .ok_or(Error::NothingToRedo)
    }

    pub fn mark_undone(&mut self) -> Result<()> {
        if self.position == 0 {
            return Err(Error::NothingToUndo);
        }
        self.position -= 1;
        self.write_head()
    }

    pub fn mark_redone(&mut self) -> Result<()> {
        if self.position >= self.size {
            return Err(Error::NothingToRedo);
        }
        self.position += 1;
        self.write_head()
    }

    pub fn revert_to(&mut self, index: usize) -> Result<Vec<Action>> {
        if index > self.size {
            return Err(Error::HistoryOutOfRange(index, self.size));
        }
        let mut to_undo = Vec::new();
        while self.position > index {
            to_undo.push(self.undo_target()?);
            self.mark_undone()?;
        }
        Ok(to_undo)
    }

    fn action_path(&self, id: usize) -> PathBuf {
        self.dir.join(format!("{id:05}.json"))
    }

    fn write_head(&self) -> Result<()> {
        let head = self.dir.join("../HEAD");
        fs::write(head, format!("{}\n", self.position))?;
        Ok(())
    }
}

fn count_actions(dir: &Path) -> Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut max = 0usize;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(stem) = name.strip_suffix(".json") {
            if let Ok(n) = stem.parse::<usize>() {
                max = max.max(n);
            }
        }
    }
    Ok(max)
}
