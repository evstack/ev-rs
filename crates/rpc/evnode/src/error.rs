//! Error types for the EVNode executor service.

use thiserror::Error;
use tonic::Status;

/// Error type for EVNode executor service operations.
#[derive(Debug, Error)]
pub enum EvnodeError {
    /// Invalid argument provided to a method.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Resource not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),

    /// Method not implemented.
    #[error("not implemented: {0}")]
    Unimplemented(String),

    /// Service unavailable.
    #[error("unavailable: {0}")]
    Unavailable(String),

    /// Chain not initialized.
    #[error("chain not initialized")]
    ChainNotInitialized,

    /// Chain already initialized.
    #[error("chain already initialized")]
    ChainAlreadyInitialized,

    /// Storage error.
    #[error("storage error: {0}")]
    Storage(String),

    /// Transaction parsing error.
    #[error("transaction error: {0}")]
    Transaction(String),

    /// Genesis error.
    #[error("genesis error: {0}")]
    Genesis(String),
}

impl From<EvnodeError> for Status {
    fn from(err: EvnodeError) -> Self {
        match err {
            EvnodeError::InvalidArgument(msg) => Status::invalid_argument(msg),
            EvnodeError::NotFound(msg) => Status::not_found(msg),
            EvnodeError::Internal(msg) => Status::internal(msg),
            EvnodeError::Unimplemented(msg) => Status::unimplemented(msg),
            EvnodeError::Unavailable(msg) => Status::unavailable(msg),
            EvnodeError::ChainNotInitialized => {
                Status::failed_precondition("chain not initialized")
            }
            EvnodeError::ChainAlreadyInitialized => {
                Status::failed_precondition("chain already initialized")
            }
            EvnodeError::Storage(msg) => Status::internal(format!("storage error: {}", msg)),
            EvnodeError::Transaction(msg) => {
                Status::invalid_argument(format!("transaction error: {}", msg))
            }
            EvnodeError::Genesis(msg) => Status::internal(format!("genesis error: {}", msg)),
        }
    }
}
