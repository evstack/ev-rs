# Evolve Storage

A storage layer for the Evolve blockchain framework that integrates with [Commonware](https://github.com/commonwarexyz/monorepo)'s authenticated database system (QMDB).

## Overview

This crate provides a bridge between Evolve's storage traits (`ReadonlyKV`) and Commonware's QMDB implementation. It enables persistent, authenticated state storage with Merkle proof capabilities for blockchain consensus-critical data.

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
│         QmdbStorage                     │
│  (Bridges Evolve traits to QMDB API)    │
└────────────────┬────────────────────────┘
                 │
┌────────────────▼────────────────────────┐
│         Commonware QMDB                 │
│   (Authenticated Database with MMR)     │
└─────────────────────────────────────────┘
```

### Implementation Details

The `QmdbStorage` struct implements all storage traits using Commonware's QMDB:

```rust
pub struct QmdbStorage<C> { /* ... */ }
```

Key design decisions:

1. **Fixed-Size Types**: Uses Commonware's `FixedBytes<256>` for keys and `FixedBytes<4096>` for value chunks
2. **Async/Sync Bridge**: Uses blocking `block_on` to bridge async QMDB operations with sync trait methods
3. **Deletion**: Uses native QMDB delete semantics
4. **Thread Safety**: QMDB is protected with an async `RwLock` and in-memory read caches

## Usage

### Basic Example

```rust
use evolve_storage::{QmdbStorage, StorageConfig};
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
    let mut storage = QmdbStorage::new(context, config).await?;

    // Basic read operation
    let value = storage.get(b"key")?;
    
    // Write operations use batch API
    use evolve_storage::{Operation, Storage as StorageTrait};
    storage.batch(vec![
        Operation::Set { key: b"key".to_vec(), value: b"value".to_vec() },
    ]).await?;
    
    let value = storage.get(b"key")?;
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
    
    /// Cache size in bytes (default: 1GB)
    pub cache_size: u64,
    
    /// Write buffer size (default: 64MB)
    pub write_buffer_size: u64,
    
    /// Partition prefix for state storage (default: "evolve-state")
    pub partition_prefix: String,
}
```

Note: `StorageConfig::path` is validated by node startup checks and used to configure the
Commonware runtime storage directory. `QmdbStorage` itself relies on the runtime context.

## Storage Layout

The storage uses Commonware's QMDB partition structure:

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

1. **Value Size**: Currently limited to 4092 bytes per value (4KB minus length prefix)
2. **Key Size**: Payload keys capped at 254 bytes (2-byte length prefix in 256-byte storage key)
3. **Deletion**: Supported via QMDB delete operations
4. **Batch Size**: Limited to 10,000 operations per batch (MAX_BATCH_SIZE)

## Error Handling

The crate uses Evolve's `ErrorCode` system with specific storage error codes
(see `crates/storage/src/types.rs` for the authoritative list):

```rust
pub const ERR_STORAGE_IO: ErrorCode = ErrorCode::new(...);
pub const ERR_ADB_ERROR: ErrorCode = ErrorCode::new(...);
pub const ERR_CONCURRENCY_ERROR: ErrorCode = ErrorCode::new(...);
pub const ERR_RUNTIME_ERROR: ErrorCode = ErrorCode::new(...);
pub const ERR_KEY_TOO_LARGE: ErrorCode = ErrorCode::new(...);
pub const ERR_VALUE_TOO_LARGE: ErrorCode = ErrorCode::new(...);
pub const ERR_BATCH_TOO_LARGE: ErrorCode = ErrorCode::new(...);
```

## Testing

Run the test suite:

```bash
cargo test -p evolve_storage
```

The test includes basic CRUD operations and demonstrates integration with Commonware's runtime.

## Dependencies

- `commonware-storage`: Provides the QMDB implementation
- `commonware-runtime`: Runtime traits (Storage, Clock, Metrics) and buffer pool
- `commonware-cryptography`: SHA256 hasher for Merkle proofs
- `commonware-utils`: FixedBytes array type
- `commonware-codec`: Encoding/decoding for fixed-size types
- `tokio`: Async runtime for bridging sync/async APIs
- `futures`: For executing futures in blocking context

## Future Improvements

1. **Value Chunking**: Support for values larger than 4092 bytes by implementing chunking
2. **Deletion Semantics**: Clarify deletion guarantees with upstream QMDB versions
3. **Prefix Iteration**: Add support for iterating over keys with a common prefix
4. **Compression**: Add optional value compression
5. **Metrics**: Expose storage metrics through the Metrics trait
6. **Migration Tools**: Tools for migrating from other storage backends
7. **Batch API**: Use native Commonware batch operations when available

## Security Considerations

1. **Key Namespacing**: Should be enforced by higher layers to prevent collisions
2. **Key Encoding**: Keys use a 2-byte length prefix inside a 256-byte storage key
3. **Access Control**: Handled by runtime layer, not storage
4. **Data Integrity**: Leverages Commonware's merkle tree authentication
5. **Resource Limits**: Enforced to prevent DoS attacks

## License

This crate is part of the Evolve SDK and follows the same license terms.
