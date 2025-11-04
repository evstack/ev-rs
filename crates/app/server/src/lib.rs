//! EvolveServer - Infrastructure composition for Evolve applications.
//!
//! This crate provides the `EvolveServer` type that composes all infrastructure
//! concerns (storage, RPC, gRPC, indexing, observability) into a single server.
//!
//! Application developers provide:
//! - STF type with modules, begin/end block hooks, tx validation
//! - Genesis initialization function
//!
//! The framework provides:
//! - Storage initialization and management
//! - JSON-RPC and gRPC servers
//! - Chain indexing (two-phase: chain data + state snapshots)
//! - Observability (logging, metrics)
//! - Graceful shutdown coordination
//!
//! # Example
//!
//! ```ignore
//! use evolve_server::{EvolveServer, ServerBuilder};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let server = EvolveServer::<MyStf, MyStorage, MyCodes>::builder()
//!         .build(stf, storage, codes, &observability_config, shutdown_timeout)
//!         .await?;
//!
//!     // Consensus layer calls register_block + commit
//!     loop {
//!         let block = receive_block().await;
//!         let result = stf.apply_block(&storage, &codes, &block);
//!         server.register_block(height, hash, block_data).await?;
//!         server.commit(|| storage.commit()).await?;
//!     }
//! }
//! ```

mod block;
mod builder;
pub mod dev;
mod error;
mod indexer;
mod persistence;
mod server;

pub use block::{Block, BlockBuilder, BlockHeader};
pub use builder::ServerBuilder;
pub use dev::{DevConfig, DevConsensus, NoopChainIndex, ProducedBlock, StfExecutor};
pub use error::ServerError;
pub use evolve_mempool::{
    new_shared_mempool, new_shared_mempool_with_base_fee, Mempool, MempoolError, MempoolResult,
    MempoolTransaction, SharedMempool,
};
pub use indexer::{BlockData, IndexEvent, IndexerHandle, Log, Receipt};
pub use persistence::{load_chain_state, save_chain_state, ChainState, CHAIN_STATE_KEY};
pub use server::EvolveServer;

// Re-export RPC-related types for convenience
pub use evolve_chain_index::{
    ChainIndex, ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex,
};
pub use evolve_eth_jsonrpc::{
    start_server as start_rpc_server, RpcServerConfig, SharedSubscriptionManager,
    SubscriptionManager,
};
pub use evolve_grpc::{GrpcServer, GrpcServerConfig};
