use commonware_utils::sequence::FixedBytes;
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

impl From<evolve_stf_traits::StateChange> for Operation {
    fn from(change: evolve_stf_traits::StateChange) -> Self {
        match change {
            evolve_stf_traits::StateChange::Set { key, value } => Operation::Set { key, value },
            evolve_stf_traits::StateChange::Remove { key } => Operation::Remove { key },
        }
    }
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

/// Length prefix size for storage keys (2 bytes for u16 length)
pub const KEY_LENGTH_PREFIX_SIZE: usize = 2;
/// Storage key size for commonware integration (fixed-size type).
pub const STORAGE_KEY_SIZE: usize = 256;
/// Maximum sizes for keys and values
/// Note: storage keys include a 2-byte length prefix, so payload keys are capped at 254 bytes.
pub const MAX_KEY_SIZE: usize = STORAGE_KEY_SIZE - KEY_LENGTH_PREFIX_SIZE;
pub const MAX_BATCH_SIZE: usize = 10_000; // 10k operations

// For commonware integration, we use fixed-size types
// Keys are 256 bytes (padded with zeros if needed)
pub type StorageKey = FixedBytes<STORAGE_KEY_SIZE>;
// Values are stored in 4KB chunks
pub const STORAGE_VALUE_SIZE: usize = 4096;
pub type StorageValueChunk = FixedBytes<STORAGE_VALUE_SIZE>;

/// Helper functions for creating storage keys
pub fn create_storage_key(key: &[u8]) -> Result<StorageKey, ErrorCode> {
    if key.len() > MAX_KEY_SIZE {
        return Err(ERR_KEY_TOO_LARGE);
    }

    let mut data = [0u8; STORAGE_KEY_SIZE];
    let len_bytes = (key.len() as u16).to_le_bytes();
    data[..KEY_LENGTH_PREFIX_SIZE].copy_from_slice(&len_bytes);
    data[KEY_LENGTH_PREFIX_SIZE..KEY_LENGTH_PREFIX_SIZE + key.len()].copy_from_slice(key);

    Ok(StorageKey::new(data))
}

/// Length prefix size for value storage (4 bytes for u32 length)
pub const VALUE_LENGTH_PREFIX_SIZE: usize = 4;

/// Maximum actual value size (chunk size minus length prefix)
pub const MAX_VALUE_DATA_SIZE: usize = STORAGE_VALUE_SIZE - VALUE_LENGTH_PREFIX_SIZE;
/// Maximum value size accepted by the storage layer.
pub const MAX_VALUE_SIZE: usize = MAX_VALUE_DATA_SIZE;

/// Helper function for creating storage value chunks
///
/// Stores value with a 4-byte length prefix to preserve exact data semantics.
/// Format: [len_u32_le][data][padding]
pub fn create_storage_value_chunk(value: &[u8]) -> Result<StorageValueChunk, ErrorCode> {
    if value.len() > MAX_VALUE_DATA_SIZE {
        return Err(ERR_VALUE_TOO_LARGE);
    }

    let mut data = [0u8; STORAGE_VALUE_SIZE];
    // Store length as 4-byte little-endian prefix
    let len_bytes = (value.len() as u32).to_le_bytes();
    data[..VALUE_LENGTH_PREFIX_SIZE].copy_from_slice(&len_bytes);
    // Store actual value after length prefix
    data[VALUE_LENGTH_PREFIX_SIZE..VALUE_LENGTH_PREFIX_SIZE + value.len()].copy_from_slice(value);

    Ok(StorageValueChunk::new(data))
}

/// Extract value from storage chunk by reading length prefix
///
/// Returns the exact bytes that were stored, preserving trailing zeros.
pub fn extract_value_from_chunk(chunk: &StorageValueChunk) -> Option<Vec<u8>> {
    let data = chunk.as_ref();
    // Read length from 4-byte little-endian prefix
    let len_bytes: [u8; 4] = data[..VALUE_LENGTH_PREFIX_SIZE].try_into().ok()?;
    let len = u32::from_le_bytes(len_bytes) as usize;

    // Validate length
    if len > MAX_VALUE_DATA_SIZE {
        return None;
    }

    // Extract exactly 'len' bytes of actual data
    Some(data[VALUE_LENGTH_PREFIX_SIZE..VALUE_LENGTH_PREFIX_SIZE + len].to_vec())
}
