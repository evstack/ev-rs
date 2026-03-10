//! QmdbStorage implementation using commonware's Current QMDB.
//!
//! QMDB (Quick Merkle Database) provides:
//! - Historical proofs for any value ever associated with a key
//! - Efficient Merkle root computation
//! - Pruning support for storage management
//!
//! Mutations are expressed as speculative
//! batches that are merkleized before they are applied to the live database.

// Instant is used for performance metrics, not consensus-affecting logic.
#![allow(clippy::disallowed_types)]

use crate::cache::{CachedValue, ShardedDbCache};
use crate::metrics::OptionalMetrics;
use crate::types::{create_storage_key, StorageKey, MAX_VALUE_DATA_SIZE};
use async_trait::async_trait;
use commonware_codec::RangeCfg;
use commonware_cryptography::sha256::Sha256;
use commonware_runtime::{
    buffer::paged::CacheRef, BufferPooler, Clock, Metrics, Storage as RStorage,
};
use commonware_storage::qmdb::current::{unordered::variable::Db, VariableConfig};
use commonware_storage::translator::EightCap;
use evolve_core::{ErrorCode, ReadonlyKV};
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::runtime::RuntimeFlavor;
use tokio::sync::RwLock;

/// Type alias for QMDB in Current state.
/// `N = 64` because SHA256 digests are 32 bytes and QMDB expects `2 * digest_size`.
type QmdbCurrent<C> = Db<C, StorageKey, Vec<u8>, Sha256, EightCap, 64>;

#[derive(Debug)]
struct PreparedBatch {
    updates: Vec<(StorageKey, Option<Vec<u8>>)>,
    keys_to_invalidate: Vec<Vec<u8>>,
    ops_count: usize,
    sets: usize,
    deletes: usize,
}

/// Error types for QmdbStorage
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("QMDB error: {0}")]
    Qmdb(String),

    #[error("Key error")]
    Key(ErrorCode),

    #[error("Value too large: {size} bytes (max: {max})")]
    ValueTooLarge { size: usize, max: usize },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    #[error("Invalid storage configuration: {0}")]
    InvalidConfig(String),
}

impl From<ErrorCode> for StorageError {
    fn from(err: ErrorCode) -> Self {
        StorageError::Key(err)
    }
}

fn map_qmdb_error(err: impl std::fmt::Display) -> StorageError {
    StorageError::Qmdb(err.to_string())
}

fn map_storage_error(err: StorageError) -> ErrorCode {
    match err {
        StorageError::Key(code) => code,
        StorageError::ValueTooLarge { .. } => crate::types::ERR_VALUE_TOO_LARGE,
        StorageError::InvalidState(_) => crate::types::ERR_CONCURRENCY_ERROR,
        StorageError::Qmdb(err) => {
            tracing::error!("QMDB error: {err}");
            crate::types::ERR_ADB_ERROR
        }
        StorageError::Io(err) => {
            tracing::error!("Storage IO error: {err}");
            crate::types::ERR_STORAGE_IO
        }
        StorageError::InvalidConfig(err) => {
            tracing::error!("Invalid storage config: {err}");
            crate::types::ERR_STORAGE_IO
        }
    }
}

fn root_to_commit_hash(root: impl AsRef<[u8]>) -> Result<crate::types::CommitHash, StorageError> {
    let bytes: [u8; 32] = root
        .as_ref()
        .try_into()
        .map_err(|_| StorageError::Qmdb("invalid root hash size".to_string()))?;
    Ok(crate::types::CommitHash::new(bytes))
}

/// QmdbStorage implements evolve's storage traits using commonware's QMDB.
pub struct QmdbStorage<C>
where
    C: RStorage + BufferPooler + Clock + Metrics + Clone + Send + Sync + 'static,
{
    db: Arc<RwLock<QmdbCurrent<C>>>,
    /// Read cache for fast synchronous lookups
    cache: Arc<ShardedDbCache>,
    /// Optional metrics for monitoring storage performance
    metrics: OptionalMetrics,
}

impl<C> Clone for QmdbStorage<C>
where
    C: RStorage + BufferPooler + Clock + Metrics + Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            cache: self.cache.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

impl<C> QmdbStorage<C>
where
    C: RStorage + BufferPooler + Clock + Metrics + Clone + Send + Sync + 'static,
{
    /// Create a new QmdbStorage instance
    pub async fn new(
        context: C,
        config: crate::types::StorageConfig,
    ) -> Result<Self, StorageError> {
        Self::with_metrics(context, config, OptionalMetrics::disabled()).await
    }

    /// Create a new QmdbStorage instance with metrics enabled
    pub async fn with_metrics(
        context: C,
        config: crate::types::StorageConfig,
        metrics: OptionalMetrics,
    ) -> Result<Self, StorageError> {
        // Configure QMDB for state storage
        let page_size = NonZeroU16::new(4096).unwrap();
        let cache_pages = config
            .cache_size
            .checked_div(u64::from(page_size.get()))
            .ok_or_else(|| {
                StorageError::InvalidConfig("cache_size must be at least one page".to_string())
            })?;
        let cache_pages = usize::try_from(cache_pages).map_err(|_| {
            StorageError::InvalidConfig("cache_size exceeds platform limits".to_string())
        })?;
        let capacity = NonZeroUsize::new(cache_pages).ok_or_else(|| {
            StorageError::InvalidConfig("cache_size must be at least one page".to_string())
        })?;
        let write_buffer_size = usize::try_from(config.write_buffer_size).map_err(|_| {
            StorageError::InvalidConfig("write_buffer_size exceeds platform limits".to_string())
        })?;
        let write_buffer_size = NonZeroUsize::new(write_buffer_size).ok_or_else(|| {
            StorageError::InvalidConfig("write_buffer_size must be non-zero".to_string())
        })?;

        let qmdb_config = VariableConfig {
            log_partition: format!("{}_log-journal", config.partition_prefix),
            log_items_per_blob: NonZeroU64::new(1000).unwrap(),
            log_write_buffer: write_buffer_size,
            log_compression: None,
            log_codec_config: ((), (RangeCfg::from(0..=MAX_VALUE_DATA_SIZE), ())),
            mmr_journal_partition: format!("{}_mmr-journal", config.partition_prefix),
            mmr_items_per_blob: NonZeroU64::new(1000).unwrap(),
            mmr_write_buffer: write_buffer_size,
            mmr_metadata_partition: format!("{}_mmr-metadata", config.partition_prefix),
            grafted_mmr_metadata_partition: format!(
                "{}_grafted-mmr-metadata",
                config.partition_prefix
            ),
            translator: EightCap,
            thread_pool: None,
            page_cache: CacheRef::from_pooler(&context, page_size, capacity),
        };

        let db = Db::init(context, qmdb_config)
            .await
            .map_err(map_qmdb_error)?;

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
            cache: Arc::new(ShardedDbCache::with_defaults()),
            metrics,
        })
    }

    /// Returns a reference to the read cache for external management.
    pub fn cache(&self) -> &Arc<ShardedDbCache> {
        &self.cache
    }

    /// Returns a reference to the metrics.
    pub fn metrics(&self) -> &OptionalMetrics {
        &self.metrics
    }

    /// Async get - for use when already in async context.
    pub async fn get_async(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        // Check cache first
        if let Some(cached) = self.cache.get(key) {
            return match cached {
                CachedValue::Present(data) => Ok(Some(data)),
                CachedValue::Absent => Ok(None),
            };
        }
        self.get_async_uncached(key).await
    }

    /// Async batch get that resolves multiple keys while taking the QMDB read lock once.
    pub async fn get_many_async(
        &self,
        keys: &[Vec<u8>],
    ) -> Result<Vec<Option<Vec<u8>>>, ErrorCode> {
        if keys.len() > crate::types::MAX_BATCH_SIZE {
            return Err(crate::types::ERR_BATCH_TOO_LARGE);
        }

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = vec![None; keys.len()];
        let mut misses = Vec::with_capacity(keys.len());

        for (idx, key) in keys.iter().enumerate() {
            if let Some(cached) = self.cache.get(key) {
                self.metrics.record_cache_hit();
                match cached {
                    CachedValue::Present(data) => results[idx] = Some(data),
                    CachedValue::Absent => {}
                }
                continue;
            }

            self.metrics.record_cache_miss();
            misses.push((idx, key.clone(), create_storage_key(key)?));
        }

        if misses.is_empty() {
            return Ok(results);
        }

        let start = Instant::now();
        let miss_values = self.get_many_async_uncached(&misses).await?;
        self.metrics
            .record_read_latency(start.elapsed().as_secs_f64());

        for (miss, value) in misses.into_iter().zip(miss_values) {
            let (idx, key, _) = miss;
            match &value {
                Some(data) => self.cache.insert_present(key, data.clone()),
                None => self.cache.insert_absent(key),
            }
            results[idx] = value;
        }

        Ok(results)
    }

    /// Synchronous wrapper for batched reads.
    pub fn get_many(&self, keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>, ErrorCode> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return match handle.runtime_flavor() {
                RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(self.get_many_async(keys)))
                }
                RuntimeFlavor::CurrentThread => Err(crate::types::ERR_RUNTIME_ERROR),
                _ => tokio::task::block_in_place(|| handle.block_on(self.get_many_async(keys))),
            };
        }

        futures::executor::block_on(self.get_many_async(keys))
    }

    fn prepare_batch(
        operations: Vec<crate::types::Operation>,
    ) -> Result<PreparedBatch, StorageError> {
        if operations.len() > crate::types::MAX_BATCH_SIZE {
            return Err(StorageError::Key(crate::types::ERR_BATCH_TOO_LARGE));
        }

        let ops_count = operations.len();
        let mut sets = 0usize;
        let mut deletes = 0usize;
        let mut keys_to_invalidate = Vec::with_capacity(ops_count);
        let mut updates = Vec::with_capacity(ops_count);

        for op in operations {
            match op {
                crate::types::Operation::Set { key, value } => {
                    if value.len() > MAX_VALUE_DATA_SIZE {
                        return Err(StorageError::ValueTooLarge {
                            size: value.len(),
                            max: MAX_VALUE_DATA_SIZE,
                        });
                    }

                    sets += 1;
                    let storage_key = create_storage_key(&key)?;
                    keys_to_invalidate.push(key);
                    updates.push((storage_key, Some(value)));
                }
                crate::types::Operation::Remove { key } => {
                    deletes += 1;
                    let storage_key = create_storage_key(&key)?;
                    keys_to_invalidate.push(key);
                    updates.push((storage_key, None));
                }
            }
        }

        Ok(PreparedBatch {
            updates,
            keys_to_invalidate,
            ops_count,
            sets,
            deletes,
        })
    }

    /// Commit the current state and generate a commit hash.
    pub async fn commit_state(&self) -> Result<crate::types::CommitHash, StorageError> {
        let start = Instant::now();
        let mut db = self.db.write().await;
        let prune_loc = db.inactivity_floor_loc();
        db.prune(prune_loc).await.map_err(map_qmdb_error)?;
        db.sync().await.map_err(map_qmdb_error)?;
        let hash = root_to_commit_hash(db.root())?;
        self.metrics.record_commit(start.elapsed().as_secs_f64());
        Ok(hash)
    }

    /// Preview the state root produced by applying a batch without mutating the database.
    pub async fn preview_batch_root(
        &self,
        operations: &[crate::types::Operation],
    ) -> Result<crate::types::CommitHash, StorageError> {
        let prepared = Self::prepare_batch(operations.to_vec())?;
        let db = self.db.read().await;

        if prepared.updates.is_empty() {
            return root_to_commit_hash(db.root());
        }

        let mut batch = db.new_batch();
        for (storage_key, maybe_value) in prepared.updates {
            batch.write(storage_key, maybe_value);
        }
        let merkleized = batch.merkleize(None).await.map_err(map_qmdb_error)?;
        root_to_commit_hash(merkleized.root())
    }

    /// Apply a batch of operations.
    pub async fn apply_batch(
        &self,
        operations: Vec<crate::types::Operation>,
    ) -> Result<(), StorageError> {
        let prepared = Self::prepare_batch(operations)?;
        if prepared.updates.is_empty() {
            return Ok(());
        }

        let start = Instant::now();
        let PreparedBatch {
            updates,
            keys_to_invalidate,
            ops_count,
            sets,
            deletes,
        } = prepared;

        {
            let mut db = self.db.write().await;
            let changeset = {
                let mut batch = db.new_batch();
                for (storage_key, maybe_value) in updates {
                    batch.write(storage_key, maybe_value);
                }
                let merkleized = batch.merkleize(None).await.map_err(map_qmdb_error)?;
                merkleized.finalize()
            };
            db.apply_batch(changeset).await.map_err(map_qmdb_error)?;
        }

        for key in keys_to_invalidate {
            self.cache.invalidate(&key);
        }

        self.metrics
            .record_batch(start.elapsed().as_secs_f64(), ops_count, sets, deletes);

        Ok(())
    }

    /// Internal async get implementation - bypasses cache, hits QMDB directly.
    async fn get_async_uncached(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        let storage_key = create_storage_key(key)?;
        let db = self.db.read().await;
        Self::decode_storage_value(db.get(&storage_key).await)
    }

    async fn get_many_async_uncached(
        &self,
        misses: &[(usize, Vec<u8>, StorageKey)],
    ) -> Result<Vec<Option<Vec<u8>>>, ErrorCode> {
        let db = self.db.read().await;
        let mut values = Vec::with_capacity(misses.len());

        for (_, _, storage_key) in misses {
            values.push(Self::decode_storage_value(db.get(storage_key).await)?);
        }

        Ok(values)
    }

    fn decode_storage_value(
        result: Result<Option<Vec<u8>>, impl std::fmt::Display>,
    ) -> Result<Option<Vec<u8>>, ErrorCode> {
        match result {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Ok(None),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("not found") {
                    Ok(None)
                } else {
                    tracing::error!("QMDB read error: {err_str}");
                    Err(crate::types::ERR_ADB_ERROR)
                }
            }
        }
    }
}

// Implement ReadonlyKV for QmdbStorage
impl<C> ReadonlyKV for QmdbStorage<C>
where
    C: RStorage + BufferPooler + Clock + Metrics + Clone + Send + Sync + 'static,
{
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        // Fast path: check cache first (synchronous, no async overhead)
        if let Some(cached) = self.cache.get(key) {
            self.metrics.record_cache_hit();
            return match cached {
                CachedValue::Present(data) => Ok(Some(data)),
                CachedValue::Absent => Ok(None),
            };
        }

        // Cache miss: fetch from QMDB (async)
        // Try tokio runtime first, fall back to futures executor
        self.metrics.record_cache_miss();
        let start = Instant::now();
        let result = if let Ok(handle) = tokio::runtime::Handle::try_current() {
            match handle.runtime_flavor() {
                RuntimeFlavor::MultiThread => {
                    // We're inside a multi-threaded tokio runtime - use block_in_place
                    tokio::task::block_in_place(|| handle.block_on(self.get_async_uncached(key)))
                }
                RuntimeFlavor::CurrentThread => {
                    return Err(crate::types::ERR_RUNTIME_ERROR);
                }
                _ => tokio::task::block_in_place(|| handle.block_on(self.get_async_uncached(key))),
            }
        } else {
            // Not in tokio runtime - use futures executor
            futures::executor::block_on(self.get_async_uncached(key))
        }?;
        self.metrics
            .record_read_latency(start.elapsed().as_secs_f64());

        // Populate cache with the result
        match &result {
            Some(data) => self.cache.insert_present(key.to_vec(), data.clone()),
            None => self.cache.insert_absent(key.to_vec()),
        }

        Ok(result)
    }
}

// Implement the Storage trait (for batch operations and commits)
#[async_trait(?Send)]
impl<C> crate::Storage for QmdbStorage<C>
where
    C: RStorage + BufferPooler + Clock + Metrics + Clone + Send + Sync + 'static,
{
    async fn commit(&self) -> Result<crate::types::CommitHash, ErrorCode> {
        self.commit_state().await.map_err(map_storage_error)
    }

    async fn batch(&self, operations: Vec<crate::types::Operation>) -> Result<(), ErrorCode> {
        self.apply_batch(operations)
            .await
            .map_err(map_storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_runtime::tokio::{Config as TokioConfig, Runner};
    use commonware_runtime::Runner as RunnerTrait;
    use tempfile::TempDir;

    #[test]
    fn test_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            // Create storage
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Test set and get using batch operations
            let key = b"test_key";
            let value = b"test_value";

            // Set value using batch
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: key.to_vec(),
                    value: value.to_vec(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(key).unwrap();
            assert_eq!(retrieved, Some(value.to_vec()));

            // Test remove using batch
            storage
                .apply_batch(vec![crate::types::Operation::Remove { key: key.to_vec() }])
                .await
                .unwrap();

            let retrieved = storage.get(key).unwrap();
            assert_eq!(retrieved, None);
        })
    }

    #[test]
    fn test_batch_size_validation() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            // Create storage
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Create a batch that exceeds MAX_BATCH_SIZE
            let mut operations = Vec::new();
            for i in 0..=crate::types::MAX_BATCH_SIZE {
                operations.push(crate::types::Operation::Set {
                    key: format!("key_{i}").into_bytes(),
                    value: b"value".to_vec(),
                });
            }

            // This should fail with ERR_BATCH_TOO_LARGE
            let result = storage.apply_batch(operations).await;
            assert!(result.is_err());

            match result.unwrap_err() {
                StorageError::Key(code) => {
                    assert_eq!(code, crate::types::ERR_BATCH_TOO_LARGE);
                }
                _ => panic!("Expected StorageError::Key with ERR_BATCH_TOO_LARGE"),
            }
        })
    }

    #[test]
    fn test_invalid_config_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            cache_size: 0,
            write_buffer_size: 0,
            partition_prefix: "evolve-state".to_string(),
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let result = QmdbStorage::new(context, config).await;
            assert!(matches!(result, Err(StorageError::InvalidConfig(_))));
        })
    }

    #[test]
    fn test_key_size_limit() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Key exactly at limit (256 bytes) should work
            let max_key = vec![b'k'; crate::types::MAX_KEY_SIZE];
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: max_key.clone(),
                    value: b"value".to_vec(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(&max_key).unwrap();
            assert_eq!(retrieved, Some(b"value".to_vec()));

            // Key exceeding limit should fail
            let oversized_key = vec![b'k'; crate::types::MAX_KEY_SIZE + 1];
            let result = storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: oversized_key,
                    value: b"value".to_vec(),
                }])
                .await;

            assert!(result.is_err());
            match result.unwrap_err() {
                StorageError::Key(code) => {
                    assert_eq!(code, crate::types::ERR_KEY_TOO_LARGE);
                }
                _ => panic!("Expected ERR_KEY_TOO_LARGE"),
            }
        })
    }

    #[test]
    fn test_trailing_zero_keys_distinct() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            let key_a = b"a".to_vec();
            let key_a_zero = b"a\0".to_vec();

            storage
                .apply_batch(vec![
                    crate::types::Operation::Set {
                        key: key_a.clone(),
                        value: b"value_a".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: key_a_zero.clone(),
                        value: b"value_a_zero".to_vec(),
                    },
                ])
                .await
                .unwrap();

            assert_eq!(storage.get(&key_a).unwrap(), Some(b"value_a".to_vec()));
            assert_eq!(
                storage.get(&key_a_zero).unwrap(),
                Some(b"value_a_zero".to_vec())
            );
        })
    }

    #[test]
    fn test_value_size_limit() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Value exactly at MAX_VALUE_DATA_SIZE limit should work
            let max_value = vec![b'v'; crate::types::MAX_VALUE_DATA_SIZE];
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key".to_vec(),
                    value: max_value.clone(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(b"key").unwrap();
            assert_eq!(retrieved, Some(max_value));

            // Value exceeding MAX_VALUE_DATA_SIZE should fail
            let oversized_value = vec![b'v'; crate::types::MAX_VALUE_DATA_SIZE + 1];
            let result = storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key2".to_vec(),
                    value: oversized_value,
                }])
                .await;

            assert!(result.is_err());
            match result.unwrap_err() {
                StorageError::ValueTooLarge { size, max } => {
                    assert_eq!(size, crate::types::MAX_VALUE_DATA_SIZE + 1);
                    assert_eq!(max, crate::types::MAX_VALUE_DATA_SIZE);
                }
                e => panic!("Expected ValueTooLarge, got {:?}", e),
            }
        })
    }

    #[test]
    fn test_commit_produces_valid_hash() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Apply some operations
            storage
                .apply_batch(vec![
                    crate::types::Operation::Set {
                        key: b"key1".to_vec(),
                        value: b"value1".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: b"key2".to_vec(),
                        value: b"value2".to_vec(),
                    },
                ])
                .await
                .unwrap();

            // Commit and get hash - should be a valid 32-byte hash
            let hash1 = storage.commit_state().await.unwrap();
            assert_eq!(hash1.as_bytes().len(), 32);

            // Commit hash should not be all zeros
            assert!(!hash1.as_bytes().iter().all(|&b| b == 0));

            // Add more data and commit - should produce a different hash
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key3".to_vec(),
                    value: b"value3".to_vec(),
                }])
                .await
                .unwrap();

            let hash2 = storage.commit_state().await.unwrap();
            assert_eq!(hash2.as_bytes().len(), 32);

            // Hashes should be different after state change
            assert_ne!(hash1.as_bytes(), hash2.as_bytes());
        })
    }

    #[test]
    fn test_cache_invalidation_on_write() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Set a value
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key".to_vec(),
                    value: b"value1".to_vec(),
                }])
                .await
                .unwrap();

            // Read to populate cache
            let v1 = storage.get(b"key").unwrap();
            assert_eq!(v1, Some(b"value1".to_vec()));

            // Update the value
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key".to_vec(),
                    value: b"value2".to_vec(),
                }])
                .await
                .unwrap();

            // Read again - should get new value (cache was invalidated)
            let v2 = storage.get(b"key").unwrap();
            assert_eq!(v2, Some(b"value2".to_vec()));
        })
    }

    #[test]
    fn test_remove_invalidates_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Set a value
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key".to_vec(),
                    value: b"value".to_vec(),
                }])
                .await
                .unwrap();

            // Read to populate cache
            assert_eq!(storage.get(b"key").unwrap(), Some(b"value".to_vec()));

            // Remove the key
            storage
                .apply_batch(vec![crate::types::Operation::Remove {
                    key: b"key".to_vec(),
                }])
                .await
                .unwrap();

            // Read again - should get None (cache was invalidated)
            assert_eq!(storage.get(b"key").unwrap(), None);
        })
    }

    #[test]
    fn test_batch_multiple_operations() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Apply a batch with multiple operations
            storage
                .apply_batch(vec![
                    crate::types::Operation::Set {
                        key: b"key1".to_vec(),
                        value: b"value1".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: b"key2".to_vec(),
                        value: b"value2".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: b"key3".to_vec(),
                        value: b"value3".to_vec(),
                    },
                ])
                .await
                .unwrap();

            // Verify all values
            assert_eq!(storage.get(b"key1").unwrap(), Some(b"value1".to_vec()));
            assert_eq!(storage.get(b"key2").unwrap(), Some(b"value2".to_vec()));
            assert_eq!(storage.get(b"key3").unwrap(), Some(b"value3".to_vec()));

            // Now remove one and update another in a batch
            storage
                .apply_batch(vec![
                    crate::types::Operation::Remove {
                        key: b"key2".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: b"key1".to_vec(),
                        value: b"updated".to_vec(),
                    },
                ])
                .await
                .unwrap();

            assert_eq!(storage.get(b"key1").unwrap(), Some(b"updated".to_vec()));
            assert_eq!(storage.get(b"key2").unwrap(), None);
            assert_eq!(storage.get(b"key3").unwrap(), Some(b"value3".to_vec()));
        })
    }

    #[test]
    fn test_empty_batch() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Empty batch should succeed
            storage.apply_batch(vec![]).await.unwrap();
        })
    }

    #[test]
    fn test_get_nonexistent_key() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Get a key that was never set
            let result = storage.get(b"nonexistent").unwrap();
            assert_eq!(result, None);

            // Second read should hit cache (negative cache)
            let result2 = storage.get(b"nonexistent").unwrap();
            assert_eq!(result2, None);
        })
    }

    #[test]
    fn test_empty_value() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"empty".to_vec(),
                    value: vec![],
                }])
                .await
                .unwrap();

            let result = storage.get(b"empty").unwrap();
            assert_eq!(result, Some(Vec::new()));
        })
    }

    #[test]
    fn test_storage_trait_impl() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Test via the Storage trait
            use crate::Storage as StorageTrait;

            storage
                .batch(vec![crate::types::Operation::Set {
                    key: b"trait_key".to_vec(),
                    value: b"trait_value".to_vec(),
                }])
                .await
                .unwrap();

            let hash = storage.commit().await.unwrap();
            assert_eq!(hash.as_bytes().len(), 32);

            // Verify the write worked
            assert_eq!(
                storage.get(b"trait_key").unwrap(),
                Some(b"trait_value".to_vec())
            );
        })
    }

    #[test]
    fn test_readonly_kv_ext() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Test ReadonlyKVExt methods
            use crate::ReadonlyKVExt;

            // exists() on non-existent key
            assert!(!storage.exists(b"nope").unwrap());

            // Set a value
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"exists_key".to_vec(),
                    value: b"exists_value".to_vec(),
                }])
                .await
                .unwrap();

            // exists() on existing key
            assert!(storage.exists(b"exists_key").unwrap());

            // get_or_default() on non-existent key
            let default = storage
                .get_or_default(b"missing", b"default".to_vec())
                .unwrap();
            assert_eq!(default, b"default");

            // get_or_default() on existing key
            let existing = storage
                .get_or_default(b"exists_key", b"default".to_vec())
                .unwrap();
            assert_eq!(existing, b"exists_value");
        })
    }

    /// Test that values with trailing zeros are preserved correctly.
    /// This is critical for storing encoded integers (u128, u64) where
    /// small values have many trailing zero bytes in little-endian.
    #[test]
    fn test_value_with_trailing_zeros() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Simulate AccountId(65539) encoded as u128 little-endian
            // This has 13 trailing zero bytes
            let account_id: u128 = 65539;
            let value = account_id.to_le_bytes().to_vec();
            assert_eq!(value.len(), 16);
            assert_eq!(
                value,
                vec![
                    0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00
                ]
            );

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"account_id".to_vec(),
                    value: value.clone(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(b"account_id").unwrap().unwrap();
            assert_eq!(
                retrieved, value,
                "Value with trailing zeros must be preserved exactly"
            );
            assert_eq!(retrieved.len(), 16, "Length must be preserved");
        })
    }

    /// Test Vec<AccountId> pattern - this is what caused the original bug.
    /// A borsh-encoded Vec has a 4-byte length prefix followed by elements.
    #[test]
    fn test_vec_account_id_pattern() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Simulate borsh-encoded Vec<AccountId> with one element (AccountId 65539)
            // Format: 4-byte length (little-endian) + 16-byte AccountId
            let mut value = Vec::new();
            value.extend_from_slice(&1u32.to_le_bytes()); // length = 1
            value.extend_from_slice(&65539u128.to_le_bytes()); // AccountId(65539)
            assert_eq!(value.len(), 20);

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"vec_account".to_vec(),
                    value: value.clone(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(b"vec_account").unwrap().unwrap();
            assert_eq!(retrieved, value, "Vec<AccountId> pattern must be preserved");
            assert_eq!(retrieved.len(), 20);

            // Verify we can decode it back
            let len = u32::from_le_bytes(retrieved[0..4].try_into().unwrap());
            assert_eq!(len, 1);
            let account_id = u128::from_le_bytes(retrieved[4..20].try_into().unwrap());
            assert_eq!(account_id, 65539);
        })
    }

    /// Test that all-zeros value (except length prefix) works correctly
    #[test]
    fn test_zero_value_bytes() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Value that is all zeros (e.g., AccountId(0))
            let value = vec![0u8; 16];

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"zero_value".to_vec(),
                    value: value.clone(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(b"zero_value").unwrap().unwrap();
            assert_eq!(retrieved, value, "All-zero value must be preserved");
            assert_eq!(retrieved.len(), 16);
        })
    }

    /// Test that values survive commit cycle (the real-world scenario)
    #[test]
    fn test_trailing_zeros_survive_commit() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();

            // Simulate scheduler's begin_block_accounts storage
            let mut value = Vec::new();
            value.extend_from_slice(&1u32.to_le_bytes()); // Vec length = 1
            value.extend_from_slice(&65539u128.to_le_bytes()); // poa AccountId

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"scheduler_key".to_vec(),
                    value: value.clone(),
                }])
                .await
                .unwrap();

            // Commit to durable storage (transitions Clean -> Mutable -> Durable -> Clean)
            let hash = storage.commit_state().await.unwrap();
            assert!(!hash.as_bytes().iter().all(|&b| b == 0));

            // Read after commit - this is where the original bug manifested
            let retrieved = storage.get(b"scheduler_key").unwrap().unwrap();
            assert_eq!(
                retrieved.len(),
                20,
                "Value length must be preserved after commit (was {} bytes)",
                retrieved.len()
            );
            assert_eq!(retrieved, value, "Value must be identical after commit");
        })
    }

    #[test]
    fn test_commit_prunes_inactive_history() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            use commonware_storage::qmdb::store::LogStore;

            let storage = QmdbStorage::new(context, config).await.unwrap();
            const KEYS: usize = 1_100;

            let initial_ops = (0..KEYS)
                .map(|i| crate::types::Operation::Set {
                    key: format!("key-{i}").into_bytes(),
                    value: format!("value-{i}-v1").into_bytes(),
                })
                .collect();
            storage.apply_batch(initial_ops).await.unwrap();
            storage.commit_state().await.unwrap();

            let start_before = {
                let db = storage.db.read().await;
                *db.bounds().await.start
            };

            let second_ops = (0..KEYS)
                .map(|i| {
                    if i % 5 == 0 {
                        crate::types::Operation::Remove {
                            key: format!("key-{i}").into_bytes(),
                        }
                    } else {
                        crate::types::Operation::Set {
                            key: format!("key-{i}").into_bytes(),
                            value: format!("value-{i}-v2").into_bytes(),
                        }
                    }
                })
                .collect();
            storage.apply_batch(second_ops).await.unwrap();
            storage.commit_state().await.unwrap();

            let start_after = {
                let db = storage.db.read().await;
                *db.bounds().await.start
            };

            assert!(
                start_after > start_before,
                "prune boundary did not advance: before={start_before}, after={start_after}"
            );
            assert_eq!(storage.get(b"key-1").unwrap(), Some(b"value-1-v2".to_vec()));
            assert_eq!(storage.get(b"key-0").unwrap(), None);
        })
    }

    #[test]
    fn test_preview_batch_root_matches_eventual_commit_hash() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"base".to_vec(),
                    value: b"value1".to_vec(),
                }])
                .await
                .unwrap();
            let committed_base_hash = storage.commit_state().await.unwrap();

            let preview_operations = vec![
                crate::types::Operation::Remove {
                    key: b"base".to_vec(),
                },
                crate::types::Operation::Set {
                    key: b"next".to_vec(),
                    value: b"value2".to_vec(),
                },
            ];
            let preview_hash = storage
                .preview_batch_root(&preview_operations)
                .await
                .unwrap();

            assert_ne!(preview_hash, committed_base_hash);
            assert_eq!(storage.get(b"base").unwrap(), Some(b"value1".to_vec()));
            assert_eq!(storage.get(b"next").unwrap(), None);

            storage.apply_batch(preview_operations).await.unwrap();
            let committed_hash = storage.commit_state().await.unwrap();

            assert_eq!(preview_hash, committed_hash);
            assert_eq!(storage.get(b"base").unwrap(), None);
            assert_eq!(storage.get(b"next").unwrap(), Some(b"value2".to_vec()));
        })
    }

    #[test]
    fn test_get_many_returns_results_in_order() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();
            storage
                .apply_batch(vec![
                    crate::types::Operation::Set {
                        key: b"key1".to_vec(),
                        value: b"value1".to_vec(),
                    },
                    crate::types::Operation::Set {
                        key: b"key2".to_vec(),
                        value: b"value2".to_vec(),
                    },
                ])
                .await
                .unwrap();

            let values = storage
                .get_many(&[b"missing".to_vec(), b"key2".to_vec(), b"key1".to_vec()])
                .unwrap();

            assert_eq!(
                values,
                vec![None, Some(b"value2".to_vec()), Some(b"value1".to_vec())]
            );
        })
    }

    #[test]
    fn test_get_many_uses_cache_for_present_and_absent_values() {
        let temp_dir = TempDir::new().unwrap();
        let config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);

        runner.start(|context| async move {
            let storage = QmdbStorage::new(context, config).await.unwrap();
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"key".to_vec(),
                    value: b"value".to_vec(),
                }])
                .await
                .unwrap();

            let first = storage
                .get_many(&[b"key".to_vec(), b"missing".to_vec()])
                .unwrap();
            let second = storage
                .get_many(&[b"key".to_vec(), b"missing".to_vec()])
                .unwrap();

            assert_eq!(first, second);
            assert_eq!(first, vec![Some(b"value".to_vec()), None]);
        })
    }
}
