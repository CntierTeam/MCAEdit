use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Nbt(#[from] fastnbt::error::Error),

    #[error("MCA error: {0}")]
    Mca(String),

    #[error("{0}")]
    Msg(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("chunk ({0}, {1}) missing")]
    ChunkMissing(i32, i32),

    #[error("no active history entry to undo")]
    NothingToUndo,

    #[error("no history entry to redo")]
    NothingToRedo,

    #[error("history index {0} out of range (size={1})")]
    HistoryOutOfRange(usize, usize),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<mca::McaError> for Error {
    fn from(value: mca::McaError) -> Self {
        Self::Mca(value.to_string())
    }
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }
}
