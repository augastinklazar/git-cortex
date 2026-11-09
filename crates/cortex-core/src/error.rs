use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CortexError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid ID format '{0}': expected prefix '{1}' followed by ULID")]
    InvalidId(String, &'static str),

    #[error("Schema error: {0}")]
    Schema(String),

    #[error("Redaction error: {0}")]
    Redaction(String),

    #[error("Canonical serialization failed: {0}")]
    Canonicalization(String),

    #[error("Path '{0}' is outside repository root '{1}'")]
    PathOutOfBounds(PathBuf, PathBuf),

    #[error("Symbol not found: '{0}' in '{1}'")]
    SymbolNotFound(String, String),

    #[error("Unsupported language for file: {0}")]
    UnsupportedLanguage(String),

    #[error("Git storage error: {0}")]
    Git(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invariant violation: {0}")]
    InvariantViolation(String),

    #[error("Rewind error: {0}")]
    Rewind(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CortexError>;
