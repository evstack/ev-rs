//! EVNode ExecutorService gRPC implementation for Evolve.
//!
//! This crate provides a gRPC server that implements the ev-node ExecutorService
//! protocol, allowing Evolve to be used as an execution layer for ev-node.
//!
//! # Overview
//!
//! The ExecutorService provides the following RPC methods:
//!
//! - `InitChain`: Initialize the blockchain with genesis parameters
//! - `GetTxs`: Fetch pending transactions from the mempool
//! - `ExecuteTxs`: Execute a batch of transactions to produce a new block
//! - `SetFinal`: Mark a block as finalized
//! - `GetExecutionInfo`: Get execution layer parameters (max gas, etc.)
//! - `FilterTxs`: Validate and filter transactions for block inclusion
//!
//! # Example
//!
//! ```ignore
//! use evolve_evnode::{EvnodeServer, EvnodeServerConfig};
//!
//! // Create the server with your STF, storage, and account codes
//! let server = EvnodeServer::new(
//!     EvnodeServerConfig::default(),
//!     stf,
//!     storage,
//!     codes,
//! );
//!
//! // Start serving
//! server.serve().await?;
//! ```

pub mod error;
pub mod server;
pub mod service;

// Testapp integration (provides EvnodeStfExecutor impl for MempoolStf)
#[cfg(feature = "testapp")]
pub mod testapp_impl;

// Re-export generated protobuf types
pub mod proto {
    pub mod evnode {
        pub mod v1 {
            include!("generated/evnode.v1.rs");
        }
    }
}

// Re-export key types
pub use error::EvnodeError;
pub use server::{
    start_evnode_server, start_evnode_server_with_mempool, EvnodeServer, EvnodeServerConfig,
};
pub use service::{
    compute_state_root, EvnodeStfExecutor, ExecutorServiceConfig, ExecutorServiceImpl,
    StateChangeCallback,
};
