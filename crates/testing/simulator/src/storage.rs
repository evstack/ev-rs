//! Simulated storage with fault injection capabilities.
//!
//! This module provides a storage implementation that can inject faults
//! (read failures, write failures) for testing error handling paths.

use crate::seed::SeededRng;
use evolve_core::{define_error, ErrorCode, ReadonlyKV};
use evolve_server_core::{StateChange, WritableKV};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

define_error!(
    ERR_SIMULATED_STORAGE_FAILURE,
    0x01,
    "simulated storage failure"
);

/// Configuration for storage fault injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Probability of read operations failing (0.0 to 1.0).
    pub read_fault_prob: f64,
    /// Probability of write operations failing (0.0 to 1.0).
    pub write_fault_prob: f64,
    /// Whether to log all storage operations.
    pub log_operations: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            read_fault_prob: 0.0,
            write_fault_prob: 0.0,
            log_operations: false,
        }
    }
}

impl StorageConfig {
    /// Creates a configuration with no faults.
    pub fn no_faults() -> Self {
        Self::default()
    }

    /// Creates a configuration with specified fault probabilities.
    pub fn with_faults(read_fault_prob: f64, write_fault_prob: f64) -> Self {
        Self {
            read_fault_prob: read_fault_prob.clamp(0.0, 1.0),
            write_fault_prob: write_fault_prob.clamp(0.0, 1.0),
            log_operations: false,
        }
    }

    /// Enables operation logging.
    pub fn with_logging(mut self) -> Self {
        self.log_operations = true;
        self
    }
}

/// A storage operation for logging/replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageOp {
    /// A read operation.
    Get {
        key: Vec<u8>,
        result: StorageOpResult,
    },
    /// A write operation.
    Set {
        key: Vec<u8>,
        value: Vec<u8>,
        result: StorageOpResult,
    },
    /// A remove operation.
    Remove { key: Vec<u8>, result: StorageOpResult },
}

/// Result of a storage operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageOpResult {
    /// Operation succeeded.
    Success,
    /// Operation found no value (for reads).
    NotFound,
    /// Operation failed due to injected fault.
    Fault,
}

/// Simulated storage with fault injection.
///
/// Wraps a HashMap-based storage and can inject random failures
/// based on configured probabilities.
pub struct SimulatedStorage {
    /// The actual key-value store.
    inner: HashMap<Vec<u8>, Vec<u8>>,
    /// Random number generator for fault injection.
    rng: SeededRng,
    /// Configuration for fault injection.
    config: StorageConfig,
    /// Log of all operations (if enabled).
    operations_log: Vec<StorageOp>,
    /// Statistics about storage operations.
    stats: StorageStats,
}

/// Statistics about storage operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageStats {
    /// Total number of read operations.
    pub reads: u64,
    /// Number of reads that hit a value.
    pub read_hits: u64,
    /// Number of reads that missed.
    pub read_misses: u64,
    /// Number of reads that failed due to fault injection.
    pub read_faults: u64,
    /// Total number of write operations.
    pub writes: u64,
    /// Number of writes that failed due to fault injection.
    pub write_faults: u64,
    /// Total number of remove operations.
    pub removes: u64,
    /// Number of removes that failed due to fault injection.
    pub remove_faults: u64,
    /// Total bytes read.
    pub bytes_read: u64,
    /// Total bytes written.
    pub bytes_written: u64,
}

impl SimulatedStorage {
    /// Creates a new empty simulated storage.
    pub fn new(seed: u64, config: StorageConfig) -> Self {
        Self {
            inner: HashMap::new(),
            rng: SeededRng::new(seed),
            config,
            operations_log: Vec::new(),
            stats: StorageStats::default(),
        }
    }

    /// Creates a new simulated storage with initial data.
    pub fn with_data(seed: u64, config: StorageConfig, data: HashMap<Vec<u8>, Vec<u8>>) -> Self {
        Self {
            inner: data,
            rng: SeededRng::new(seed),
            config,
            operations_log: Vec::new(),
            stats: StorageStats::default(),
        }
    }

    /// Returns whether a read should fail based on configured probability.
    fn should_fail_read(&mut self) -> bool {
        self.config.read_fault_prob > 0.0 && self.rng.gen_bool(self.config.read_fault_prob)
    }

    /// Returns whether a write should fail based on configured probability.
    fn should_fail_write(&mut self) -> bool {
        self.config.write_fault_prob > 0.0 && self.rng.gen_bool(self.config.write_fault_prob)
    }

    /// Logs an operation if logging is enabled.
    fn log_op(&mut self, op: StorageOp) {
        if self.config.log_operations {
            self.operations_log.push(op);
        }
    }

    /// Returns the operations log.
    pub fn operations_log(&self) -> &[StorageOp] {
        &self.operations_log
    }

    /// Clears the operations log.
    pub fn clear_log(&mut self) {
        self.operations_log.clear();
    }

    /// Returns storage statistics.
    pub fn stats(&self) -> &StorageStats {
        &self.stats
    }

    /// Resets storage statistics.
    pub fn reset_stats(&mut self) {
        self.stats = StorageStats::default();
    }

    /// Returns the number of keys in storage.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if storage is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the total size of all stored data in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.inner
            .iter()
            .map(|(k, v)| (k.len() + v.len()) as u64)
            .sum()
    }

    /// Computes a hash of the entire storage state.
    ///
    /// This is useful for verifying state consistency across replicas.
    pub fn state_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();

        // Sort keys for deterministic ordering
        let mut keys: Vec<_> = self.inner.keys().collect();
        keys.sort();

        for key in keys {
            if let Some(value) = self.inner.get(key) {
                hasher.update(key);
                hasher.update(value);
            }
        }

        hasher.finalize().into()
    }

    /// Creates a snapshot of the current storage state.
    pub fn snapshot(&self) -> StorageSnapshot {
        StorageSnapshot {
            data: self.inner.clone(),
            stats: self.stats.clone(),
        }
    }

    /// Restores from a snapshot.
    pub fn restore(&mut self, snapshot: StorageSnapshot) {
        self.inner = snapshot.data;
        self.stats = snapshot.stats;
    }

    /// Returns all keys matching a prefix.
    pub fn keys_with_prefix(&self, prefix: &[u8]) -> Vec<Vec<u8>> {
        self.inner
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Iterates over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Vec<u8>)> {
        self.inner.iter()
    }

    /// Direct get without fault injection (for debugging).
    pub fn get_unchecked(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.inner.get(key)
    }
}

impl ReadonlyKV for SimulatedStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        // Note: We need interior mutability for stats and rng.
        // For now, this returns without fault injection in the ReadonlyKV impl.
        // Use get_with_faults for fault injection.
        Ok(self.inner.get(key).cloned())
    }
}

impl SimulatedStorage {
    /// Gets a value with potential fault injection.
    ///
    /// Unlike the ReadonlyKV::get, this method can inject faults
    /// and updates statistics.
    pub fn get_with_faults(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.stats.reads = self.stats.reads.saturating_add(1);

        if self.should_fail_read() {
            self.stats.read_faults = self.stats.read_faults.saturating_add(1);
            self.log_op(StorageOp::Get {
                key: key.to_vec(),
                result: StorageOpResult::Fault,
            });
            return Err(ERR_SIMULATED_STORAGE_FAILURE);
        }

        // Clone the value first to avoid borrow conflicts
        let value = self.inner.get(key).cloned();

        match value {
            Some(ref v) => {
                self.stats.read_hits = self.stats.read_hits.saturating_add(1);
                self.stats.bytes_read = self
                    .stats
                    .bytes_read
                    .saturating_add((key.len() + v.len()) as u64);
                self.log_op(StorageOp::Get {
                    key: key.to_vec(),
                    result: StorageOpResult::Success,
                });
                Ok(Some(v.clone()))
            }
            None => {
                self.stats.read_misses = self.stats.read_misses.saturating_add(1);
                self.log_op(StorageOp::Get {
                    key: key.to_vec(),
                    result: StorageOpResult::NotFound,
                });
                Ok(None)
            }
        }
    }

    /// Sets a value with potential fault injection.
    pub fn set_with_faults(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), ErrorCode> {
        self.stats.writes = self.stats.writes.saturating_add(1);

        if self.should_fail_write() {
            self.stats.write_faults = self.stats.write_faults.saturating_add(1);
            self.log_op(StorageOp::Set {
                key: key.clone(),
                value: value.clone(),
                result: StorageOpResult::Fault,
            });
            return Err(ERR_SIMULATED_STORAGE_FAILURE);
        }

        self.stats.bytes_written = self
            .stats
            .bytes_written
            .saturating_add((key.len() + value.len()) as u64);
        self.log_op(StorageOp::Set {
            key: key.clone(),
            value: value.clone(),
            result: StorageOpResult::Success,
        });
        self.inner.insert(key, value);
        Ok(())
    }

    /// Removes a value with potential fault injection.
    pub fn remove_with_faults(&mut self, key: &[u8]) -> Result<(), ErrorCode> {
        self.stats.removes = self.stats.removes.saturating_add(1);

        if self.should_fail_write() {
            self.stats.remove_faults = self.stats.remove_faults.saturating_add(1);
            self.log_op(StorageOp::Remove {
                key: key.to_vec(),
                result: StorageOpResult::Fault,
            });
            return Err(ERR_SIMULATED_STORAGE_FAILURE);
        }

        self.log_op(StorageOp::Remove {
            key: key.to_vec(),
            result: StorageOpResult::Success,
        });
        self.inner.remove(key);
        Ok(())
    }
}

impl WritableKV for SimulatedStorage {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        for change in changes {
            match change {
                StateChange::Set { key, value } => {
                    self.stats.writes = self.stats.writes.saturating_add(1);
                    self.stats.bytes_written = self
                        .stats
                        .bytes_written
                        .saturating_add((key.len() + value.len()) as u64);
                    self.inner.insert(key, value);
                }
                StateChange::Remove { key } => {
                    self.stats.removes = self.stats.removes.saturating_add(1);
                    self.inner.remove(&key);
                }
            }
        }
        Ok(())
    }
}

/// A snapshot of storage state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSnapshot {
    pub data: HashMap<Vec<u8>, Vec<u8>>,
    pub stats: StorageStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut storage = SimulatedStorage::new(42, StorageConfig::no_faults());

        // Set and get
        storage.set_with_faults(b"key1".to_vec(), b"value1".to_vec()).unwrap();
        let value = storage.get_with_faults(b"key1").unwrap();
        assert_eq!(value, Some(b"value1".to_vec()));

        // Remove
        storage.remove_with_faults(b"key1").unwrap();
        let value = storage.get_with_faults(b"key1").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn test_read_fault_injection() {
        let config = StorageConfig::with_faults(1.0, 0.0); // 100% read failure
        let mut storage = SimulatedStorage::new(42, config);

        storage.inner.insert(b"key".to_vec(), b"value".to_vec());

        let result = storage.get_with_faults(b"key");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_SIMULATED_STORAGE_FAILURE);
        assert_eq!(storage.stats().read_faults, 1);
    }

    #[test]
    fn test_write_fault_injection() {
        let config = StorageConfig::with_faults(0.0, 1.0); // 100% write failure
        let mut storage = SimulatedStorage::new(42, config);

        let result = storage.set_with_faults(b"key".to_vec(), b"value".to_vec());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ERR_SIMULATED_STORAGE_FAILURE);
        assert_eq!(storage.stats().write_faults, 1);
    }

    #[test]
    fn test_statistics() {
        let config = StorageConfig::no_faults().with_logging();
        let mut storage = SimulatedStorage::new(42, config);

        storage.set_with_faults(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        storage.set_with_faults(b"k2".to_vec(), b"v2".to_vec()).unwrap();
        let _ = storage.get_with_faults(b"k1");
        let _ = storage.get_with_faults(b"k3"); // miss

        let stats = storage.stats();
        assert_eq!(stats.writes, 2);
        assert_eq!(stats.reads, 2);
        assert_eq!(stats.read_hits, 1);
        assert_eq!(stats.read_misses, 1);
    }

    #[test]
    fn test_state_hash_determinism() {
        let mut storage1 = SimulatedStorage::new(1, StorageConfig::no_faults());
        let mut storage2 = SimulatedStorage::new(2, StorageConfig::no_faults());

        // Same data should produce same hash regardless of insertion order
        storage1.set_with_faults(b"a".to_vec(), b"1".to_vec()).unwrap();
        storage1.set_with_faults(b"b".to_vec(), b"2".to_vec()).unwrap();

        storage2.set_with_faults(b"b".to_vec(), b"2".to_vec()).unwrap();
        storage2.set_with_faults(b"a".to_vec(), b"1".to_vec()).unwrap();

        assert_eq!(storage1.state_hash(), storage2.state_hash());
    }

    #[test]
    fn test_snapshot_restore() {
        let mut storage = SimulatedStorage::new(42, StorageConfig::no_faults());

        storage.set_with_faults(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        let snapshot = storage.snapshot();

        storage.set_with_faults(b"k2".to_vec(), b"v2".to_vec()).unwrap();
        assert_eq!(storage.len(), 2);

        storage.restore(snapshot);
        assert_eq!(storage.len(), 1);
        assert!(storage.get_with_faults(b"k2").unwrap().is_none());
    }

    #[test]
    fn test_operations_log() {
        let config = StorageConfig::no_faults().with_logging();
        let mut storage = SimulatedStorage::new(42, config);

        storage.set_with_faults(b"key".to_vec(), b"value".to_vec()).unwrap();
        let _ = storage.get_with_faults(b"key");
        storage.remove_with_faults(b"key").unwrap();

        assert_eq!(storage.operations_log().len(), 3);
    }
}
