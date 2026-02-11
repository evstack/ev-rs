//! Traits and types for generic mempool support.
//!
//! This module defines:
//! - `MempoolTx` - trait for transactions that can be stored in a mempool
//! - `MempoolOps` - trait for mempool implementations (allows custom backends)
//! - `GasPriceOrdering` / `FifoOrdering` - ordering strategies for different tx types

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::sync::Arc;

use crate::error::MempoolError;

/// Maximum length for sender keys used in mempool indexing.
///
/// This keeps sender tracking allocation-free while allowing different account
/// address formats.
pub const MAX_SENDER_KEY_LEN: usize = 64;

/// Bounded sender key used for per-sender mempool tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SenderKey {
    len: u8,
    bytes: [u8; MAX_SENDER_KEY_LEN],
}

impl SenderKey {
    /// Build a sender key from bytes if it fits the configured bound.
    pub fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_SENDER_KEY_LEN {
            return None;
        }
        let len = u8::try_from(bytes.len()).ok()?;
        let mut key_bytes = [0u8; MAX_SENDER_KEY_LEN];
        key_bytes[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            len,
            bytes: key_bytes,
        })
    }

    /// Borrow the sender key bytes.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Trait for transactions that can be stored in a mempool.
///
/// The associated `OrderingKey` determines priority ordering.
/// Higher keys (per Ord) = higher priority in the max-heap.
pub trait MempoolTx: Clone + Send + Sync + 'static {
    /// The key type used for priority ordering.
    ///
    /// The `Ord` implementation defines ordering behavior:
    /// - For gas-price ordering: higher gas price = higher key
    /// - For FIFO ordering: older timestamp = higher key (via `Reverse`)
    type OrderingKey: Ord + Clone + Send + Sync;

    /// Unique transaction identifier (32-byte hash).
    fn tx_id(&self) -> [u8; 32];

    /// Extract the ordering key for priority queue placement.
    ///
    /// Higher keys have higher priority in the max-heap.
    fn ordering_key(&self) -> Self::OrderingKey;

    /// Optional sender key for per-sender tracking.
    ///
    /// Returns `None` if per-sender tracking is not needed.
    fn sender_key(&self) -> Option<SenderKey> {
        None
    }

    /// Gas limit for this transaction.
    ///
    /// Used by `select_with_gas_budget` to fill blocks up to a gas limit.
    fn gas_limit(&self) -> u64;
}

/// Trait for mempool implementations.
///
/// This trait abstracts mempool operations, allowing different backends:
/// - In-memory (default `Mempool<T>`)
/// - Redis-backed
/// - Persistent/journaled
/// - Custom priority logic
///
/// Implementations should be `Send + Sync` to allow shared access via `Arc<RwLock<M>>`.
pub trait MempoolOps<Tx: MempoolTx>: Send + Sync {
    /// Add a verified transaction to the mempool.
    ///
    /// Returns the transaction ID (hash) on success.
    /// Returns `MempoolError::AlreadyExists` if the transaction is already in the pool.
    fn add(&mut self, tx: Tx) -> Result<[u8; 32], MempoolError>;

    /// Select up to `limit` transactions for block inclusion.
    ///
    /// Returns transactions ordered by priority (implementation-defined).
    /// The returned transactions remain in the mempool until explicitly removed.
    fn select(&mut self, limit: usize) -> Vec<Arc<Tx>>;

    /// Select transactions for block inclusion up to gas and count limits.
    ///
    /// Selects transactions in priority order until either:
    /// - The cumulative gas would exceed `max_gas`
    /// - The count reaches `max_txs`
    /// - The mempool is exhausted
    ///
    /// Transactions that exceed the remaining gas budget are skipped (not selected),
    /// allowing smaller transactions to fill the remaining space.
    ///
    /// # Arguments
    /// * `max_gas` - Maximum cumulative gas (0 means no gas limit)
    /// * `max_txs` - Maximum number of transactions (0 means no count limit)
    ///
    /// # Returns
    /// Tuple of (selected transactions, total gas of selected transactions)
    fn select_with_gas_budget(&mut self, max_gas: u64, max_txs: usize) -> (Vec<Arc<Tx>>, u64);

    /// Remove multiple transactions by their hashes.
    ///
    /// Silently ignores hashes that don't exist in the mempool.
    fn remove_many(&mut self, hashes: &[[u8; 32]]);

    /// Get a transaction by its hash.
    fn get(&self, hash: &[u8; 32]) -> Option<Arc<Tx>>;

    /// Check if a transaction exists in the mempool.
    fn contains(&self, hash: &[u8; 32]) -> bool;

    /// Get the number of pending transactions.
    fn len(&self) -> usize;

    /// Check if the mempool is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove a single transaction by hash.
    ///
    /// Returns the removed transaction if it existed.
    fn remove(&mut self, hash: &[u8; 32]) -> Option<Arc<Tx>>;

    /// Clear all transactions from the mempool.
    fn clear(&mut self);
}

/// Ordering key for Ethereum transactions: higher gas price = higher priority.
///
/// Tie-breaking: for equal gas prices, lower nonce comes first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GasPriceOrdering {
    /// Effective gas price (determines primary ordering).
    pub gas_price: u128,
    /// Nonce for tie-breaking (lower nonce = higher priority).
    pub nonce: u64,
}

impl GasPriceOrdering {
    /// Create a new gas price ordering key.
    #[inline]
    pub fn new(gas_price: u128, nonce: u64) -> Self {
        Self { gas_price, nonce }
    }
}

impl PartialOrd for GasPriceOrdering {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GasPriceOrdering {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher gas price = higher priority
        self.gas_price
            .cmp(&other.gas_price)
            // For same gas price, lower nonce = higher priority (reversed for max-heap)
            .then_with(|| other.nonce.cmp(&self.nonce))
    }
}

/// Ordering key for FIFO transactions: older timestamp = higher priority.
///
/// Uses `Reverse<u64>` so that smaller timestamps (older) compare greater.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FifoOrdering(pub Reverse<u64>);

impl FifoOrdering {
    /// Create a new FIFO ordering key from a timestamp.
    ///
    /// Older timestamps (smaller values) will have higher priority.
    #[inline]
    pub fn new(timestamp: u64) -> Self {
        Self(Reverse(timestamp))
    }

    /// Get the underlying timestamp.
    #[inline]
    pub fn timestamp(&self) -> u64 {
        self.0 .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_price_ordering_higher_price_first() {
        let high = GasPriceOrdering::new(100, 0);
        let low = GasPriceOrdering::new(50, 0);

        // Higher gas price should be greater (higher priority in max-heap)
        assert!(high > low);
    }

    #[test]
    fn test_gas_price_ordering_same_price_lower_nonce_first() {
        let nonce_0 = GasPriceOrdering::new(100, 0);
        let nonce_1 = GasPriceOrdering::new(100, 1);

        // Same gas price, lower nonce should be greater (higher priority)
        assert!(nonce_0 > nonce_1);
    }

    #[test]
    fn test_gas_price_ordering_equality() {
        let a = GasPriceOrdering::new(100, 5);
        let b = GasPriceOrdering::new(100, 5);

        assert_eq!(a, b);
        assert!(a.cmp(&b) == Ordering::Equal);
    }

    #[test]
    fn test_fifo_ordering_older_first() {
        let older = FifoOrdering::new(1000);
        let newer = FifoOrdering::new(2000);

        // Older timestamp (smaller) should be greater (higher priority in max-heap)
        assert!(older > newer);
    }

    #[test]
    fn test_fifo_ordering_equality() {
        let a = FifoOrdering::new(1000);
        let b = FifoOrdering::new(1000);

        assert_eq!(a, b);
    }

    #[test]
    fn test_fifo_timestamp_accessor() {
        let ordering = FifoOrdering::new(12345);
        assert_eq!(ordering.timestamp(), 12345);
    }
}
