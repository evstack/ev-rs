//! Server error types.

use thiserror::Error;

/// Errors that can occur during server operations.
#[derive(Debug, Error)]
pub enum ServerError {
    /// No block has been applied yet, cannot commit.
    #[error("no pending block to commit")]
    NoPendingBlock,

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(String),

    /// Genesis initialization failed.
    #[error("genesis initialization failed: {0}")]
    Genesis(String),

    /// RPC server error.
    #[error("RPC server error: {0}")]
    Rpc(String),

    /// gRPC server error.
    #[error("gRPC server error: {0}")]
    Grpc(String),

    /// Block execution error.
    #[error("block execution error: {0}")]
    Execution(String),

    /// Shutdown error.
    #[error("shutdown error: {0}")]
    Shutdown(String),

    /// Indexer error.
    #[error("indexer error: {0}")]
    Indexer(String),
}

impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> Self {
        ServerError::Storage(err.to_string())
    }
}
