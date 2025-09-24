//! gRPC error types and conversion to tonic::Status.

use evolve_eth_jsonrpc::RpcError;
use thiserror::Error;
use tonic::{Code, Status};

/// gRPC-specific errors.
#[derive(Debug, Error)]
pub enum GrpcError {
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unimplemented: {0}")]
    Unimplemented(String),

    #[error("Unavailable: {0}")]
    Unavailable(String),

    #[error(transparent)]
    Rpc(#[from] RpcError),
}

impl From<GrpcError> for Status {
    fn from(err: GrpcError) -> Self {
        match err {
            GrpcError::InvalidArgument(msg) => Status::new(Code::InvalidArgument, msg),
            GrpcError::NotFound(msg) => Status::new(Code::NotFound, msg),
            GrpcError::Internal(msg) => Status::new(Code::Internal, msg),
            GrpcError::Unimplemented(msg) => Status::new(Code::Unimplemented, msg),
            GrpcError::Unavailable(msg) => Status::new(Code::Unavailable, msg),
            GrpcError::Rpc(rpc_err) => rpc_error_to_status(rpc_err),
        }
    }
}

/// Convert an RpcError to a tonic::Status with appropriate gRPC code.
fn rpc_error_to_status(err: RpcError) -> Status {
    match err {
        RpcError::ParseError(msg) => Status::new(Code::InvalidArgument, msg),
        RpcError::InvalidParams(msg) => Status::new(Code::InvalidArgument, msg),
        RpcError::InternalError(msg) => Status::new(Code::Internal, msg),
        RpcError::BlockNotFound => Status::new(Code::NotFound, "Block not found"),
        RpcError::TransactionNotFound => Status::new(Code::NotFound, "Transaction not found"),
        RpcError::ReceiptNotFound => Status::new(Code::NotFound, "Receipt not found"),
        RpcError::AccountNotFound => Status::new(Code::NotFound, "Account not found"),
        RpcError::ExecutionReverted(msg) => {
            Status::new(Code::Aborted, format!("Execution reverted: {}", msg))
        }
        RpcError::GasEstimationFailed(msg) => {
            Status::new(Code::Aborted, format!("Gas estimation failed: {}", msg))
        }
        RpcError::NotImplemented(method) => Status::new(
            Code::Unimplemented,
            format!("Method not implemented: {}", method),
        ),
        RpcError::InvalidBlockNumberOrTag => {
            Status::new(Code::InvalidArgument, "Invalid block number or tag")
        }
        RpcError::InvalidAddress(msg) => Status::new(Code::InvalidArgument, msg),
        RpcError::InvalidTransaction(msg) => Status::new(
            Code::InvalidArgument,
            format!("Invalid transaction: {}", msg),
        ),
    }
}

/// Result type for gRPC operations.
pub type GrpcResult<T> = Result<T, GrpcError>;
