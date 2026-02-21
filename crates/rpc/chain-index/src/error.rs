//! Error types for chain indexing operations.

use thiserror::Error;

/// Errors that can occur during chain indexing operations.
#[derive(Debug, Error)]
pub enum ChainIndexError {
    /// Block not found in index.
    #[error("block not found: {0}")]
    BlockNotFound(u64),

    /// Block hash not found in index.
    #[error("block hash not found: {0}")]
    BlockHashNotFound(String),

    /// Transaction not found in index.
    #[error("transaction not found: {0}")]
    TransactionNotFound(String),

    /// Receipt not found in index.
    #[error("receipt not found: {0}")]
    ReceiptNotFound(String),

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(String),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Deserialization error.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Invalid block - parent hash mismatch.
    #[error("invalid block: parent hash mismatch at height {0}")]
    InvalidParentHash(u64),

    /// Block already exists.
    #[error("block already exists at height {0}")]
    BlockAlreadyExists(u64),

    /// Index is empty (no blocks stored).
    #[error("chain index is empty")]
    EmptyIndex,

    /// SQLite database error.
    #[error("sqlite error: {0}")]
    Sqlite(String),
}

impl From<serde_json::Error> for ChainIndexError {
    fn from(err: serde_json::Error) -> Self {
        ChainIndexError::Serialization(err.to_string())
    }
}

impl From<evolve_core::ErrorCode> for ChainIndexError {
    fn from(err: evolve_core::ErrorCode) -> Self {
        ChainIndexError::Storage(format!("error code: {:?}", err))
    }
}

impl From<rusqlite::Error> for ChainIndexError {
    fn from(err: rusqlite::Error) -> Self {
        ChainIndexError::Sqlite(err.to_string())
    }
}

/// Result type for chain indexing operations.
pub type ChainIndexResult<T> = Result<T, ChainIndexError>;
