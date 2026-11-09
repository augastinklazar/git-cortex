use thiserror::Error;

#[derive(Error, Debug)]
pub enum CortexError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid ID format: {0}")]
    InvalidId(String),
    #[error("Schema validation failed: {0}")]
    SchemaValidation(String),
    #[error("Generic error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CortexError>;
