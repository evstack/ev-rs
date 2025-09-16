//! Chain index trait and persistent implementation.
//!
//! The `ChainIndex` trait defines operations for storing and retrieving chain data.
//! `PersistentChainIndex` implements this trait using the storage layer.
//!
//! Note: This uses synchronous methods to avoid Send bound issues with the underlying
//! storage. Reads are already synchronous, and writes use `futures::executor::block_on`.

use std::sync::Arc;

use alloy_primitives::B256;

use crate::cache::ChainCache;
use crate::error::{ChainIndexError, ChainIndexResult};
use crate::types::{StoredBlock, StoredLog, StoredReceipt, StoredTransaction, TxLocation};
use evolve_storage::Storage;

/// Storage key prefixes for chain data.
mod keys {
    use alloy_primitives::B256;

    /// Block header by number: `blk:h:{number}` -> StoredBlock
    pub const BLOCK_HEADER: &[u8] = b"blk:h:";
    /// Block number by hash: `blk:n:{hash}` -> u64
    pub const BLOCK_NUMBER: &[u8] = b"blk:n:";
    /// Transaction hashes in block: `blk:t:{number}` -> Vec<B256>
    pub const BLOCK_TXS: &[u8] = b"blk:t:";
    /// Transaction data by hash: `tx:d:{hash}` -> StoredTransaction
    pub const TX_DATA: &[u8] = b"tx:d:";
    /// Transaction location: `tx:l:{hash}` -> TxLocation
    pub const TX_LOCATION: &[u8] = b"tx:l:";
    /// Receipt by tx hash: `tx:r:{hash}` -> StoredReceipt
    pub const TX_RECEIPT: &[u8] = b"tx:r:";
    /// Logs by block: `log:b:{number}` -> Vec<StoredLog>
    pub const LOGS_BY_BLOCK: &[u8] = b"log:b:";
    /// Latest block number: `meta:latest` -> u64
    pub const META_LATEST: &[u8] = b"meta:latest";

    pub fn block_header_key(number: u64) -> Vec<u8> {
        let mut key = BLOCK_HEADER.to_vec();
        key.extend_from_slice(&number.to_be_bytes());
        key
    }

    pub fn block_number_key(hash: &B256) -> Vec<u8> {
        let mut key = BLOCK_NUMBER.to_vec();
        key.extend_from_slice(hash.as_slice());
        key
    }

    pub fn block_txs_key(number: u64) -> Vec<u8> {
        let mut key = BLOCK_TXS.to_vec();
        key.extend_from_slice(&number.to_be_bytes());
        key
    }

    pub fn tx_data_key(hash: &B256) -> Vec<u8> {
        let mut key = TX_DATA.to_vec();
        key.extend_from_slice(hash.as_slice());
        key
    }

    pub fn tx_location_key(hash: &B256) -> Vec<u8> {
        let mut key = TX_LOCATION.to_vec();
        key.extend_from_slice(hash.as_slice());
        key
    }

    pub fn tx_receipt_key(hash: &B256) -> Vec<u8> {
        let mut key = TX_RECEIPT.to_vec();
        key.extend_from_slice(hash.as_slice());
        key
    }

    pub fn logs_by_block_key(number: u64) -> Vec<u8> {
        let mut key = LOGS_BY_BLOCK.to_vec();
        key.extend_from_slice(&number.to_be_bytes());
        key
    }
}

/// Trait for chain data indexing operations.
///
/// All methods are synchronous to avoid Send bound issues with the storage layer.
pub trait ChainIndex: Send + Sync {
    /// Get the latest indexed block number.
    fn latest_block_number(&self) -> ChainIndexResult<Option<u64>>;

    /// Get a block by number.
    fn get_block(&self, number: u64) -> ChainIndexResult<Option<StoredBlock>>;

    /// Get a block by hash.
    fn get_block_by_hash(&self, hash: B256) -> ChainIndexResult<Option<StoredBlock>>;

    /// Get block number by hash.
    fn get_block_number(&self, hash: B256) -> ChainIndexResult<Option<u64>>;

    /// Get transaction hashes in a block.
    fn get_block_transactions(&self, number: u64) -> ChainIndexResult<Vec<B256>>;

    /// Get a transaction by hash.
    fn get_transaction(&self, hash: B256) -> ChainIndexResult<Option<StoredTransaction>>;

    /// Get transaction location (block number and index).
    fn get_transaction_location(&self, hash: B256) -> ChainIndexResult<Option<TxLocation>>;

    /// Get a transaction receipt by hash.
    fn get_receipt(&self, hash: B256) -> ChainIndexResult<Option<StoredReceipt>>;

    /// Get logs for a block.
    fn get_logs_by_block(&self, number: u64) -> ChainIndexResult<Vec<StoredLog>>;

    /// Store a block with its transactions and receipts.
    /// This is the only potentially slow operation as it writes to storage.
    fn store_block(
        &self,
        block: StoredBlock,
        transactions: Vec<StoredTransaction>,
        receipts: Vec<StoredReceipt>,
    ) -> ChainIndexResult<()>;
}

/// Persistent chain index backed by storage.
pub struct PersistentChainIndex<S: Storage> {
    storage: Arc<S>,
    cache: ChainCache,
}

impl<S: Storage> PersistentChainIndex<S> {
    /// Create a new persistent chain index.
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            cache: ChainCache::with_defaults(),
        }
    }

    /// Create with custom cache configuration.
    pub fn with_cache(storage: Arc<S>, cache: ChainCache) -> Self {
        Self { storage, cache }
    }

    /// Get direct access to the cache.
    pub fn cache(&self) -> &ChainCache {
        &self.cache
    }

    /// Initialize from storage, loading the latest block number.
    pub fn initialize(&self) -> ChainIndexResult<()> {
        if let Some(latest) = self.load_latest_block_number()? {
            self.cache.set_latest_block_number(latest);
            log::info!("Chain index initialized at block {}", latest);
        } else {
            log::info!("Chain index initialized (empty)");
        }
        Ok(())
    }

    fn load_latest_block_number(&self) -> ChainIndexResult<Option<u64>> {
        let value = self.storage.get(keys::META_LATEST)?;
        match value {
            Some(bytes) if bytes.len() == 8 => {
                let arr: [u8; 8] = bytes.try_into().unwrap();
                Ok(Some(u64::from_be_bytes(arr)))
            }
            Some(_) => Err(ChainIndexError::Deserialization(
                "invalid latest block format".to_string(),
            )),
            None => Ok(None),
        }
    }
}

impl<S: Storage + 'static> ChainIndex for PersistentChainIndex<S> {
    fn latest_block_number(&self) -> ChainIndexResult<Option<u64>> {
        // Check cache first
        if let Some(n) = self.cache.latest_block_number() {
            return Ok(Some(n));
        }
        // Fall back to storage
        self.load_latest_block_number()
    }

    fn get_block(&self, number: u64) -> ChainIndexResult<Option<StoredBlock>> {
        // Check cache first
        if let Some(block) = self.cache.get_block_by_number(number) {
            return Ok(Some((*block).clone()));
        }

        // Load from storage (synchronous)
        let key = keys::block_header_key(number);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let block: StoredBlock = serde_json::from_slice(&bytes)?;
                // Populate cache
                self.cache.insert_block(block.clone());
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    fn get_block_by_hash(&self, hash: B256) -> ChainIndexResult<Option<StoredBlock>> {
        // Check cache first
        if let Some(block) = self.cache.get_block_by_hash(hash) {
            return Ok(Some((*block).clone()));
        }

        // Look up block number by hash
        let number = match self.get_block_number(hash)? {
            Some(n) => n,
            None => return Ok(None),
        };

        self.get_block(number)
    }

    fn get_block_number(&self, hash: B256) -> ChainIndexResult<Option<u64>> {
        // Check cache first
        if let Some(n) = self.cache.get_block_number_by_hash(hash) {
            return Ok(Some(n));
        }

        // Load from storage
        let key = keys::block_number_key(&hash);
        match self.storage.get(&key)? {
            Some(bytes) if bytes.len() == 8 => {
                let arr: [u8; 8] = bytes.try_into().unwrap();
                Ok(Some(u64::from_be_bytes(arr)))
            }
            Some(_) => Err(ChainIndexError::Deserialization(
                "invalid block number format".to_string(),
            )),
            None => Ok(None),
        }
    }

    fn get_block_transactions(&self, number: u64) -> ChainIndexResult<Vec<B256>> {
        let key = keys::block_txs_key(number);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let hashes: Vec<B256> = serde_json::from_slice(&bytes)?;
                Ok(hashes)
            }
            None => Ok(vec![]),
        }
    }

    fn get_transaction(&self, hash: B256) -> ChainIndexResult<Option<StoredTransaction>> {
        // Check cache first
        if let Some(tx) = self.cache.get_transaction(hash) {
            return Ok(Some((*tx).clone()));
        }

        // Load from storage
        let key = keys::tx_data_key(&hash);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let tx: StoredTransaction = serde_json::from_slice(&bytes)?;
                // Populate cache
                self.cache.insert_transaction(tx.clone());
                Ok(Some(tx))
            }
            None => Ok(None),
        }
    }

    fn get_transaction_location(&self, hash: B256) -> ChainIndexResult<Option<TxLocation>> {
        let key = keys::tx_location_key(&hash);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let loc: TxLocation = serde_json::from_slice(&bytes)?;
                Ok(Some(loc))
            }
            None => Ok(None),
        }
    }

    fn get_receipt(&self, hash: B256) -> ChainIndexResult<Option<StoredReceipt>> {
        // Check cache first
        if let Some(receipt) = self.cache.get_receipt(hash) {
            return Ok(Some((*receipt).clone()));
        }

        // Load from storage
        let key = keys::tx_receipt_key(&hash);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let receipt: StoredReceipt = serde_json::from_slice(&bytes)?;
                // Populate cache
                self.cache.insert_receipt(receipt.clone());
                Ok(Some(receipt))
            }
            None => Ok(None),
        }
    }

    fn get_logs_by_block(&self, number: u64) -> ChainIndexResult<Vec<StoredLog>> {
        let key = keys::logs_by_block_key(number);
        match self.storage.get(&key)? {
            Some(bytes) => {
                let logs: Vec<StoredLog> = serde_json::from_slice(&bytes)?;
                Ok(logs)
            }
            None => Ok(vec![]),
        }
    }

    fn store_block(
        &self,
        block: StoredBlock,
        transactions: Vec<StoredTransaction>,
        receipts: Vec<StoredReceipt>,
    ) -> ChainIndexResult<()> {
        let block_number = block.number;
        let block_hash = block.hash;

        // Collect all logs from receipts
        let all_logs: Vec<StoredLog> = receipts.iter().flat_map(|r| r.logs.clone()).collect();

        // Build batch operations
        let mut ops = Vec::new();

        // Store block header
        ops.push(evolve_storage::Operation::Set {
            key: keys::block_header_key(block_number),
            value: serde_json::to_vec(&block)?,
        });

        // Store block number by hash
        ops.push(evolve_storage::Operation::Set {
            key: keys::block_number_key(&block_hash),
            value: block_number.to_be_bytes().to_vec(),
        });

        // Store transaction hashes for this block
        let tx_hashes: Vec<B256> = transactions.iter().map(|tx| tx.hash).collect();
        ops.push(evolve_storage::Operation::Set {
            key: keys::block_txs_key(block_number),
            value: serde_json::to_vec(&tx_hashes)?,
        });

        // Store each transaction
        for (idx, tx) in transactions.iter().enumerate() {
            ops.push(evolve_storage::Operation::Set {
                key: keys::tx_data_key(&tx.hash),
                value: serde_json::to_vec(tx)?,
            });

            ops.push(evolve_storage::Operation::Set {
                key: keys::tx_location_key(&tx.hash),
                value: serde_json::to_vec(&TxLocation {
                    block_number,
                    transaction_index: idx as u32,
                })?,
            });
        }

        // Store each receipt
        for receipt in &receipts {
            ops.push(evolve_storage::Operation::Set {
                key: keys::tx_receipt_key(&receipt.transaction_hash),
                value: serde_json::to_vec(receipt)?,
            });
        }

        // Store logs by block
        if !all_logs.is_empty() {
            ops.push(evolve_storage::Operation::Set {
                key: keys::logs_by_block_key(block_number),
                value: serde_json::to_vec(&all_logs)?,
            });
        }

        // Update latest block number
        ops.push(evolve_storage::Operation::Set {
            key: keys::META_LATEST.to_vec(),
            value: block_number.to_be_bytes().to_vec(),
        });

        // Batch write and commit using block_on for the async operations
        // This is acceptable because writes are infrequent and not latency-sensitive
        futures::executor::block_on(async {
            self.storage
                .batch(ops)
                .await
                .map_err(|e| ChainIndexError::Storage(format!("batch write failed: {:?}", e)))?;
            self.storage
                .commit()
                .await
                .map_err(|e| ChainIndexError::Storage(format!("commit failed: {:?}", e)))?;
            Ok::<_, ChainIndexError>(())
        })?;

        // Update cache
        self.cache
            .insert_block_with_data(block, transactions, receipts);

        log::debug!("Stored block {} with hash {:?}", block_number, block_hash);

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, U256};
    use async_trait::async_trait;
    use evolve_core::ErrorCode;
    use evolve_storage::{CommitHash, Operation};
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// A mock storage implementation for testing.
    pub struct MockStorage {
        data: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
        commit_count: RwLock<u64>,
    }

    impl MockStorage {
        pub fn new() -> Self {
            Self {
                data: RwLock::new(HashMap::new()),
                commit_count: RwLock::new(0),
            }
        }
    }

    impl evolve_core::ReadonlyKV for MockStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
            Ok(self.data.read().unwrap().get(key).cloned())
        }
    }

    #[async_trait(?Send)]
    impl evolve_storage::Storage for MockStorage {
        async fn commit(&self) -> Result<CommitHash, ErrorCode> {
            let mut count = self.commit_count.write().unwrap();
            *count += 1;
            Ok(CommitHash::new([0u8; 32]))
        }

        async fn batch(&self, operations: Vec<Operation>) -> Result<(), ErrorCode> {
            let mut data = self.data.write().unwrap();
            for op in operations {
                match op {
                    Operation::Set { key, value } => {
                        data.insert(key, value);
                    }
                    Operation::Remove { key } => {
                        data.remove(&key);
                    }
                }
            }
            Ok(())
        }
    }

    /// Helper to create a test StoredBlock.
    pub fn make_test_stored_block(number: u64) -> StoredBlock {
        StoredBlock {
            number,
            hash: B256::repeat_byte(number as u8),
            parent_hash: if number > 0 {
                B256::repeat_byte((number - 1) as u8)
            } else {
                B256::ZERO
            },
            state_root: B256::repeat_byte(0xaa),
            transactions_root: B256::repeat_byte(0xbb),
            receipts_root: B256::repeat_byte(0xcc),
            timestamp: 1000 + number * 12,
            gas_used: 21000,
            gas_limit: 30_000_000,
            transaction_count: 1,
            miner: Address::repeat_byte(0xdd),
            extra_data: Bytes::new(),
        }
    }

    /// Helper to create a test StoredTransaction.
    pub fn make_test_stored_transaction(
        hash: B256,
        block_number: u64,
        block_hash: B256,
        index: u32,
    ) -> StoredTransaction {
        StoredTransaction {
            hash,
            block_number,
            block_hash,
            transaction_index: index,
            from: Address::repeat_byte(0x01),
            to: Some(Address::repeat_byte(0x02)),
            value: U256::from(100u64),
            gas: 21000,
            gas_price: U256::ZERO,
            input: Bytes::new(),
            nonce: 0,
            v: 0,
            r: U256::ZERO,
            s: U256::ZERO,
            tx_type: 0,
            chain_id: Some(1),
        }
    }

    /// Helper to create a test StoredReceipt.
    pub fn make_test_stored_receipt(
        tx_hash: B256,
        block_number: u64,
        block_hash: B256,
        index: u32,
        success: bool,
    ) -> StoredReceipt {
        StoredReceipt {
            transaction_hash: tx_hash,
            transaction_index: index,
            block_hash,
            block_number,
            from: Address::repeat_byte(0x01),
            to: Some(Address::repeat_byte(0x02)),
            cumulative_gas_used: 21000 * (index as u64 + 1),
            gas_used: 21000,
            contract_address: None,
            logs: vec![],
            status: if success { 1 } else { 0 },
            tx_type: 0,
        }
    }

    // ==================== Complex behavior tests ====================
    // Note: Basic store/get operations are covered by model-based tests in model_tests module.
    // These tests focus on complex behaviors not easily covered by model tests.

    /// Tests that transaction location uses array index, not the field value.
    /// This is important because TxLocation.transaction_index should reflect
    /// the actual position in the block, not what was passed in StoredTransaction.
    #[test]
    fn test_transaction_location_uses_array_index() {
        let storage = Arc::new(MockStorage::new());
        let index = PersistentChainIndex::new(storage);

        let block = make_test_stored_block(100);
        // Create tx with transaction_index=5, but it will be at array position 0
        let tx_hash = B256::repeat_byte(0x40);
        let tx = make_test_stored_transaction(tx_hash, 100, block.hash, 5);
        let receipt = make_test_stored_receipt(tx_hash, 100, block.hash, 5, true);

        index.store_block(block, vec![tx], vec![receipt]).unwrap();

        // TxLocation should use array position (0), not the field value (5)
        let location = index.get_transaction_location(tx_hash).unwrap().unwrap();
        assert_eq!(location.transaction_index, 0);
    }

    /// Tests that transaction ordering is preserved when storing multiple txs.
    #[test]
    fn test_transaction_ordering_preserved() {
        let storage = Arc::new(MockStorage::new());
        let index = PersistentChainIndex::new(storage);

        let block = make_test_stored_block(100);
        let hashes: Vec<_> = (0..5).map(|i| B256::repeat_byte(0x50 + i)).collect();

        let txs: Vec<_> = hashes
            .iter()
            .enumerate()
            .map(|(i, &hash)| make_test_stored_transaction(hash, 100, block.hash, i as u32))
            .collect();
        let receipts: Vec<_> = hashes
            .iter()
            .enumerate()
            .map(|(i, &hash)| make_test_stored_receipt(hash, 100, block.hash, i as u32, true))
            .collect();

        index.store_block(block, txs, receipts).unwrap();

        let retrieved_hashes = index.get_block_transactions(100).unwrap();
        assert_eq!(retrieved_hashes, hashes);
    }

    /// Tests log aggregation from multiple receipts.
    #[test]
    fn test_logs_aggregated_from_receipts() {
        let storage = Arc::new(MockStorage::new());
        let index = PersistentChainIndex::new(storage);

        let block = make_test_stored_block(100);

        // Two transactions, each with logs
        let tx1_hash = B256::repeat_byte(0x60);
        let tx2_hash = B256::repeat_byte(0x61);

        let tx1 = make_test_stored_transaction(tx1_hash, 100, block.hash, 0);
        let tx2 = make_test_stored_transaction(tx2_hash, 100, block.hash, 1);

        let mut receipt1 = make_test_stored_receipt(tx1_hash, 100, block.hash, 0, true);
        receipt1.logs = vec![StoredLog {
            address: Address::repeat_byte(0x70),
            topics: vec![B256::repeat_byte(0x71)],
            data: Bytes::from(vec![0x01]),
        }];

        let mut receipt2 = make_test_stored_receipt(tx2_hash, 100, block.hash, 1, true);
        receipt2.logs = vec![
            StoredLog {
                address: Address::repeat_byte(0x72),
                topics: vec![],
                data: Bytes::from(vec![0x02]),
            },
            StoredLog {
                address: Address::repeat_byte(0x73),
                topics: vec![],
                data: Bytes::from(vec![0x03]),
            },
        ];

        index
            .store_block(block, vec![tx1, tx2], vec![receipt1, receipt2])
            .unwrap();

        // All 3 logs should be aggregated
        let logs = index.get_logs_by_block(100).unwrap();
        assert_eq!(logs.len(), 3);
    }

    /// Tests cache population after store_block.
    #[test]
    fn test_cache_populated_on_store() {
        let storage = Arc::new(MockStorage::new());
        let index = PersistentChainIndex::new(storage);

        let block = make_test_stored_block(100);
        let block_hash = block.hash;
        let tx_hash = B256::repeat_byte(0x80);
        let tx = make_test_stored_transaction(tx_hash, 100, block.hash, 0);
        let receipt = make_test_stored_receipt(tx_hash, 100, block.hash, 0, true);

        index.store_block(block, vec![tx], vec![receipt]).unwrap();

        // Cache should be populated without hitting storage again
        let cache = index.cache();
        assert!(cache.get_block_by_number(100).is_some());
        assert!(cache.get_block_by_hash(block_hash).is_some());
        assert!(cache.get_transaction(tx_hash).is_some());
        assert!(cache.get_receipt(tx_hash).is_some());
    }

    /// Tests persistence across index instances (simulating restart).
    #[test]
    fn test_persistence_across_restart() {
        let storage = Arc::new(MockStorage::new());

        // First instance stores data
        let index1 = PersistentChainIndex::new(Arc::clone(&storage));
        for i in 0..5 {
            let block = make_test_stored_block(i);
            index1.store_block(block, vec![], vec![]).unwrap();
        }

        // Second instance (simulating restart) should recover state
        let index2 = PersistentChainIndex::new(storage);
        index2.initialize().unwrap();

        assert_eq!(index2.latest_block_number().unwrap(), Some(4));
        assert!(index2.get_block(3).unwrap().is_some());
    }
}

// ==================== Model-based tests ====================
#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod model_tests {
    use super::tests::{
        make_test_stored_block, make_test_stored_receipt, make_test_stored_transaction, MockStorage,
    };
    use super::*;
    use alloy_primitives::Address;
    use proptest::prelude::*;
    use std::collections::HashMap;

    /// A simple reference model for ChainIndex behavior.
    /// This is a straightforward HashMap-based implementation that serves
    /// as the "oracle" to verify the real implementation against.
    #[derive(Debug, Default, Clone)]
    #[allow(dead_code)]
    struct ChainIndexModel {
        blocks_by_number: HashMap<u64, StoredBlock>,
        blocks_by_hash: HashMap<B256, StoredBlock>,
        transactions: HashMap<B256, StoredTransaction>,
        tx_locations: HashMap<B256, TxLocation>,
        receipts: HashMap<B256, StoredReceipt>,
        logs_by_block: HashMap<u64, Vec<StoredLog>>,
        latest_block: Option<u64>,
    }

    #[allow(dead_code)]
    impl ChainIndexModel {
        fn new() -> Self {
            Self::default()
        }

        fn store_block(
            &mut self,
            block: StoredBlock,
            transactions: Vec<StoredTransaction>,
            receipts: Vec<StoredReceipt>,
        ) {
            let block_number = block.number;
            let block_hash = block.hash;

            // Store block
            self.blocks_by_number.insert(block_number, block.clone());
            self.blocks_by_hash.insert(block_hash, block);

            // Store transactions and their locations
            for (idx, tx) in transactions.iter().enumerate() {
                self.transactions.insert(tx.hash, tx.clone());
                self.tx_locations.insert(
                    tx.hash,
                    TxLocation {
                        block_number,
                        transaction_index: idx as u32,
                    },
                );
            }

            // Store receipts and collect logs
            let mut all_logs = Vec::new();
            for receipt in receipts {
                all_logs.extend(receipt.logs.clone());
                self.receipts.insert(receipt.transaction_hash, receipt);
            }
            if !all_logs.is_empty() {
                self.logs_by_block.insert(block_number, all_logs);
            }

            // Update latest block
            self.latest_block = Some(
                self.latest_block
                    .map(|l| l.max(block_number))
                    .unwrap_or(block_number),
            );
        }

        fn get_block(&self, number: u64) -> Option<&StoredBlock> {
            self.blocks_by_number.get(&number)
        }

        fn get_block_by_hash(&self, hash: B256) -> Option<&StoredBlock> {
            self.blocks_by_hash.get(&hash)
        }

        fn get_block_number(&self, hash: B256) -> Option<u64> {
            self.blocks_by_hash.get(&hash).map(|b| b.number)
        }

        fn get_transaction(&self, hash: B256) -> Option<&StoredTransaction> {
            self.transactions.get(&hash)
        }

        fn get_transaction_location(&self, hash: B256) -> Option<&TxLocation> {
            self.tx_locations.get(&hash)
        }

        fn get_receipt(&self, hash: B256) -> Option<&StoredReceipt> {
            self.receipts.get(&hash)
        }

        fn get_logs_by_block(&self, number: u64) -> Vec<StoredLog> {
            self.logs_by_block.get(&number).cloned().unwrap_or_default()
        }

        fn latest_block_number(&self) -> Option<u64> {
            self.latest_block
        }
    }

    /// Operations that can be performed on a ChainIndex.
    #[derive(Debug, Clone)]
    enum Operation {
        StoreBlock {
            block_number: u64,
            tx_count: usize,
        },
        GetBlock {
            number: u64,
        },
        GetBlockByHash {
            /// Index into previously stored blocks (modulo stored count)
            block_idx: usize,
        },
        GetTransaction {
            /// Index into previously stored transactions (modulo stored count)
            tx_idx: usize,
        },
        GetReceipt {
            /// Index into previously stored transactions (modulo stored count)
            tx_idx: usize,
        },
        GetTransactionLocation {
            /// Index into previously stored transactions (modulo stored count)
            tx_idx: usize,
        },
        GetLogsByBlock {
            number: u64,
        },
        GetLatestBlockNumber,
    }

    /// State tracker for generating valid operations.
    #[derive(Debug, Default)]
    struct OperationState {
        stored_block_numbers: Vec<u64>,
        stored_block_hashes: Vec<B256>,
        stored_tx_hashes: Vec<B256>,
    }

    /// Strategy to generate a single operation.
    fn arb_operation(max_block: u64, _max_txs: usize) -> impl Strategy<Value = Operation> {
        prop_oneof![
            // Store block with 0-3 transactions
            (0..max_block, 0..=3usize).prop_map(|(block_number, tx_count)| Operation::StoreBlock {
                block_number,
                tx_count
            }),
            // Get block by number
            (0..max_block).prop_map(|number| Operation::GetBlock { number }),
            // Get block by hash (index into stored blocks)
            (0..10usize).prop_map(|block_idx| Operation::GetBlockByHash { block_idx }),
            // Get transaction
            (0..10usize).prop_map(|tx_idx| Operation::GetTransaction { tx_idx }),
            // Get receipt
            (0..10usize).prop_map(|tx_idx| Operation::GetReceipt { tx_idx }),
            // Get transaction location
            (0..10usize).prop_map(|tx_idx| Operation::GetTransactionLocation { tx_idx }),
            // Get logs by block
            (0..max_block).prop_map(|number| Operation::GetLogsByBlock { number }),
            // Get latest block number
            Just(Operation::GetLatestBlockNumber),
        ]
    }

    /// Strategy to generate a sequence of operations.
    fn arb_operations(count: usize) -> impl Strategy<Value = Vec<Operation>> {
        proptest::collection::vec(arb_operation(20, 5), 1..=count)
    }

    /// Generate test data for a block with transactions.
    fn generate_block_data(
        block_number: u64,
        tx_count: usize,
        tx_counter: &mut u64,
    ) -> (StoredBlock, Vec<StoredTransaction>, Vec<StoredReceipt>) {
        let block = make_test_stored_block(block_number);
        let mut transactions = Vec::new();
        let mut receipts = Vec::new();

        for i in 0..tx_count {
            let tx_id = *tx_counter;
            *tx_counter += 1;

            let tx_hash = B256::from_slice(&{
                let mut bytes = [0u8; 32];
                bytes[0..8].copy_from_slice(&tx_id.to_be_bytes());
                bytes
            });

            let tx = make_test_stored_transaction(tx_hash, block_number, block.hash, i as u32);
            let mut receipt =
                make_test_stored_receipt(tx_hash, block_number, block.hash, i as u32, true);

            // Add a log to every other transaction
            if i % 2 == 0 {
                receipt.logs.push(crate::types::StoredLog {
                    address: Address::repeat_byte((tx_id % 256) as u8),
                    topics: vec![B256::repeat_byte((tx_id % 256) as u8)],
                    data: alloy_primitives::Bytes::from(vec![(tx_id % 256) as u8]),
                });
            }

            transactions.push(tx);
            receipts.push(receipt);
        }

        (block, transactions, receipts)
    }

    proptest! {
        /// Model-based test: verify that PersistentChainIndex behaves identically to the reference model.
        #[test]
        fn prop_chain_index_matches_model(operations in arb_operations(30)) {
            let storage = Arc::new(MockStorage::new());
            let index = PersistentChainIndex::new(storage);
            let mut model = ChainIndexModel::new();
            let mut state = OperationState::default();
            let mut tx_counter = 0u64;

            for op in operations {
                match op {
                    Operation::StoreBlock { block_number, tx_count } => {
                        // Skip if block already stored
                        if state.stored_block_numbers.contains(&block_number) {
                            continue;
                        }

                        let (block, txs, receipts) =
                            generate_block_data(block_number, tx_count, &mut tx_counter);

                        // Track stored data
                        state.stored_block_numbers.push(block_number);
                        state.stored_block_hashes.push(block.hash);
                        for tx in &txs {
                            state.stored_tx_hashes.push(tx.hash);
                        }

                        // Apply to both
                        model.store_block(block.clone(), txs.clone(), receipts.clone());
                        index.store_block(block, txs, receipts).unwrap();
                    }
                    Operation::GetBlock { number } => {
                        let model_result = model.get_block(number);
                        let index_result = index.get_block(number).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.number, i.number);
                                prop_assert_eq!(m.hash, i.hash);
                            }
                            _ => {
                                prop_assert!(false, "Mismatch: model={:?}, index={:?}", model_result.is_some(), index_result.is_some());
                            }
                        }
                    }
                    Operation::GetBlockByHash { block_idx } => {
                        if state.stored_block_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_block_hashes[block_idx % state.stored_block_hashes.len()];

                        let model_result = model.get_block_by_hash(hash);
                        let index_result = index.get_block_by_hash(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.number, i.number);
                                prop_assert_eq!(m.hash, i.hash);
                            }
                            _ => {
                                prop_assert!(false, "GetBlockByHash mismatch");
                            }
                        }
                    }
                    Operation::GetTransaction { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_transaction(hash);
                        let index_result = index.get_transaction(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.hash, i.hash);
                                prop_assert_eq!(m.block_number, i.block_number);
                            }
                            _ => {
                                prop_assert!(false, "GetTransaction mismatch");
                            }
                        }
                    }
                    Operation::GetReceipt { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_receipt(hash);
                        let index_result = index.get_receipt(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.transaction_hash, i.transaction_hash);
                                prop_assert_eq!(m.status, i.status);
                            }
                            _ => {
                                prop_assert!(false, "GetReceipt mismatch");
                            }
                        }
                    }
                    Operation::GetTransactionLocation { tx_idx } => {
                        if state.stored_tx_hashes.is_empty() {
                            continue;
                        }
                        let hash = state.stored_tx_hashes[tx_idx % state.stored_tx_hashes.len()];

                        let model_result = model.get_transaction_location(hash);
                        let index_result = index.get_transaction_location(hash).unwrap();

                        match (model_result, &index_result) {
                            (None, None) => {}
                            (Some(m), Some(i)) => {
                                prop_assert_eq!(m.block_number, i.block_number);
                                prop_assert_eq!(m.transaction_index, i.transaction_index);
                            }
                            _ => {
                                prop_assert!(false, "GetTransactionLocation mismatch");
                            }
                        }
                    }
                    Operation::GetLogsByBlock { number } => {
                        let model_logs = model.get_logs_by_block(number);
                        let index_logs = index.get_logs_by_block(number).unwrap();

                        prop_assert_eq!(model_logs.len(), index_logs.len(), "Log count mismatch for block {}", number);
                        for (m, i) in model_logs.iter().zip(index_logs.iter()) {
                            prop_assert_eq!(m.address, i.address);
                            prop_assert_eq!(&m.topics, &i.topics);
                        }
                    }
                    Operation::GetLatestBlockNumber => {
                        let model_result = model.latest_block_number();
                        let index_result = index.latest_block_number().unwrap();
                        prop_assert_eq!(model_result, index_result, "Latest block number mismatch");
                    }
                }
            }
        }

        /// Test that storing the same block number twice doesn't corrupt state.
        /// The second store overwrites the first.
        #[test]
        fn prop_duplicate_block_storage(block_number in 0u64..100, tx_count1 in 0usize..3, tx_count2 in 0usize..3) {
            let storage = Arc::new(MockStorage::new());
            let index = PersistentChainIndex::new(storage);
            let mut tx_counter = 0u64;

            // Store first version
            let (block1, txs1, receipts1) = generate_block_data(block_number, tx_count1, &mut tx_counter);
            index.store_block(block1, txs1, receipts1).unwrap();

            // Store second version (different hash due to different timestamp in test helper)
            let (block2, txs2, receipts2) = generate_block_data(block_number, tx_count2, &mut tx_counter);
            let block2_hash = block2.hash;
            index.store_block(block2, txs2.clone(), receipts2).unwrap();

            // The block should reflect the second store
            let retrieved = index.get_block(block_number).unwrap().unwrap();
            prop_assert_eq!(retrieved.hash, block2_hash);

            // Transaction count should match second store
            let tx_hashes = index.get_block_transactions(block_number).unwrap();
            prop_assert_eq!(tx_hashes.len(), txs2.len());
        }

        /// Test retrieval of non-existent data never crashes.
        #[test]
        fn prop_missing_data_returns_none(
            block_number in 1000u64..2000,
            tx_hash_bytes in proptest::collection::vec(any::<u8>(), 32)
        ) {
            let storage = Arc::new(MockStorage::new());
            let index = PersistentChainIndex::new(storage);

            let tx_hash = B256::from_slice(&tx_hash_bytes);

            // All queries for non-existent data should return None/empty
            prop_assert!(index.get_block(block_number).unwrap().is_none());
            prop_assert!(index.get_block_by_hash(tx_hash).unwrap().is_none());
            prop_assert!(index.get_transaction(tx_hash).unwrap().is_none());
            prop_assert!(index.get_receipt(tx_hash).unwrap().is_none());
            prop_assert!(index.get_transaction_location(tx_hash).unwrap().is_none());
            prop_assert!(index.get_logs_by_block(block_number).unwrap().is_empty());
        }

        /// Test that block number lookups are consistent with block storage.
        #[test]
        fn prop_block_number_lookup_consistent(blocks in proptest::collection::vec(0u64..50, 1..10)) {
            let storage = Arc::new(MockStorage::new());
            let index = PersistentChainIndex::new(storage);
            let mut tx_counter = 0u64;

            let mut stored_blocks = Vec::new();
            for &block_number in &blocks {
                // Skip duplicates
                if stored_blocks.iter().any(|(n, _)| *n == block_number) {
                    continue;
                }

                let (block, txs, receipts) = generate_block_data(block_number, 0, &mut tx_counter);
                let hash = block.hash;
                index.store_block(block, txs, receipts).unwrap();
                stored_blocks.push((block_number, hash));
            }

            // Verify all lookups are consistent
            for (number, hash) in &stored_blocks {
                // get_block_number(hash) should return the number
                let looked_up_number = index.get_block_number(*hash).unwrap();
                prop_assert_eq!(looked_up_number, Some(*number));

                // get_block(number).hash should equal the stored hash
                let block = index.get_block(*number).unwrap().unwrap();
                prop_assert_eq!(block.hash, *hash);

                // get_block_by_hash(hash).number should equal the stored number
                let block_by_hash = index.get_block_by_hash(*hash).unwrap().unwrap();
                prop_assert_eq!(block_by_hash.number, *number);
            }
        }
    }
}
