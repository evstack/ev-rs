//! QmdbStorage implementation using commonware's QMDB
//!
//! QMDB (Quick Merkle Database) provides:
//! - Historical proofs for any value ever associated with a key
//! - Efficient Merkle root computation
//! - Pruning support for storage management
//!
//! ## State Machine
//!
//! QMDB uses a state machine pattern:
//! - Clean (Merkleized, Durable) - ready for proofs, has root hash
//! - Mutable (Unmerkleized, NonDurable) - can update/delete keys
//!
//! Transitions:
//! - init() → Clean
//! - into_mutable() → Mutable
//! - commit(metadata) → (Durable, Range) - returns tuple
//! - into_merkleized() → Clean

// Instant is used for performance metrics, not consensus-affecting logic.
#![allow(clippy::disallowed_types)]

use crate::cache::{CachedValue, ShardedDbCache};
use crate::metrics::OptionalMetrics;
use crate::types::{
    create_storage_key, create_storage_value_chunk, extract_value_from_chunk, StorageKey,
    StorageValueChunk, MAX_VALUE_DATA_SIZE,
};
use async_trait::async_trait;
use commonware_cryptography::sha256::Sha256;
use commonware_runtime::{utils::buffer::pool::PoolRef, Clock, Metrics, Storage as RStorage};
use commonware_storage::qmdb::{
    current::{unordered::fixed::Db, FixedConfig},
    store::{Durable, NonDurable},
    Merkleized, Unmerkleized,
};
use commonware_storage::translator::EightCap;
use evolve_core::{ErrorCode, ReadonlyKV};
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::runtime::RuntimeFlavor;
use tokio::sync::RwLock;

/// Type alias for QMDB in Clean state (Merkleized, Durable)
/// N = 64 because SHA256 digest is 32 bytes, and N must be 2 * digest_size
type QmdbClean<C> =
    Db<C, StorageKey, StorageValueChunk, Sha256, EightCap, 64, Merkleized<Sha256>, Durable>;

/// Type alias for QMDB in Mutable state (Unmerkleized, NonDurable)
type QmdbMutable<C> =
    Db<C, StorageKey, StorageValueChunk, Sha256, EightCap, 64, Unmerkleized, NonDurable>;

/// Type alias for QMDB in Durable/Unmerkleized state (after commit, before merkleize)
type QmdbDurable<C> =
    Db<C, StorageKey, StorageValueChunk, Sha256, EightCap, 64, Unmerkleized, Durable>;

/// Internal state enum to track QMDB state machine transitions
enum QmdbState<C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static> {
    /// Clean state - ready for proofs, durable
    Clean(QmdbClean<C>),
    /// Mutable state - can perform updates/deletes
    Mutable(QmdbMutable<C>),
    /// Transitional state for ownership management
    Transitioning,
}

/// Error types for QmdbStorage
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("QMDB error: {0}")]
    Qmdb(String),

    #[error("Key error")]
    Key(ErrorCode),

    #[error("Value too large for single chunk: {size} bytes (max: {max})")]
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

/// Maps QMDB errors to evolve error codes
fn map_qmdb_error(err: impl std::fmt::Display) -> ErrorCode {
    tracing::error!("QMDB error: {err}");
    crate::types::ERR_ADB_ERROR
}

/// Maps concurrency errors to evolve error codes
fn map_concurrency_error(err: impl std::fmt::Display) -> ErrorCode {
    tracing::error!("Concurrency error: {err}");
    crate::types::ERR_CONCURRENCY_ERROR
}

/// QmdbStorage implements evolve's storage traits using commonware's QMDB
///
/// Provides:
/// - Synchronous read via `ReadonlyKV::get()` (uses block_on internally)
/// - Async batch operations via `Storage::batch()`
/// - Async commit with Merkle root via `Storage::commit()`
/// - In-memory read cache (ShardedDbCache) for reduced lock contention
///
/// ## State Machine Management
///
/// The wrapper automatically manages QMDB state transitions:
/// - `batch()` ensures the DB is in Mutable state before applying operations
/// - `commit()` transitions through: Mutable → Durable → Clean (with Merkle root)
pub struct QmdbStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    #[allow(dead_code)]
    context: Arc<C>,
    state: Arc<RwLock<QmdbState<C>>>,
    /// Read cache for fast synchronous lookups
    cache: Arc<ShardedDbCache>,
    /// Optional metrics for monitoring storage performance
    metrics: OptionalMetrics,
}

impl<C> Clone for QmdbStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            state: self.state.clone(),
            cache: self.cache.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

impl<C> QmdbStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
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

        let qmdb_config = FixedConfig {
            log_journal_partition: format!("{}_log-journal", config.partition_prefix),
            log_items_per_blob: NonZeroU64::new(1000).unwrap(),
            log_write_buffer: write_buffer_size,
            mmr_journal_partition: format!("{}_mmr-journal", config.partition_prefix),
            mmr_items_per_blob: NonZeroU64::new(1000).unwrap(),
            mmr_write_buffer: write_buffer_size,
            mmr_metadata_partition: format!("{}_mmr-metadata", config.partition_prefix),
            bitmap_metadata_partition: format!("{}_bitmap-metadata", config.partition_prefix),
            translator: EightCap,
            thread_pool: None,
            buffer_pool: PoolRef::new(page_size, capacity),
        };

        // Initialize QMDB - starts in Clean state (Merkleized, Durable)
        let db: QmdbClean<C> = Db::init(context.clone(), qmdb_config)
            .await
            .map_err(|e| StorageError::Qmdb(e.to_string()))?;

        Ok(Self {
            context: Arc::new(context),
            state: Arc::new(RwLock::new(QmdbState::Clean(db))),
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

    /// Commit the current state and generate a commit hash
    ///
    /// State transition: Mutable → Durable → Clean (Merkleized)
    pub async fn commit_state(&self) -> Result<crate::types::CommitHash, StorageError> {
        let start = Instant::now();
        let mut state_guard = self.state.write().await;

        // Take ownership to perform state transitions
        let current_state = std::mem::replace(&mut *state_guard, QmdbState::Transitioning);

        let clean_db = match current_state {
            QmdbState::Clean(db) => {
                // Already clean, just return current root
                db
            }
            QmdbState::Mutable(db) => {
                // Mutable → commit() → (Durable, Range)
                let (durable_db, _range): (QmdbDurable<C>, _) = db
                    .commit(None)
                    .await
                    .map_err(|e| StorageError::Qmdb(e.to_string()))?;

                // Durable → into_merkleized() → Clean
                let clean: QmdbClean<C> = durable_db
                    .into_merkleized()
                    .await
                    .map_err(|e| StorageError::Qmdb(e.to_string()))?;

                clean
            }
            QmdbState::Transitioning => {
                return Err(StorageError::InvalidState(
                    "Storage in transitioning state".to_string(),
                ));
            }
        };

        // Get the root hash
        let root = clean_db.root();
        let hash = match root.as_ref().try_into() {
            Ok(bytes) => crate::types::CommitHash::new(bytes),
            Err(_) => {
                *state_guard = QmdbState::Clean(clean_db);
                return Err(StorageError::Qmdb("Invalid root hash size".to_string()));
            }
        };

        // Store clean state back
        *state_guard = QmdbState::Clean(clean_db);

        // Record commit latency
        self.metrics.record_commit(start.elapsed().as_secs_f64());

        Ok(hash)
    }

    /// Apply a batch of operations
    pub async fn apply_batch(
        &self,
        operations: Vec<crate::types::Operation>,
    ) -> Result<(), StorageError> {
        // Validate batch size against MAX_BATCH_SIZE
        if operations.len() > crate::types::MAX_BATCH_SIZE {
            return Err(StorageError::Key(crate::types::ERR_BATCH_TOO_LARGE));
        }

        if operations.is_empty() {
            return Ok(());
        }

        let start = Instant::now();
        let ops_count = operations.len();
        let mut sets = 0usize;
        let mut deletes = 0usize;

        // Collect keys for cache invalidation
        let mut keys_to_invalidate = Vec::with_capacity(operations.len());

        // Pre-compute all storage keys and values before acquiring the lock
        let mut prepared_updates: Vec<(StorageKey, Option<StorageValueChunk>)> =
            Vec::with_capacity(operations.len());

        for op in operations {
            match op {
                crate::types::Operation::Set { key, value } => {
                    // Validate value size before processing (must fit with length prefix)
                    if value.len() > MAX_VALUE_DATA_SIZE {
                        return Err(StorageError::ValueTooLarge {
                            size: value.len(),
                            max: MAX_VALUE_DATA_SIZE,
                        });
                    }

                    sets += 1;
                    keys_to_invalidate.push(key.clone());
                    let storage_key = create_storage_key(&key)?;
                    let storage_value = create_storage_value_chunk(&value)?;
                    prepared_updates.push((storage_key, Some(storage_value)));
                }
                crate::types::Operation::Remove { key } => {
                    deletes += 1;
                    keys_to_invalidate.push(key.clone());
                    let storage_key = create_storage_key(&key)?;
                    prepared_updates.push((storage_key, None));
                }
            }
        }

        let mut state_guard = self.state.write().await;

        // Take ownership to perform state transition if needed
        let current_state = std::mem::replace(&mut *state_guard, QmdbState::Transitioning);

        let mut mutable_db: QmdbMutable<C> = match current_state {
            QmdbState::Clean(db) => {
                // Clean → into_mutable() → Mutable (sync method)
                db.into_mutable()
            }
            QmdbState::Mutable(db) => db,
            QmdbState::Transitioning => {
                return Err(StorageError::InvalidState(
                    "Storage in transitioning state".to_string(),
                ));
            }
        };

        // Apply all updates
        for (storage_key, maybe_value) in prepared_updates {
            match maybe_value {
                Some(storage_value) => {
                    if let Err(e) = mutable_db.update(storage_key, storage_value).await {
                        *state_guard = QmdbState::Mutable(mutable_db);
                        return Err(StorageError::Qmdb(e.to_string()));
                    }
                }
                None => {
                    // Delete the key
                    if let Err(e) = mutable_db.delete(storage_key).await {
                        *state_guard = QmdbState::Mutable(mutable_db);
                        return Err(StorageError::Qmdb(e.to_string()));
                    }
                }
            }
        }

        // Store mutable state back
        *state_guard = QmdbState::Mutable(mutable_db);

        // Invalidate cache entries for modified keys
        for key in keys_to_invalidate {
            self.cache.invalidate(&key);
        }

        // Record batch metrics
        self.metrics
            .record_batch(start.elapsed().as_secs_f64(), ops_count, sets, deletes);

        Ok(())
    }

    /// Internal async get implementation - bypasses cache, hits QMDB directly.
    async fn get_async_uncached(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        let storage_key = create_storage_key(key)?;
        let state = self.state.clone();

        let result: Result<Option<StorageValueChunk>, _> = {
            let state_guard = state.read().await;
            let state_name = match &*state_guard {
                QmdbState::Clean(_) => "Clean",
                QmdbState::Mutable(_) => "Mutable",
                QmdbState::Transitioning => "Transitioning",
            };
            tracing::debug!(
                "get_async_uncached: state={}, key_len={}",
                state_name,
                key.len()
            );

            match &*state_guard {
                QmdbState::Clean(db) => db.get(&storage_key).await,
                QmdbState::Mutable(db) => db.get(&storage_key).await,
                QmdbState::Transitioning => {
                    return Err(crate::types::ERR_CONCURRENCY_ERROR);
                }
            }
        };

        match result {
            Ok(Some(value_chunk)) => {
                // Extract value using length prefix
                match extract_value_from_chunk(&value_chunk) {
                    Some(data) if data.is_empty() => Ok(None), // Empty value treated as absent
                    Some(data) => Ok(Some(data)),
                    None => {
                        // Invalid length prefix - treat as corrupted/absent
                        tracing::warn!("Invalid value chunk format, treating as absent");
                        Ok(None)
                    }
                }
            }
            Ok(None) => Ok(None),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("not found") {
                    Ok(None)
                } else {
                    Err(map_qmdb_error(err_str))
                }
            }
        }
    }
}

// Implement ReadonlyKV for QmdbStorage
impl<C> ReadonlyKV for QmdbStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
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
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    async fn commit(&self) -> Result<crate::types::CommitHash, ErrorCode> {
        self.commit_state()
            .await
            .map_err(|_| crate::types::ERR_STORAGE_IO)
    }

    async fn batch(&self, operations: Vec<crate::types::Operation>) -> Result<(), ErrorCode> {
        self.apply_batch(operations)
            .await
            .map_err(map_concurrency_error)
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

            // Empty values are treated as removed (since we use all-zeros to signal removal)
            // This is a known limitation
            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"empty".to_vec(),
                    value: vec![],
                }])
                .await
                .unwrap();

            // Empty value should be treated as None (due to removal semantics)
            let result = storage.get(b"empty").unwrap();
            assert_eq!(result, None);
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

    /// Test values with zeros in the middle
    #[test]
    fn test_value_with_embedded_zeros() {
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

            // Value with zeros in middle and at end
            let value = vec![0x01, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00];

            storage
                .apply_batch(vec![crate::types::Operation::Set {
                    key: b"embedded_zeros".to_vec(),
                    value: value.clone(),
                }])
                .await
                .unwrap();

            let retrieved = storage.get(b"embedded_zeros").unwrap().unwrap();
            assert_eq!(
                retrieved, value,
                "Value with embedded zeros must be preserved exactly"
            );
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
}
