//! Generic in-memory transaction mempool for Evolve.
//!
//! This crate provides a thread-safe, generic mempool that supports multiple
//! transaction types with configurable ordering strategies.
//!
//! # Features
//!
//! - Generic `Mempool<T>` where `T: MempoolTx` defines the transaction type
//! - Configurable ordering via the `MempoolTx::OrderingKey` associated type
//! - Built-in ordering strategies: `GasPriceOrdering` (ETH), `FifoOrdering` (micro)
//! - `MempoolOps` trait for custom mempool implementations
//! - Thread-safe via `Arc<RwLock<M>>` where `M: MempoolOps<T>`
//!
//! # Usage
//!
//! ```ignore
//! use evolve_mempool::{Mempool, MempoolTx, MempoolOps, GasPriceOrdering};
//!
//! // Implement MempoolTx for your transaction type
//! impl MempoolTx for MyTx {
//!     type OrderingKey = GasPriceOrdering;
//!
//!     fn tx_id(&self) -> [u8; 32] { ... }
//!     fn ordering_key(&self) -> Self::OrderingKey { ... }
//! }
//!
//! // Create and use the mempool
//! let mut pool: Mempool<MyTx> = Mempool::new();
//! pool.add(verified_tx)?;
//! let txs = pool.select(100);
//! ```
//!
//! # Custom Mempool Implementations
//!
//! Implement `MempoolOps<T>` for custom backends (Redis, persistent, etc.):
//!
//! ```ignore
//! impl MempoolOps<MyTx> for MyCustomMempool {
//!     fn add(&mut self, tx: MyTx) -> Result<[u8; 32], MempoolError> { ... }
//!     fn select(&mut self, limit: usize) -> Vec<Arc<MyTx>> { ... }
//!     // ... other methods
//! }
//! ```
//!
//! # Architecture
//!
//! Transaction decoding and verification should happen in a "gateway" layer
//! (e.g., `EthGateway` in `evolve_tx_eth`) before adding to the mempool.
//! The mempool only handles storage and ordering of already-verified transactions.

mod error;
mod pool;
mod traits;

pub use error::{MempoolError, MempoolResult};
pub use pool::{new_shared_mempool, shared_mempool_from, Mempool, SharedMempool};
pub use traits::{FifoOrdering, GasPriceOrdering, MempoolOps, MempoolTx};
