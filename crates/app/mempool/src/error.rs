//! Mempool error types.

use thiserror::Error;

/// Errors that can occur when interacting with the mempool.
#[derive(Debug, Error)]
pub enum MempoolError {
    /// Transaction decoding failed.
    #[error("failed to decode transaction: {0}")]
    DecodeFailed(String),

    /// Invalid chain ID.
    #[error("invalid chain id: expected {expected}, got {actual}")]
    InvalidChainId { expected: u64, actual: u64 },

    /// Missing chain ID.
    #[error("missing chain id (EIP-155 required)")]
    MissingChainId,

    /// Transaction already in mempool.
    #[error("transaction already in mempool")]
    AlreadyExists,

    /// Transaction has no recipient (contract creation not supported yet).
    #[error("contract creation transactions not supported")]
    ContractCreationNotSupported,

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Result type for mempool operations.
pub type MempoolResult<T> = Result<T, MempoolError>;
