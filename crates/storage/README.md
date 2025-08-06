# Evolve Storage

A storage layer for the Evolve blockchain framework that integrates with [Commonware](https://github.com/commonwarexyz/monorepo)'s authenticated database (ADB) system.

## Overview

This crate provides a bridge between Evolve's storage traits (`ReadonlyKV`) and Commonware's ADB implementation. It enables persistent, authenticated state storage with Merkle proof capabilities for blockchain consensus-critical data.

## Architecture

### Storage Traits

The crate defines three main traits:

- **`ReadonlyKV`**: Read-only key-value operations (imported from `evolve_core`)
- **`Storage`**: Async batch operations and commit functionality (extends `ReadonlyKVExt`)

### Architecture Layers

```ascii
┌─────────────────────────────────────────┐
│         Application Layer               │
│   (STF, Modules, Consensus)             │
└────────────────┬────────────────────────┘
                 │
┌────────────────▼────────────────────────┐
│         Storage Trait Layer             │
│    (ReadonlyKV + Storage)               │
└────────────────┬────────────────────────┘
                 │
┌────────────────▼────────────────────────┐
│       CommonwareStorage                 │
│  (Bridges Evolve traits to ADB API)     │
└────────────────┬────────────────────────┘
                 │
┌────────────────▼────────────────────────┐
│         Commonware ADB                  │
│   (Authenticated Database with MMR)     │
└─────────────────────────────────────────┘
```

### Implementation Details

The `CommonwareStorage` struct implements all storage traits using Commonware's ADB:

```rust
pub struct CommonwareStorage<C> {
    context: Arc<C>,
    adb: Arc<Mutex<Current<C, StorageKey, StorageValueChunk, Sha256, EightCap, 256>>>,
}
```

Key design decisions:

1. **Fixed-Size Types**: Uses Commonware's `FixedBytes<256>` for keys and `FixedBytes<4096>` for value chunks
2. **Async/Sync Bridge**: Uses `tokio::task::spawn_blocking` to bridge async ADB operations with sync trait methods
3. **Removal Strategy**: Since ADB doesn't support deletion, removed keys are set to empty values
4. **Thread Safety**: ADB is wrapped in `Arc<Mutex<...>>` for safe concurrent access

## Usage

### Basic Example

```rust
use evolve_storage::{CommonwareStorage, StorageConfig};
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::Runner as RunnerTrait;

// Configure the runtime
let runtime_config = TokioConfig::default()
    .with_storage_directory("./data")
    .with_worker_threads(4);

let runner = Runner::new(runtime_config);

runner.start(|context| async move {
    // Create storage configuration
    let config = StorageConfig {
        path: "./data".into(),
        partition_prefix: "evolve-state".to_string(),
        ..Default::default()
    };

    // Initialize storage
    let mut storage = CommonwareStorage::new(context, config).await?;

    // Basic read operation
    let value = storage.get(b"key").await?;
    
    // Write operations use batch API
    use evolve_storage::{Operation, Storage as StorageTrait};
    storage.batch(vec![
        Operation::Set { key: b"key".to_vec(), value: b"value".to_vec() },
    ]).await?;
    
    let value = storage.get(b"key").await?;
    assert_eq!(value, Some(b"value".to_vec()));
    
    // Batch operations
    use evolve_storage::{Operation, Storage as StorageTrait};
    storage.batch(vec![
        Operation::Set { key: b"k1".to_vec(), value: b"v1".to_vec() },
        Operation::Set { key: b"k2".to_vec(), value: b"v2".to_vec() },
    ]).await?;

    // Commit the state
    let commit_hash = storage.commit().await?;
});
```

### Configuration

The `StorageConfig` struct provides the following options:

```rust
pub struct StorageConfig {
    /// Path to the database directory
    pub path: PathBuf,
    
    /// Maximum database size in bytes (default: 100GB)
    pub max_size: u64,
    
    /// Cache size in bytes (default: 1GB)
    pub cache_size: u64,
    
    /// Write buffer size (default: 64MB)
    pub write_buffer_size: u64,
    
    /// Partition prefix for state storage (default: "evolve-state")
    pub partition_prefix: String,
}
```

## Storage Layout

The storage uses Commonware's ADB partition structure:

```tree
{partition_prefix}/
├── log-journal/      # Operation log journal
├── mmr-journal/      # Merkle Mountain Range journal
├── mmr-metadata/     # MMR metadata
└── bitmap-metadata/  # Bitmap metadata for tracking
```

Each partition serves a specific purpose in maintaining the authenticated state.

## Key Features

### Authenticated Storage

- Merkle Mountain Range (MMR) based authentication
- Cryptographic proofs for all stored data
- Efficient incremental hashing with SHA256

### Performance Optimizations

- Direct passthrough to Commonware's optimized storage
- Buffer pool for caching frequently accessed pages
- Write batching for improved throughput
- Lock-free reads where possible

### Type Safety

- Fixed-size keys and values enforced at compile time
- Type-safe error handling
- No runtime type conversions

## API Reference

### ReadonlyKV Operations

```rust
// Get a value by key
let value: Option<Vec<u8>> = storage.get(b"key")?;

// Check if a key exists (via ReadonlyKVExt)
let exists: bool = storage.exists(b"key")?;

// Get with default value (via ReadonlyKVExt)
let value: Vec<u8> = storage.get_or_default(b"key", b"default".to_vec())?;
```


### Storage Operations (Async)

```rust
// Batch operations
let operations = vec![
    Operation::Set { key: b"k1".to_vec(), value: b"v1".to_vec() },
    Operation::Remove { key: b"k2".to_vec() },
];
storage.batch(operations).await?;

// Commit and get hash
let commit_hash = storage.commit().await?;
```

## Limitations

1. **Value Size**: Currently limited to 4KB per value (STORAGE_VALUE_SIZE) <!-- TODO: make this bigger-->
2. **Key Size**: Fixed at 256 bytes (MAX_KEY_SIZE)
3. **Deletion**: No true deletion - removed keys are set to empty values
4. **Batch Size**: Limited to 10,000 operations per batch (MAX_BATCH_SIZE)

## Error Handling

The crate uses Evolve's `ErrorCode` system with specific storage error codes:

```rust
pub const ERR_STORAGE_IO: ErrorCode = ErrorCode::new(500, "storage I/O error");
pub const ERR_STORAGE_CORRUPTION: ErrorCode = ErrorCode::new(501, "storage corruption detected");
pub const ERR_STORAGE_FULL: ErrorCode = ErrorCode::new(502, "storage full");
pub const ERR_INVALID_COMMIT: ErrorCode = ErrorCode::new(503, "invalid commit hash");
pub const ERR_KEY_TOO_LARGE: ErrorCode = ErrorCode::new(504, "key exceeds maximum size");
pub const ERR_VALUE_TOO_LARGE: ErrorCode = ErrorCode::new(505, "value exceeds maximum size");
pub const ERR_BATCH_TOO_LARGE: ErrorCode = ErrorCode::new(506, "batch exceeds maximum size");
```

## Testing

Run the test suite:

```bash
cargo test -p evolve_storage
```

The test includes basic CRUD operations and demonstrates integration with Commonware's runtime.

## Dependencies

- `commonware-storage`: Provides the ADB implementation
- `commonware-runtime`: Runtime traits (Storage, Clock, Metrics) and buffer pool
- `commonware-cryptography`: SHA256 hasher for Merkle proofs
- `commonware-utils`: FixedBytes array type
- `commonware-codec`: Encoding/decoding for fixed-size types
- `tokio`: Async runtime for bridging sync/async APIs
- `futures`: For executing futures in blocking context

## Future Improvements

1. **Value Chunking**: Support for values larger than 4KB by implementing chunking
2. **True Deletion**: Implement proper key deletion when ADB supports it
3. **Prefix Iteration**: Add support for iterating over keys with a common prefix
4. **Compression**: Add optional value compression
5. **Metrics**: Expose storage metrics through the Metrics trait
6. **Migration Tools**: Tools for migrating from other storage backends
7. **Batch API**: Use native Commonware batch operations when available

## Security Considerations

1. **Key Namespacing**: Should be enforced by higher layers to prevent collisions
2. **Access Control**: Handled by runtime layer, not storage
3. **Data Integrity**: Leverages Commonware's merkle tree authentication
4. **Resource Limits**: Enforced to prevent DoS attacks

## License

This crate is part of the Evolve SDK and follows the same license terms.
