//! In-memory LRU cache for recent chain data.
//!
//! Provides fast access to recent blocks, transactions, and receipts without
//! hitting persistent storage.

use std::num::NonZeroUsize;
use std::sync::Arc;

use alloy_primitives::B256;
use lru::LruCache;
use parking_lot::RwLock;

use crate::types::{StoredBlock, StoredReceipt, StoredTransaction};

/// Default number of blocks to cache.
const DEFAULT_BLOCK_CACHE_SIZE: usize = 128;
/// Default number of transactions to cache.
const DEFAULT_TX_CACHE_SIZE: usize = 4096;
/// Default number of receipts to cache.
const DEFAULT_RECEIPT_CACHE_SIZE: usize = 4096;

/// Configuration for the chain cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Number of blocks to cache.
    pub block_cache_size: usize,
    /// Number of transactions to cache.
    pub tx_cache_size: usize,
    /// Number of receipts to cache.
    pub receipt_cache_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            block_cache_size: DEFAULT_BLOCK_CACHE_SIZE,
            tx_cache_size: DEFAULT_TX_CACHE_SIZE,
            receipt_cache_size: DEFAULT_RECEIPT_CACHE_SIZE,
        }
    }
}

/// In-memory LRU cache for chain data.
pub struct ChainCache {
    /// Blocks by number.
    blocks_by_number: RwLock<LruCache<u64, Arc<StoredBlock>>>,
    /// Block number by hash.
    block_number_by_hash: RwLock<LruCache<B256, u64>>,
    /// Transactions by hash.
    transactions: RwLock<LruCache<B256, Arc<StoredTransaction>>>,
    /// Receipts by transaction hash.
    receipts: RwLock<LruCache<B256, Arc<StoredReceipt>>>,
    /// Latest block number.
    latest_block: RwLock<Option<u64>>,
}

impl ChainCache {
    /// Create a new cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            blocks_by_number: RwLock::new(LruCache::new(
                NonZeroUsize::new(config.block_cache_size).unwrap(),
            )),
            block_number_by_hash: RwLock::new(LruCache::new(
                NonZeroUsize::new(config.block_cache_size).unwrap(),
            )),
            transactions: RwLock::new(LruCache::new(
                NonZeroUsize::new(config.tx_cache_size).unwrap(),
            )),
            receipts: RwLock::new(LruCache::new(
                NonZeroUsize::new(config.receipt_cache_size).unwrap(),
            )),
            latest_block: RwLock::new(None),
        }
    }

    /// Create a cache with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Get the latest block number.
    pub fn latest_block_number(&self) -> Option<u64> {
        *self.latest_block.read()
    }

    /// Set the latest block number.
    pub fn set_latest_block_number(&self, number: u64) {
        let mut latest = self.latest_block.write();
        if latest.is_none_or(|n| number > n) {
            *latest = Some(number);
        }
    }

    /// Insert a block into the cache.
    pub fn insert_block(&self, block: StoredBlock) {
        let number = block.number;
        let hash = block.hash;
        let block = Arc::new(block);

        self.blocks_by_number.write().put(number, block);
        self.block_number_by_hash.write().put(hash, number);
        self.set_latest_block_number(number);
    }

    /// Get a block by number.
    pub fn get_block_by_number(&self, number: u64) -> Option<Arc<StoredBlock>> {
        self.blocks_by_number.write().get(&number).cloned()
    }

    /// Get a block by hash.
    pub fn get_block_by_hash(&self, hash: B256) -> Option<Arc<StoredBlock>> {
        let number = self.block_number_by_hash.write().get(&hash).copied()?;
        self.get_block_by_number(number)
    }

    /// Get block number by hash.
    pub fn get_block_number_by_hash(&self, hash: B256) -> Option<u64> {
        self.block_number_by_hash.write().get(&hash).copied()
    }

    /// Insert a transaction into the cache.
    pub fn insert_transaction(&self, tx: StoredTransaction) {
        let hash = tx.hash;
        self.transactions.write().put(hash, Arc::new(tx));
    }

    /// Get a transaction by hash.
    pub fn get_transaction(&self, hash: B256) -> Option<Arc<StoredTransaction>> {
        self.transactions.write().get(&hash).cloned()
    }

    /// Insert a receipt into the cache.
    pub fn insert_receipt(&self, receipt: StoredReceipt) {
        let hash = receipt.transaction_hash;
        self.receipts.write().put(hash, Arc::new(receipt));
    }

    /// Get a receipt by transaction hash.
    pub fn get_receipt(&self, tx_hash: B256) -> Option<Arc<StoredReceipt>> {
        self.receipts.write().get(&tx_hash).cloned()
    }

    /// Insert a full block with all transactions and receipts.
    pub fn insert_block_with_data(
        &self,
        block: StoredBlock,
        transactions: Vec<StoredTransaction>,
        receipts: Vec<StoredReceipt>,
    ) {
        self.insert_block(block);
        for tx in transactions {
            self.insert_transaction(tx);
        }
        for receipt in receipts {
            self.insert_receipt(receipt);
        }
    }

    /// Clear all cached data.
    pub fn clear(&self) {
        self.blocks_by_number.write().clear();
        self.block_number_by_hash.write().clear();
        self.transactions.write().clear();
        self.receipts.write().clear();
        *self.latest_block.write() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;

    fn make_test_block(number: u64) -> StoredBlock {
        StoredBlock {
            number,
            hash: B256::from([number as u8; 32]),
            parent_hash: B256::from([(number.saturating_sub(1)) as u8; 32]),
            state_root: B256::ZERO,
            transactions_root: B256::ZERO,
            receipts_root: B256::ZERO,
            timestamp: 1000 + number,
            gas_used: 21000 * number,
            gas_limit: 30_000_000,
            transaction_count: number as u32,
            miner: Address::ZERO,
            extra_data: alloy_primitives::Bytes::new(),
        }
    }

    #[test]
    fn test_block_cache() {
        let cache = ChainCache::with_defaults();

        let block = make_test_block(1);
        let hash = block.hash;
        cache.insert_block(block);

        assert_eq!(cache.latest_block_number(), Some(1));
        assert!(cache.get_block_by_number(1).is_some());
        assert!(cache.get_block_by_hash(hash).is_some());
        assert!(cache.get_block_by_number(2).is_none());
    }

    #[test]
    fn test_latest_block_tracking() {
        let cache = ChainCache::with_defaults();

        cache.insert_block(make_test_block(5));
        assert_eq!(cache.latest_block_number(), Some(5));

        cache.insert_block(make_test_block(3));
        // Should not go backwards
        assert_eq!(cache.latest_block_number(), Some(5));

        cache.insert_block(make_test_block(10));
        assert_eq!(cache.latest_block_number(), Some(10));
    }
}
