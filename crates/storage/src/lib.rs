pub mod cache;
pub mod metrics;
pub mod mock;
pub mod qmdb_impl;
pub mod types;
pub mod warming;

pub use cache::{CachedStorage, CachedValue, DbCache, ShardedDbCache};
pub use metrics::{OptionalMetrics, StorageMetrics};
pub use mock::MockStorage;
pub use qmdb_impl::QmdbStorage;
pub use types::*;
pub use warming::{warm_cache, warm_cache_async, KeyPredictor, NoOpKeyPredictor};

use async_trait::async_trait;
use evolve_core::{ErrorCode, ReadonlyKV};

/// Combined storage trait with async commit functionality
#[async_trait(?Send)]
pub trait Storage: ReadonlyKVExt + Send + Sync {
    /// Commit current changes and return the commit hash
    async fn commit(&self) -> Result<CommitHash, ErrorCode>;

    /// Apply a batch of operations atomically
    async fn batch(&self, operations: Vec<Operation>) -> Result<(), ErrorCode>;
}

/// Extension trait for ReadonlyKV with additional utility methods
pub trait ReadonlyKVExt: ReadonlyKV {
    /// Check if a key exists
    fn exists(&self, key: &[u8]) -> Result<bool, ErrorCode> {
        Ok(self.get(key)?.is_some())
    }

    /// Get a value or return a default if not found
    fn get_or_default(&self, key: &[u8], default: Vec<u8>) -> Result<Vec<u8>, ErrorCode> {
        Ok(self.get(key)?.unwrap_or(default))
    }
}

impl<T: ReadonlyKV + ?Sized> ReadonlyKVExt for T {}
