//! JSON-RPC error types following Ethereum error code conventions.

use jsonrpsee::types::ErrorObjectOwned;
use thiserror::Error;

/// Standard Ethereum JSON-RPC error codes.
pub mod codes {
    /// Parse error - Invalid JSON
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid request - JSON is not a valid request object
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error
    pub const INTERNAL_ERROR: i32 = -32603;

    // Ethereum-specific error codes (in server error range -32000 to -32099)

    /// Execution reverted
    pub const EXECUTION_REVERTED: i32 = -32000;
    /// Resource not found
    pub const RESOURCE_NOT_FOUND: i32 = -32001;
    /// Resource unavailable
    pub const RESOURCE_UNAVAILABLE: i32 = -32002;
    /// Transaction rejected
    pub const TRANSACTION_REJECTED: i32 = -32003;
    /// Method not supported
    pub const METHOD_NOT_SUPPORTED: i32 = -32004;
}

/// RPC-specific errors.
#[derive(Debug, Error)]
pub enum RpcError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid params: {0}")]
    InvalidParams(String),

    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Block not found")]
    BlockNotFound,

    #[error("Transaction not found")]
    TransactionNotFound,

    #[error("Receipt not found")]
    ReceiptNotFound,

    #[error("Account not found")]
    AccountNotFound,

    #[error("Execution reverted: {0}")]
    ExecutionReverted(String),

    #[error("Gas estimation failed: {0}")]
    GasEstimationFailed(String),

    #[error("Method not implemented: {0}")]
    NotImplemented(String),

    #[error("Invalid block number or tag")]
    InvalidBlockNumberOrTag,

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
}

impl From<RpcError> for ErrorObjectOwned {
    fn from(err: RpcError) -> Self {
        let (code, message) = match &err {
            RpcError::ParseError(msg) => (codes::PARSE_ERROR, msg.clone()),
            RpcError::InvalidParams(msg) => (codes::INVALID_PARAMS, msg.clone()),
            RpcError::InternalError(msg) => (codes::INTERNAL_ERROR, msg.clone()),
            RpcError::BlockNotFound => (codes::RESOURCE_NOT_FOUND, "Block not found".to_string()),
            RpcError::TransactionNotFound => (
                codes::RESOURCE_NOT_FOUND,
                "Transaction not found".to_string(),
            ),
            RpcError::ReceiptNotFound => {
                (codes::RESOURCE_NOT_FOUND, "Receipt not found".to_string())
            }
            RpcError::AccountNotFound => {
                (codes::RESOURCE_NOT_FOUND, "Account not found".to_string())
            }
            RpcError::ExecutionReverted(msg) => (codes::EXECUTION_REVERTED, msg.clone()),
            RpcError::GasEstimationFailed(msg) => (codes::EXECUTION_REVERTED, msg.clone()),
            RpcError::NotImplemented(method) => (
                codes::METHOD_NOT_SUPPORTED,
                format!("Method not implemented: {}", method),
            ),
            RpcError::InvalidBlockNumberOrTag => (
                codes::INVALID_PARAMS,
                "Invalid block number or tag".to_string(),
            ),
            RpcError::InvalidAddress(msg) => (codes::INVALID_PARAMS, msg.clone()),
            RpcError::InvalidTransaction(msg) => (codes::TRANSACTION_REJECTED, msg.clone()),
        };

        ErrorObjectOwned::owned(code, message, None::<()>)
    }
}

/// Result type for RPC operations.
pub type RpcResult<T> = Result<T, RpcError>;
