use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database {0} is in use by another Agenium process")]
    Locked(PathBuf),
    #[error("database schema version {found} is newer than this build supports ({supported}); update Agenium")]
    TooNew { found: u32, supported: u32 },
    #[error("migration {version} failed: {message}")]
    Migration { version: u32, message: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("corrupt: {0}")]
    Corrupt(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        StoreError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;
