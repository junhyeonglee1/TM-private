use std::path::PathBuf;

/// Errors produced by the shared TM core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("directory traversal error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("data invariant violated: {0}")]
    Invariant(String),

    #[error("backup file is not a valid TM database: {0}")]
    InvalidBackup(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

pub(crate) fn not_found(entity: &'static str, id: impl Into<String>) -> Error {
    Error::NotFound {
        entity,
        id: id.into(),
    }
}
