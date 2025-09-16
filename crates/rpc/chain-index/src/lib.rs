//! Chain data indexing and persistence for RPC queries.
//!
//! This crate provides storage and indexing for chain data (blocks, transactions,
//! receipts) that is separate from the core state storage. This separation allows:
//!
//! - Different retention policies (prune old blocks, keep state)
//! - Optimized access patterns (append-only chain data vs mutable state)
//! - Optional archive mode for full historical queries
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                    RPC Server                        │
//! │              (evolve_eth_jsonrpc)                    │
//! └───────────────────────┬─────────────────────────────┘
//!                         │
//!           ┌─────────────▼─────────────┐
//!           │      StateProvider        │
//!           │   (ChainStateProvider)    │
//!           └─────────────┬─────────────┘
//!                         │
//!         ┌───────────────┼───────────────┐
//!         │               │               │
//!         ▼               ▼               ▼
//! ┌───────────────┐ ┌───────────┐ ┌───────────────┐
//! │  ChainIndex   │ │  Storage  │ │     STF       │
//! │ (this crate)  │ │  (state)  │ │  (execution)  │
//! └───────────────┘ └───────────┘ └───────────────┘
//! ```

pub mod cache;
pub mod error;
pub mod index;
pub mod integration;
pub mod provider;
pub mod types;

pub use cache::ChainCache;
pub use error::{ChainIndexError, ChainIndexResult};
pub use index::{ChainIndex, PersistentChainIndex};
pub use integration::{build_index_data, event_to_stored_log, index_block, BlockMetadata};
pub use provider::ChainStateProvider;
pub use types::*;
