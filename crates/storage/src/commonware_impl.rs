//! CommonwareStorage implementation using commonware's ADB

use crate::types::{
    create_storage_key, create_storage_value_chunk, StorageKey, StorageValueChunk,
    STORAGE_VALUE_SIZE,
};
use async_trait::async_trait;
use commonware_cryptography::sha256::Sha256;

use commonware_runtime::{buffer::PoolRef, Clock, Metrics, Storage};
use commonware_storage::{
    adb::current::{Config as AdbConfig, Current},
    mmr::hasher::Standard,
    translator::EightCap,
};

/// Type alias for the ADB instance used in CommonwareStorage
type AdbInstance<C> = Current<C, StorageKey, StorageValueChunk, Sha256, EightCap, 256>;
use evolve_core::{ErrorCode, ReadonlyKV};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Error types for CommonwareStorage
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("ADB error: {0}")]
    Adb(String),

    #[error("Key error")]
    Key(ErrorCode),

    #[error("Value too large for single chunk: {size} bytes (max: {max})")]
    ValueTooLarge { size: usize, max: usize },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<ErrorCode> for StorageError {
    fn from(err: ErrorCode) -> Self {
        StorageError::Key(err)
    }
}

/// Maps commonware errors to evolve ADB error codes
fn map_adb_error(err: impl std::fmt::Display) -> ErrorCode {
    log::error!("ADB error: {err}");
    crate::types::ERR_ADB_ERROR
}

/// Maps concurrency errors to evolve error codes
fn map_concurrency_error(err: impl std::fmt::Display) -> ErrorCode {
    log::error!("Concurrency error: {err}");
    crate::types::ERR_CONCURRENCY_ERROR
}

/// CommonwareStorage implements evolve's storage traits using commonware's ADB
pub struct CommonwareStorage<C>
where
    C: Storage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    context: Arc<C>,
    adb: Arc<RwLock<AdbInstance<C>>>,
}

impl<C> Clone for CommonwareStorage<C>
where
    C: Storage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            adb: self.adb.clone(),
        }
    }
}

impl<C> CommonwareStorage<C>
where
    C: Storage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    /// Create a new CommonwareStorage instance
    pub async fn new(
        context: C,
        config: crate::types::StorageConfig,
    ) -> Result<Self, StorageError> {
        // Configure ADB for state storage
        let adb_config = AdbConfig {
            log_journal_partition: format!("{}/log-journal", config.partition_prefix),
            log_items_per_blob: 1000,
            log_write_buffer: config.write_buffer_size as usize,
            mmr_journal_partition: format!("{}/mmr-journal", config.partition_prefix),
            mmr_items_per_blob: 1000,
            mmr_write_buffer: config.write_buffer_size as usize,
            mmr_metadata_partition: format!("{}/mmr-metadata", config.partition_prefix),
            bitmap_metadata_partition: format!("{}/bitmap-metadata", config.partition_prefix),
            translator: EightCap,
            thread_pool: None,
            buffer_pool: PoolRef::new(1024, 10), // 1KB pages, 10 page cache
        };

        // Initialize ADBc
        let adb = Current::init(context.clone(), adb_config)
            .await
            .map_err(|e| StorageError::Adb(e.to_string()))?;

        Ok(Self {
            context: Arc::new(context),
            adb: Arc::new(RwLock::new(adb)),
        })
    }

    /// Commit the current state and generate a commit hash
    pub async fn commit_state(&self) -> Result<crate::types::CommitHash, StorageError> {
        let mut adb_guard = self.adb.write().await;

        adb_guard
            .commit()
            .await
            .map_err(|e| StorageError::Adb(e.to_string()))?;

        let mut hasher = Standard::new();
        let root = adb_guard
            .root(&mut hasher)
            .await
            .map_err(|e| StorageError::Adb(e.to_string()))?;

        Ok(crate::types::CommitHash::new(
            root.as_ref()
                .try_into()
                .map_err(|_| StorageError::Adb("Invalid root hash size".to_string()))?,
        ))
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

        // Pre-compute all storage keys and values before acquiring the lock
        let mut prepared_updates = Vec::with_capacity(operations.len());

        for op in operations {
            match op {
                crate::types::Operation::Set { key, value } => {
                    // Validate value size before processing
                    if value.len() > STORAGE_VALUE_SIZE {
                        return Err(StorageError::ValueTooLarge {
                            size: value.len(),
                            max: STORAGE_VALUE_SIZE,
                        });
                    }

                    let storage_key = create_storage_key(&key)?;
                    let storage_value = create_storage_value_chunk(&value)?;
                    prepared_updates.push((storage_key, storage_value));
                }
                crate::types::Operation::Remove { key } => {
                    let storage_key = create_storage_key(&key)?;
                    // Set empty value to indicate removal
                    let empty_value = create_storage_value_chunk(&[])?;
                    prepared_updates.push((storage_key, empty_value));
                }
            }
        }

        // Now acquire the lock and apply all updates with minimal lock time
        let mut adb_guard = self.adb.write().await;

        for (storage_key, storage_value) in prepared_updates {
            adb_guard
                .update(storage_key, storage_value)
                .await
                .map_err(|e| StorageError::Adb(e.to_string()))?;
        }

        Ok(())
    }

    /// Internal async get implementation
    async fn get_async(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        let storage_key = create_storage_key(key)?;
        let adb = self.adb.clone();

        let result = {
            let adb_guard = adb.read().await;
            adb_guard.get(&storage_key).await
        };

        match result {
            Ok(Some(value_chunk)) => {
                let data = value_chunk.as_ref();
                if data.iter().all(|&b| b == 0) {
                    Ok(None)
                } else {
                    let len = data
                        .iter()
                        .rposition(|&b| b != 0)
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    Ok(Some(data[..len].to_vec()))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => {
                if e.to_string().contains("not found") {
                    Ok(None)
                } else {
                    Err(map_adb_error(e))
                }
            }
        }
    }
}

// Implement ReadonlyKV for CommonwareStorage
impl<C> ReadonlyKV for CommonwareStorage<C>
where
    C: Storage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        // Use futures::executor to block on the async operation
        // This works both inside and outside of async contexts
        futures::executor::block_on(self.get_async(key))
    }
}

// Implement the Storage trait (for batch operations and commits)
#[async_trait(?Send)]
impl<C> crate::Storage for CommonwareStorage<C>
where
    C: Storage + Clock + Metrics + Clone + Send + Sync + 'static,
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
            let storage = CommonwareStorage::new(context, config).await.unwrap();

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
            let storage = CommonwareStorage::new(context, config).await.unwrap();

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
}
