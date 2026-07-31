use thiserror::Error;

pub type Result<T> = std::result::Result<T, OkfError>;

/// Operational failures that prevent OKF processing from producing a complete
/// result. Format defects are represented by findings instead.
#[derive(Debug, Error)]
pub enum OkfError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite staging error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON staging error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid OKF bundle: {0}")]
    InvalidBundle(String),
}
