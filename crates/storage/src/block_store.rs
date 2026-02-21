//! Archive-backed block storage that is independent of the QMDB state root.
//!
//! ## Design Rationale
//!
//! QMDB is used for application state (account data, balances, etc.) and produces
//! a Merkle root (`CommitHash`) on every `commit()` call. Any KV pair written via
//! QMDB's `batch()` + `commit()` affects this Merkle root.
//!
//! Block data (headers, transaction lists, receipts) must NOT affect the app hash.
//! This is achieved by using `commonware_storage::archive::prunable::Archive` as a
//! completely separate storage backend.
//!
//! The archive:
//! - Uses two partitions (key journal + value blob) that are independent from QMDB
//! - Does not participate in any Merkle computation
//! - Supports lookups by block number (`u64`) or block hash (`[u8; 32]`)
//! - Supports pruning old blocks by minimum block number
//!
//! ## State Machine
//!
//! The archive is a single mutable struct (not a state machine like QMDB).
//! Writes require `&mut self`; reads require `&self`.
//!
//! ```text
//! BlockStorage::new() → archive initialized
//! put(block_num, block_hash, block_bytes) → appended to journal
//! get_by_number(block_num) → reads from journal
//! get_by_hash(block_hash) → reads from journal via in-memory key index
//! prune(min_block) → removes sections older than min_block
//! sync() → fsync pending writes
//! ```

use crate::types::{BlockHash, BlockStorageConfig};
use commonware_codec::RangeCfg;
use commonware_runtime::{buffer::PoolRef, Clock, Metrics, Storage as RStorage};
use commonware_storage::{
    archive::{
        prunable::{Archive, Config as ArchiveConfig},
        Archive as ArchiveTrait, Identifier,
    },
    translator::EightCap,
};
use std::num::NonZeroUsize;
use thiserror::Error;

/// Error types for block storage operations.
#[derive(Debug, Error)]
pub enum BlockStorageError {
    #[error("archive error: {0}")]
    Archive(#[from] commonware_storage::archive::Error),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

}

/// Archive-backed block storage.
///
/// Stores block data indexed by both block number and block hash. Independent
/// of QMDB — writes here have no effect on the app hash / commit hash.
///
/// Block data is stored as raw bytes (`bytes::Bytes`). The caller is responsible
/// for serializing and deserializing block contents.
///
/// # Thread Safety
///
/// `BlockStorage` requires `&mut self` for writes and `&self` for reads. It
/// should be wrapped in a lock (`tokio::sync::Mutex`) when shared across tasks.
/// Page size for the key journal buffer pool (bytes).
const KEY_JOURNAL_PAGE_SIZE: u16 = 4096;

/// Number of cached pages in the key journal buffer pool.
/// Total cache: KEY_JOURNAL_PAGE_SIZE * KEY_JOURNAL_CACHE_PAGES = 256KB by default.
const KEY_JOURNAL_CACHE_PAGES: usize = 64;

pub struct BlockStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    archive: Archive<EightCap, C, BlockHash, bytes::Bytes>,
}

impl<C> BlockStorage<C>
where
    C: RStorage + Clock + Metrics + Clone + Send + Sync + 'static,
{
    /// Initialize block storage using the provided runtime context and config.
    ///
    /// If block data was previously stored, the in-memory index is rebuilt from
    /// the key journal on startup (no value reads are performed during init).
    pub async fn new(context: C, config: BlockStorageConfig) -> Result<Self, BlockStorageError> {
        let blocks_per_section = std::num::NonZeroU64::new(config.blocks_per_section)
            .ok_or_else(|| {
                BlockStorageError::InvalidConfig("blocks_per_section must be non-zero".to_string())
            })?;

        let key_write_buffer = NonZeroUsize::new(config.key_write_buffer).ok_or_else(|| {
            BlockStorageError::InvalidConfig("key_write_buffer must be non-zero".to_string())
        })?;

        let value_write_buffer = NonZeroUsize::new(config.value_write_buffer).ok_or_else(|| {
            BlockStorageError::InvalidConfig("value_write_buffer must be non-zero".to_string())
        })?;

        let replay_buffer = NonZeroUsize::new(config.replay_buffer).ok_or_else(|| {
            BlockStorageError::InvalidConfig("replay_buffer must be non-zero".to_string())
        })?;

        // Buffer pool for the key journal.
        let page_size = std::num::NonZeroU16::new(KEY_JOURNAL_PAGE_SIZE).unwrap();
        let cache_pages = std::num::NonZeroUsize::new(KEY_JOURNAL_CACHE_PAGES).unwrap();
        let key_buffer_pool = PoolRef::new(page_size, cache_pages);

        let cfg = ArchiveConfig {
            translator: EightCap,
            key_partition: format!("{}-block-index", config.partition_prefix),
            key_buffer_pool,
            value_partition: format!("{}-block-data", config.partition_prefix),
            // No compression by default. Blocks are often already compressed (gzip/zstd
            // at the application layer), so double-compression wastes CPU.
            compression: None,
            // `bytes::Bytes` uses `RangeCfg<usize>` as its codec config.
            // An unbounded range accepts blocks of any size.
            codec_config: RangeCfg::from(..),
            items_per_section: blocks_per_section,
            key_write_buffer,
            value_write_buffer,
            replay_buffer,
        };

        let archive = Archive::init(context, cfg).await?;

        Ok(Self { archive })
    }

    /// Store a block by its block number and hash.
    ///
    /// Both `block_number` and `block_hash` are assumed to be globally unique.
    /// If `block_number` already exists, this is a no-op (idempotent).
    ///
    /// Returns an error if the block number is older than the current prune horizon.
    #[allow(clippy::disallowed_types)] // Instant is for metrics only, not consensus.
    pub async fn put(
        &mut self,
        block_number: u64,
        block_hash: BlockHash,
        block_bytes: bytes::Bytes,
    ) -> Result<(), BlockStorageError> {
        let start = std::time::Instant::now();
        self.archive
            .put(block_number, block_hash, block_bytes)
            .await?;
        tracing::debug!(
            block_number,
            elapsed_us = start.elapsed().as_micros(),
            "block stored"
        );
        Ok(())
    }

    /// Retrieve block bytes by block number.
    ///
    /// Returns `None` if the block number is not present (never stored, or pruned).
    pub async fn get_by_number(
        &self,
        block_number: u64,
    ) -> Result<Option<bytes::Bytes>, BlockStorageError> {
        Ok(self.archive.get(Identifier::Index(block_number)).await?)
    }

    /// Retrieve block bytes by block hash.
    ///
    /// Returns `None` if the hash is not in the index (never stored, or pruned).
    pub async fn get_by_hash(
        &self,
        block_hash: &BlockHash,
    ) -> Result<Option<bytes::Bytes>, BlockStorageError> {
        Ok(self.archive.get(Identifier::Key(block_hash)).await?)
    }

    /// Check whether a block number exists in the archive.
    pub async fn has_block_number(&self, block_number: u64) -> Result<bool, BlockStorageError> {
        Ok(self.archive.has(Identifier::Index(block_number)).await?)
    }

    /// Check whether a block hash exists in the archive.
    pub async fn has_block_hash(&self, block_hash: &BlockHash) -> Result<bool, BlockStorageError> {
        Ok(self.archive.has(Identifier::Key(block_hash)).await?)
    }

    /// Return the lowest stored block number, or `None` if the archive is empty.
    pub fn first_block_number(&self) -> Option<u64> {
        self.archive.first_index()
    }

    /// Return the highest stored block number, or `None` if the archive is empty.
    pub fn last_block_number(&self) -> Option<u64> {
        self.archive.last_index()
    }

    /// Prune all blocks with `block_number < min_block`.
    ///
    /// Pruning is done at section granularity (`blocks_per_section`). The actual
    /// prune horizon may be lower than `min_block` when `min_block` is not aligned
    /// to a section boundary.
    ///
    /// Calling prune with a `min_block` lower than the current prune horizon is a
    /// no-op (safe to call repeatedly).
    pub async fn prune(&mut self, min_block: u64) -> Result<(), BlockStorageError> {
        self.archive.prune(min_block).await?;
        tracing::debug!(min_block, "block storage pruned");
        Ok(())
    }

    /// Sync all pending writes to durable storage.
    ///
    /// Must be called periodically to ensure crash safety. Each call to `put`
    /// buffers data in memory; `sync` flushes it to the underlying journal blobs.
    pub async fn sync(&mut self) -> Result<(), BlockStorageError> {
        self.archive.sync().await?;
        Ok(())
    }

    /// Store a block and immediately sync.
    ///
    /// Equivalent to `put` followed by `sync`. Use when write durability is
    /// required for each block (e.g., during block finalization).
    #[allow(clippy::disallowed_types)] // Instant is for metrics only, not consensus.
    pub async fn put_sync(
        &mut self,
        block_number: u64,
        block_hash: BlockHash,
        block_bytes: bytes::Bytes,
    ) -> Result<(), BlockStorageError> {
        let start = std::time::Instant::now();
        self.archive
            .put_sync(block_number, block_hash, block_bytes)
            .await?;
        tracing::debug!(
            block_number,
            elapsed_us = start.elapsed().as_micros(),
            "block stored (durable)"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_runtime::tokio::{Config as TokioConfig, Runner};
    use commonware_runtime::Runner as RunnerTrait;
    use commonware_utils::sequence::FixedBytes;
    use tempfile::TempDir;

    fn make_block_hash(n: u8) -> BlockHash {
        FixedBytes::new([n; 32])
    }

    fn make_block_bytes(content: &[u8]) -> bytes::Bytes {
        bytes::Bytes::copy_from_slice(content)
    }

    #[test]
    fn test_block_storage_basic() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            // Store a block
            let block_number = 1u64;
            let block_hash = make_block_hash(1);
            let block_bytes = make_block_bytes(b"block data for block 1");

            store
                .put(block_number, block_hash.clone(), block_bytes.clone())
                .await
                .unwrap();

            // Retrieve by block number
            let retrieved = store.get_by_number(block_number).await.unwrap();
            assert_eq!(retrieved, Some(block_bytes.clone()));

            // Retrieve by block hash
            let retrieved = store.get_by_hash(&block_hash).await.unwrap();
            assert_eq!(retrieved, Some(block_bytes));

            // Non-existent block
            let missing = store.get_by_number(999).await.unwrap();
            assert_eq!(missing, None);
        });
    }

    #[test]
    fn test_block_storage_has() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            let block_hash = make_block_hash(42);

            // Before storing
            assert!(!store.has_block_number(5).await.unwrap());
            assert!(!store.has_block_hash(&block_hash).await.unwrap());

            // After storing
            store
                .put(5, block_hash.clone(), make_block_bytes(b"block 5"))
                .await
                .unwrap();

            assert!(store.has_block_number(5).await.unwrap());
            assert!(store.has_block_hash(&block_hash).await.unwrap());
        });
    }

    #[test]
    fn test_block_storage_first_last_index() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            assert_eq!(store.first_block_number(), None);
            assert_eq!(store.last_block_number(), None);

            store
                .put(10, make_block_hash(10), make_block_bytes(b"block 10"))
                .await
                .unwrap();
            store
                .put(20, make_block_hash(20), make_block_bytes(b"block 20"))
                .await
                .unwrap();
            store
                .put(30, make_block_hash(30), make_block_bytes(b"block 30"))
                .await
                .unwrap();

            assert_eq!(store.first_block_number(), Some(10));
            assert_eq!(store.last_block_number(), Some(30));
        });
    }

    #[test]
    fn test_block_storage_idempotent_put() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            let block_number = 1u64;
            let block_hash = make_block_hash(1);
            let block_bytes_v1 = make_block_bytes(b"first write");
            let block_bytes_v2 = make_block_bytes(b"second write - should be ignored");

            // First put
            store
                .put(block_number, block_hash.clone(), block_bytes_v1.clone())
                .await
                .unwrap();

            // Second put with same index - should be idempotent (no-op)
            store
                .put(block_number, block_hash.clone(), block_bytes_v2)
                .await
                .unwrap();

            // Value should be from first put
            let retrieved = store.get_by_number(block_number).await.unwrap().unwrap();
            assert_eq!(retrieved, block_bytes_v1);
        });
    }

    #[test]
    fn test_block_storage_multiple_blocks() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            // Store 10 blocks
            for i in 0..10u64 {
                let hash = make_block_hash(i as u8);
                let data = format!("block data {i}");
                store
                    .put(i, hash, make_block_bytes(data.as_bytes()))
                    .await
                    .unwrap();
            }

            // Verify all 10 blocks are retrievable
            for i in 0..10u64 {
                let result = store.get_by_number(i).await.unwrap();
                let expected = format!("block data {i}");
                assert_eq!(result, Some(make_block_bytes(expected.as_bytes())));
            }
        });
    }

    #[test]
    fn test_block_storage_large_block() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            // Store a block larger than the QMDB 4KB value chunk limit (>4092 bytes)
            // Block storage has no such constraint.
            let large_block: Vec<u8> = (0..=255u8).cycle().take(1_000_000).collect();
            let block_bytes = bytes::Bytes::from(large_block.clone());

            store
                .put(1, make_block_hash(1), block_bytes.clone())
                .await
                .unwrap();

            let retrieved = store.get_by_number(1).await.unwrap().unwrap();
            assert_eq!(retrieved.len(), 1_000_000);
            assert_eq!(retrieved.as_ref(), large_block.as_slice());
        });
    }

    #[test]
    fn test_block_storage_sync_and_retrieve() {
        let temp_dir = TempDir::new().unwrap();

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            let config = BlockStorageConfig::default();
            let mut store = BlockStorage::new(context, config).await.unwrap();

            let block_bytes = make_block_bytes(b"durable block");

            store
                .put_sync(1, make_block_hash(1), block_bytes.clone())
                .await
                .unwrap();

            let retrieved = store.get_by_number(1).await.unwrap();
            assert_eq!(retrieved, Some(block_bytes));
        });
    }

    /// Verifies the core requirement: block storage does NOT affect the QMDB commit hash.
    ///
    /// We create a QmdbStorage, take a commit hash, then write many blocks to a separate
    /// BlockStorage, then commit QMDB again and verify the hash is unchanged.
    #[test]
    fn test_block_storage_does_not_affect_commit_hash() {
        use crate::QmdbStorage;

        let temp_dir = TempDir::new().unwrap();
        let state_config = crate::types::StorageConfig {
            path: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let runtime_config = TokioConfig::default()
            .with_storage_directory(temp_dir.path())
            .with_worker_threads(2);

        let runner = Runner::new(runtime_config);
        runner.start(|context| async move {
            // Set up QmdbStorage (state)
            let qmdb = QmdbStorage::new(context.clone(), state_config)
                .await
                .unwrap();

            // Write some state and commit to get a baseline hash
            qmdb.apply_batch(vec![crate::types::Operation::Set {
                key: b"account_1".to_vec(),
                value: b"balance_100".to_vec(),
            }])
            .await
            .unwrap();
            let hash_before = qmdb.commit_state().await.unwrap();

            // Write blocks to the SEPARATE block storage
            let block_config = BlockStorageConfig {
                partition_prefix: "test-blocks".to_string(),
                ..Default::default()
            };
            let mut block_store = BlockStorage::new(context.clone(), block_config)
                .await
                .unwrap();

            for i in 0..100u64 {
                let hash = make_block_hash(i as u8);
                let data = format!("block {i} data with lots of content");
                block_store
                    .put(i, hash, make_block_bytes(data.as_bytes()))
                    .await
                    .unwrap();
            }
            block_store.sync().await.unwrap();

            // Commit QMDB again (no new state writes) — hash must be identical
            let hash_after = qmdb.commit_state().await.unwrap();

            assert_eq!(
                hash_before.as_bytes(),
                hash_after.as_bytes(),
                "block storage writes must not change the QMDB commit hash"
            );

            // Stronger proof: write new state AFTER block storage writes.
            // The hash should change only due to the state write, proving block
            // storage didn't corrupt the QMDB namespace.
            qmdb.apply_batch(vec![crate::types::Operation::Set {
                key: b"account_2".to_vec(),
                value: b"balance_200".to_vec(),
            }])
            .await
            .unwrap();
            let hash_with_new_state = qmdb.commit_state().await.unwrap();

            assert_ne!(
                hash_after.as_bytes(),
                hash_with_new_state.as_bytes(),
                "new state write must change the QMDB commit hash (proves QMDB is still functional after block storage writes)"
            );
        });
    }
}
