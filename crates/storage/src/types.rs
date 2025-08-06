use commonware_utils::array::FixedBytes;
use evolve_core::{define_error, ErrorCode};
use std::fmt;

/// Represents a commit hash from the storage layer
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommitHash([u8; 32]);

impl CommitHash {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl fmt::Debug for CommitHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitHash({})", hex::encode(self.0))
    }
}

impl fmt::Display for CommitHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl From<[u8; 32]> for CommitHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

/// Storage operation for batch processing
#[derive(Debug, Clone)]
pub enum Operation {
    Set { key: Vec<u8>, value: Vec<u8> },
    Remove { key: Vec<u8> },
}

/// Storage configuration
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Path to the database directory
    pub path: std::path::PathBuf,

    /// Cache size in bytes (passed to Commonware)
    pub cache_size: u64,

    /// Write buffer size
    pub write_buffer_size: u64,

    /// Partition prefix for evolve state
    pub partition_prefix: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: std::path::PathBuf::from("./data"),
            cache_size: 1024 * 1024 * 1024,      // 1GB
            write_buffer_size: 64 * 1024 * 1024, // 64MB
            partition_prefix: "evolve-state".to_string(),
        }
    }
}

define_error!(ERR_STORAGE_IO, 0x1, "storage I/O error");
define_error!(ERR_ADB_ERROR, 0x2, "ADB operation failed");
define_error!(ERR_CONCURRENCY_ERROR, 0x3, "concurrency error");
define_error!(ERR_RUNTIME_ERROR, 0x4, "runtime error");
define_error!(ERR_KEY_TOO_LARGE, 0x5, "key exceeds maximum size");
define_error!(ERR_VALUE_TOO_LARGE, 0x6, "value exceeds maximum size");
define_error!(ERR_BATCH_TOO_LARGE, 0x7, "batch exceeds maximum size");

/// Maximum sizes for keys and values
pub const MAX_KEY_SIZE: usize = 256;
pub const MAX_VALUE_SIZE: usize = 10 * 1024 * 1024; // 10MB
pub const MAX_BATCH_SIZE: usize = 10_000; // 10k operations

// For commonware integration, we use fixed-size types
// Keys are 256 bytes (padded with zeros if needed)
pub type StorageKey = FixedBytes<256>;
// Values are stored in 4KB chunks
pub const STORAGE_VALUE_SIZE: usize = 4096;
pub type StorageValueChunk = FixedBytes<STORAGE_VALUE_SIZE>;

/// Helper functions for creating storage keys
pub fn create_storage_key(key: &[u8]) -> Result<StorageKey, ErrorCode> {
    if key.len() > MAX_KEY_SIZE {
        return Err(ERR_KEY_TOO_LARGE);
    }

    let mut data = [0u8; MAX_KEY_SIZE];
    data[..key.len()].copy_from_slice(key);

    Ok(StorageKey::new(data))
}

/// Helper function for creating storage value chunks
pub fn create_storage_value_chunk(value: &[u8]) -> Result<StorageValueChunk, ErrorCode> {
    if value.len() > STORAGE_VALUE_SIZE {
        return Err(ERR_VALUE_TOO_LARGE);
    }

    let mut data = [0u8; STORAGE_VALUE_SIZE];
    data[..value.len()].copy_from_slice(value);

    Ok(StorageValueChunk::new(data))
}
