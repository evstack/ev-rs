//! evolve_server - Server infrastructure for Evolve applications.
//!
//! This crate provides:
//! - `DevConsensus` for development/testing block production
//! - Block building utilities
//! - Chain indexing infrastructure
//! - Persistence helpers for chain state
//!
//! # Example
//!
//! ```ignore
//! use evolve_server::{DevConfig, DevConsensus};
//! use commonware_runtime::Spawner;
//!
//! // Create dev consensus
//! let dev = Arc::new(DevConsensus::new(stf, storage, codes, config));
//!
//! // Run block production with commonware-runtime lifecycle
//! context.spawn(|ctx| {
//!     let dev = dev.clone();
//!     async move { dev.run_block_production(ctx).await }
//! });
//! ```

mod block;
pub mod dev;
mod error;
mod persistence;

pub use block::{Block, BlockBuilder, BlockHeader};
pub use dev::{DevConfig, DevConsensus, NoopChainIndex, ProducedBlock, StfExecutor};
pub use error::ServerError;
pub use evolve_mempool::{
    new_shared_mempool, new_shared_mempool_with_base_fee, Mempool, MempoolError, MempoolResult,
    SharedMempool, TxContext,
};
pub use persistence::{load_chain_state, save_chain_state, ChainState, CHAIN_STATE_KEY};

// Re-export RPC-related types for convenience
pub use evolve_chain_index::{
    ChainIndex, ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex,
};
pub use evolve_eth_jsonrpc::{
    start_server as start_rpc_server, RpcServerConfig, SharedSubscriptionManager,
    SubscriptionManager,
};
pub use evolve_grpc::{GrpcServer, GrpcServerConfig};
