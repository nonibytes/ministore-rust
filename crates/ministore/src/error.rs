use thiserror::Error;

pub type Result<T> = std::result::Result<T, MinistoreError>;

#[derive(Debug, Error)]
pub enum MinistoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("schema error: {0}")]
    Schema(String),

    #[error("query parse error: {0}")]
    QueryParse(String),

    #[error("query rejected: {0}")]
    QueryRejected(String),

    #[error("unknown field: {0}")]
    UnknownField(String),

    #[error("type mismatch for field '{field}': {message}")]
    TypeMismatch { field: String, message: String },

    #[error("cursor error: {0}")]
    Cursor(String),

    #[error("sqlite missing required feature: {0}")]
    SqliteFeatureMissing(String),

    #[error("item not found: {0}")]
    NotFound(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
