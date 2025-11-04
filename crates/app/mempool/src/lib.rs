//! In-memory transaction mempool for Evolve.
//!
//! This crate provides a thread-safe mempool for pending Ethereum transactions,
//! designed to work with the dev consensus engine.
//!
//! # Features
//!
//! - Decodes and validates Ethereum transactions (Legacy and EIP-1559)
//! - Enforces chain ID for replay protection
//! - Orders transactions by effective gas price
//! - Thread-safe via `Arc<RwLock<Mempool>>`
//!
//! # Usage
//!
//! ```ignore
//! use evolve_mempool::{new_shared_mempool, SharedMempool};
//!
//! // Create a shared mempool for chain ID 1337
//! let mempool: SharedMempool = new_shared_mempool(1337);
//!
//! // Add a raw transaction (from RPC)
//! let hash = mempool.write().await.add_raw(&raw_tx_bytes)?;
//!
//! // Select transactions for block production
//! let txs = mempool.write().await.select(100);
//!
//! // Remove included transactions after block is produced
//! let hashes: Vec<_> = txs.iter().map(|tx| tx.hash()).collect();
//! mempool.write().await.remove_many(&hashes);
//! ```

mod error;
mod pool;
mod tx;

pub use error::{MempoolError, MempoolResult};
pub use pool::{new_shared_mempool, new_shared_mempool_with_base_fee, Mempool, SharedMempool};
pub use tx::{MempoolTransaction, EVM_CALL_FUNCTION_ID};
