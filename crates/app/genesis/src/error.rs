//! Genesis error types.

use thiserror::Error;

/// Errors that can occur during genesis processing.
#[derive(Debug, Error)]
pub enum GenesisError {
    /// Failed to parse genesis file
    #[error("failed to parse genesis file: {0}")]
    ParseError(String),

    /// Invalid reference in genesis file
    #[error("invalid reference '${0}' at transaction {1}")]
    InvalidReference(String, usize),

    /// Circular reference detected
    #[error("circular reference detected involving '{0}'")]
    CircularReference(String),

    /// Duplicate transaction ID
    #[error("duplicate transaction id '{0}'")]
    DuplicateId(String),

    /// Genesis failed
    #[error("genesis failed: {0}")]
    Failed(String),

    /// A genesis transaction failed
    #[error("genesis transaction {index} (id: {id:?}) failed: {error}")]
    TransactionFailed {
        index: usize,
        id: Option<String>,
        error: String,
    },

    /// Unknown message type
    #[error("unknown message type '{0}'")]
    UnknownMessageType(String),

    /// Failed to encode message
    #[error("failed to encode message: {0}")]
    EncodeError(String),

    /// IO error
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON error
    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}
