//! Cache warming infrastructure for prefetching storage keys.
//!
//! Cache warming improves performance by predicting which storage keys
//! a transaction will access and prefetching them before execution.
//!
//! # Usage
//!
//! 1. Implement `KeyPredictor` for your transaction type
//! 2. Call `warm_cache` before executing a batch of transactions
//! 3. The cache will be populated with predicted values

use crate::cache::ShardedDbCache;
use evolve_core::ReadonlyKV;
use std::sync::Arc;

/// Trait for predicting which storage keys a transaction will access.
///
/// Implement this trait for your transaction type to enable cache warming.
/// The predictions don't need to be perfect - false positives just waste
/// some prefetch work, while false negatives result in cache misses.
pub trait KeyPredictor<Tx> {
    /// Predicts the storage keys that a transaction is likely to access.
    ///
    /// # Arguments
    /// * `tx` - The transaction to predict keys for
    ///
    /// # Returns
    /// A vector of predicted storage keys
    fn predict_keys(&self, tx: &Tx) -> Vec<Vec<u8>>;
}

/// A no-op key predictor that predicts no keys.
/// Use this when you don't want cache warming.
#[derive(Default, Clone, Copy)]
pub struct NoOpKeyPredictor;

impl<Tx> KeyPredictor<Tx> for NoOpKeyPredictor {
    fn predict_keys(&self, _tx: &Tx) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

/// Warms the cache by prefetching predicted keys for a batch of transactions.
///
/// This function:
/// 1. Collects all predicted keys from all transactions
/// 2. Deduplicates the keys
/// 3. Prefetches them into the cache in parallel (grouped by shard)
///
/// # Arguments
/// * `cache` - The sharded cache to warm
/// * `storage` - The underlying storage to fetch from
/// * `txs` - The transactions to warm the cache for
/// * `predictor` - The key predictor implementation
///
/// # Type Parameters
/// * `Tx` - The transaction type
/// * `S` - The storage backend type
/// * `P` - The key predictor type
pub fn warm_cache<Tx, S, P>(cache: &ShardedDbCache, storage: &S, txs: &[Tx], predictor: &P)
where
    S: ReadonlyKV + Sync,
    P: KeyPredictor<Tx>,
{
    // Collect all predicted keys
    let mut all_keys: Vec<Vec<u8>> = txs
        .iter()
        .flat_map(|tx| predictor.predict_keys(tx))
        .collect();

    // Deduplicate keys
    all_keys.sort();
    all_keys.dedup();

    if all_keys.is_empty() {
        return;
    }

    // Prefetch into cache
    cache.prefetch_keys(&all_keys, |key| {
        // Fetch from storage, ignoring errors (treat as absent)
        storage.get(key).ok().flatten()
    });
}

/// Warms the cache asynchronously for a batch of transactions.
///
/// This is useful when the storage backend is async and you want to
/// avoid blocking the executor.
pub async fn warm_cache_async<Tx, S, P>(
    cache: Arc<ShardedDbCache>,
    storage: Arc<S>,
    txs: Vec<Tx>,
    predictor: Arc<P>,
) where
    Tx: Send + Sync + 'static,
    S: ReadonlyKV + Send + Sync + 'static,
    P: KeyPredictor<Tx> + Send + Sync + 'static,
{
    // Spawn blocking task to avoid blocking the async runtime
    tokio::task::spawn_blocking(move || {
        warm_cache(&cache, storage.as_ref(), &txs, predictor.as_ref());
    })
    .await
    .ok();
}

/// Common key patterns for blockchain transactions.
///
/// These helper functions generate common storage key patterns that
/// can be used in KeyPredictor implementations.
pub mod patterns {
    use evolve_core::AccountId;

    /// Generates a key for an account's balance of a specific token.
    ///
    /// # Arguments
    /// * `token_account` - The token account ID
    /// * `holder` - The account holding the balance
    pub fn balance_key(token_account: AccountId, holder: AccountId) -> Vec<u8> {
        let mut key = Vec::with_capacity(32 + 16);
        key.extend_from_slice(&token_account.as_bytes());
        key.extend_from_slice(b"balances");
        key.extend_from_slice(&holder.as_bytes());
        key
    }

    /// Generates a key for an account's nonce (sequence number).
    ///
    /// # Arguments
    /// * `account` - The account ID
    pub fn nonce_key(account: AccountId) -> Vec<u8> {
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(&account.as_bytes());
        key.extend_from_slice(b"nonce");
        key
    }

    /// Generates a key for account code identifier lookup.
    ///
    /// # Arguments
    /// * `account` - The account ID
    pub fn code_id_key(account: AccountId) -> Vec<u8> {
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(&account.as_bytes());
        key.extend_from_slice(b"code_id");
        key
    }

    /// Generates keys for a typical token transfer transaction.
    ///
    /// Returns keys for:
    /// - Sender's balance
    /// - Recipient's balance
    /// - Sender's nonce
    ///
    /// # Arguments
    /// * `token` - The token account ID
    /// * `sender` - The sender's account ID
    /// * `recipient` - The recipient's account ID
    pub fn token_transfer_keys(
        token: AccountId,
        sender: AccountId,
        recipient: AccountId,
    ) -> Vec<Vec<u8>> {
        vec![
            balance_key(token, sender),
            balance_key(token, recipient),
            nonce_key(sender),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedValue;
    use evolve_core::ErrorCode;
    use hashbrown::HashMap;
    use std::sync::RwLock;

    /// Mock storage for testing
    struct MockStorage {
        data: RwLock<HashMap<Vec<u8>, Vec<u8>>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                data: RwLock::new(HashMap::new()),
            }
        }

        fn insert(&self, key: Vec<u8>, value: Vec<u8>) {
            self.data.write().unwrap().insert(key, value);
        }
    }

    impl ReadonlyKV for MockStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
            Ok(self.data.read().unwrap().get(key).cloned())
        }
    }

    /// Simple test transaction
    struct TestTx {
        keys: Vec<Vec<u8>>,
    }

    /// Test predictor that returns the transaction's stored keys
    struct TestPredictor;

    impl KeyPredictor<TestTx> for TestPredictor {
        fn predict_keys(&self, tx: &TestTx) -> Vec<Vec<u8>> {
            tx.keys.clone()
        }
    }

    #[test]
    fn test_warm_cache_basic() {
        let cache = ShardedDbCache::with_defaults();
        let storage = MockStorage::new();

        // Add some data to storage
        storage.insert(b"key1".to_vec(), b"value1".to_vec());
        storage.insert(b"key2".to_vec(), b"value2".to_vec());

        // Create transactions that will access these keys
        let txs = vec![
            TestTx {
                keys: vec![b"key1".to_vec()],
            },
            TestTx {
                keys: vec![b"key2".to_vec(), b"key3".to_vec()], // key3 doesn't exist
            },
        ];

        // Warm the cache
        warm_cache(&cache, &storage, &txs, &TestPredictor);

        // Check that keys are cached
        match cache.get(b"key1") {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value1"),
            _ => panic!("key1 should be cached as present"),
        }

        match cache.get(b"key2") {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value2"),
            _ => panic!("key2 should be cached as present"),
        }

        match cache.get(b"key3") {
            Some(CachedValue::Absent) => {}
            _ => panic!("key3 should be cached as absent"),
        }
    }

    #[test]
    fn test_warm_cache_deduplication() {
        let cache = ShardedDbCache::with_defaults();
        let storage = MockStorage::new();
        storage.insert(b"key1".to_vec(), b"value1".to_vec());

        // Multiple transactions accessing the same key
        let txs = vec![
            TestTx {
                keys: vec![b"key1".to_vec()],
            },
            TestTx {
                keys: vec![b"key1".to_vec()],
            },
            TestTx {
                keys: vec![b"key1".to_vec()],
            },
        ];

        // This should still work correctly with deduplication
        warm_cache(&cache, &storage, &txs, &TestPredictor);

        match cache.get(b"key1") {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value1"),
            _ => panic!("key1 should be cached"),
        }
    }

    #[test]
    fn test_warm_cache_empty_transactions() {
        let cache = ShardedDbCache::with_defaults();
        let storage = MockStorage::new();
        let txs: Vec<TestTx> = vec![];

        // Should not panic with empty transaction list
        warm_cache(&cache, &storage, &txs, &TestPredictor);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_noop_predictor() {
        let cache = ShardedDbCache::with_defaults();
        let storage = MockStorage::new();
        storage.insert(b"key1".to_vec(), b"value1".to_vec());

        let txs = vec![TestTx {
            keys: vec![b"key1".to_vec()],
        }];

        // NoOp predictor should not warm any keys
        warm_cache(&cache, &storage, &txs, &NoOpKeyPredictor);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_key_patterns() {
        use evolve_core::AccountId;
        use patterns::*;

        let token = AccountId::from_u64(1);
        let sender = AccountId::from_u64(2);
        let recipient = AccountId::from_u64(3);

        // Just verify the functions don't panic and return non-empty keys
        assert!(!balance_key(token, sender).is_empty());
        assert!(!nonce_key(sender).is_empty());
        assert!(!code_id_key(sender).is_empty());

        let keys = token_transfer_keys(token, sender, recipient);
        assert_eq!(keys.len(), 3);
    }
}
