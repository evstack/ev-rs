# Server Integration Guide

This guide covers the `EvolveServer` - a **composition layer** that orchestrates infrastructure concerns for Evolve applications. It sits above the SDK, providing a default integration of storage, indexing, RPC, and observability that consensus layers can use directly.

**Crate:** `evolve_server` (`crates/app/server`)

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Consensus Layer                          │
│              (CometBFT, Raft, custom, etc.)                 │
└─────────────────────────┬───────────────────────────────────┘
                          │ register_block() + commit()
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                      EvolveServer                            │
│  ┌───────────┐  ┌───────────┐  ┌─────────────┐              │
│  │  Storage  │  │    STF    │  │  Shutdown   │              │
│  │   (Arc)   │  │   (Arc)   │  │ Coordinator │              │
│  └───────────┘  └───────────┘  └─────────────┘              │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │                    IndexerHandle                         ││
│  │   Phase 1: Chain Data → Phase 2: State Snapshots        ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
                          │
          ┌───────────────┼───────────────┐
          ▼               ▼               ▼
    ┌──────────┐    ┌──────────┐    ┌──────────┐
    │ JSON-RPC │    │   gRPC   │    │  Indexer │
    │  Server  │    │  Server  │    │   Task   │
    └──────────┘    └──────────┘    └──────────┘
```

## What the Server Provides

| Component | Description |
|-----------|-------------|
| **Storage Management** | Arc-wrapped storage backends for thread-safe sharing |
| **STF Composition** | Wraps your state transition function for concurrent access |
| **Two-Phase Indexing** | Non-blocking chain data + state snapshot indexing |
| **Configuration** | YAML config loading with sensible defaults |
| **Observability** | Logging initialization (json/pretty formats) |
| **Graceful Shutdown** | Coordinated shutdown with configurable timeout |

## What You Provide

| Component | Description |
|-----------|-------------|
| **STF** | Your state transition function with modules, hooks, validators |
| **Storage** | Storage backend (persistent or mock) |
| **Account Codes** | Contract bytecode storage |
| **Consensus Integration** | Logic to feed blocks into the server |

## Assumptions

### Block Execution Model

1. **Sequential execution** - Blocks are processed one at a time
2. **Single pending block** - Only one block can be "in flight" between `register_block()` and `commit()`
3. **Synchronous STF** - The STF executes synchronously; the server waits for completion
4. **Deterministic execution** - Same block + state must produce same result

### Storage

- **Persistent storage** at configurable path (default: `./data`)
- **KV-store interface** - LevelDB/RocksDB-style semantics expected
- **Thread-safe** - Storage must be `Send + Sync + 'static`

### Networking (Optional)

- JSON-RPC binds to `127.0.0.1:8545` by default
- gRPC binds to `127.0.0.1:9545` (disabled by default)
- Both are optional - the server works without them

### Concurrency

- **Tokio runtime** - async/await throughout
- **Arc for sharing** - STF, Storage, Codes are Arc-wrapped
- **mpsc channels** - Indexer uses bounded channels (default: 256)
- **broadcast channels** - Subscriptions use tokio broadcast

## Usage

### Basic Setup (Using Defaults)

```rust
use evolve_server::{EvolveServer, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Your application components
    let stf = build_my_stf();
    let storage = MyStorage::new();
    let codes = MyAccountCodes::new();

    // Build server with defaults
    let server = EvolveServer::<MyStf, MyStorage, MyAccountCodes>::builder()
        .build(stf, storage, codes)
        .await?;

    // Consensus loop
    loop {
        let block = receive_block_from_consensus().await;

        // Execute through STF
        let result = server.stf().apply_block(
            server.storage().as_ref(),
            server.account_codes().as_ref(),
            &block,
        );

        // Register and commit
        let block_data = extract_block_data(&result);
        server.register_block(height, hash, block_data).await?;
        server.commit(|| storage.commit()).await?;
    }
}
```

### With Configuration File

```rust
let server = EvolveServer::<MyStf, MyStorage, MyAccountCodes>::builder()
    .config_path("./config.yaml")
    .build(stf, storage, codes)
    .await?;
```

### With Explicit Configuration

```rust
use evolve_operations::config::NodeConfig;

let config = NodeConfig {
    chain: ChainConfig { chain_id: 42, ..Default::default() },
    storage: StorageConfig {
        path: "/var/data/evolve".to_string(),
        cache_size: 2 * 1024 * 1024 * 1024, // 2GB
        ..Default::default()
    },
    rpc: RpcConfig { http_addr: "0.0.0.0:8545".to_string(), ..Default::default() },
    ..Default::default()
};

let server = EvolveServer::<MyStf, MyStorage, MyAccountCodes>::builder()
    .config(config)
    .indexer_buffer(512)
    .build(stf, storage, codes)
    .await?;
```

## Building a Custom Server

The `EvolveServer` is one way to compose Evolve infrastructure. You can build your own server by using the underlying components directly:

### Option 1: Wrap EvolveServer

Extend the default server with additional functionality:

```rust
pub struct MyServer {
    inner: EvolveServer<MyStf, MyStorage, MyCodes>,
    metrics: MetricsCollector,
    custom_rpc: CustomRpcServer,
}

impl MyServer {
    pub async fn new() -> Result<Self, Error> {
        let inner = EvolveServer::builder().build(stf, storage, codes).await?;
        Ok(Self {
            inner,
            metrics: MetricsCollector::new(),
            custom_rpc: CustomRpcServer::new(),
        })
    }

    pub async fn run_block(&self, block: Block) -> Result<(), Error> {
        // Custom logic before
        self.metrics.record_block_start();

        // Use inner server
        let result = self.inner.stf().apply_block(/*...*/);
        self.inner.register_block(/*...*/).await?;
        self.inner.commit(|| /*...*/).await?;

        // Custom logic after
        self.metrics.record_block_complete();
        self.custom_rpc.notify_new_block();
        Ok(())
    }
}
```

### Option 2: Build from Components

For full control, compose the pieces yourself:

```rust
use evolve_stf::Stf;
use evolve_storage::Storage;
use evolve_operations::shutdown::ShutdownCoordinator;
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;

pub struct CustomServer<S, T, C> {
    stf: Arc<S>,
    storage: Arc<T>,
    codes: Arc<C>,
    shutdown: ShutdownCoordinator,
    // Your custom components
    consensus: Box<dyn Consensus>,
    p2p: P2PNetwork,
}

impl<S, T, C> CustomServer<S, T, C>
where
    S: Send + Sync + 'static,
    T: Send + Sync + 'static,
    C: Send + Sync + 'static,
{
    pub fn new(
        stf: S,
        storage: T,
        codes: C,
        consensus: impl Consensus + 'static,
        p2p: P2PNetwork,
    ) -> Self {
        Self {
            stf: Arc::new(stf),
            storage: Arc::new(storage),
            codes: Arc::new(codes),
            shutdown: ShutdownCoordinator::new(Duration::from_secs(30)),
            consensus: Box::new(consensus),
            p2p,
        }
    }

    pub async fn run(&self) -> Result<(), Error> {
        loop {
            tokio::select! {
                block = self.consensus.next_block() => {
                    self.execute_block(block?).await?;
                }
                _ = self.shutdown.wait() => break,
            }
        }
        Ok(())
    }

    async fn execute_block(&self, block: Block) -> Result<(), Error> {
        // Direct STF execution - no intermediate server
        let result = self.stf.apply_block(&*self.storage, &*self.codes, &block);

        // Commit directly
        self.storage.commit()?;

        // Custom indexing
        self.index_block(&block, &result).await;

        // P2P broadcast
        self.p2p.broadcast_block(&block).await;

        Ok(())
    }
}
```

### Option 3: Mix and Match

Use specific components from the server crate:

```rust
use evolve_server::{IndexerHandle, spawn_indexer, BlockData};

// Use just the indexer
let indexer = spawn_indexer(256);

// Your own block execution
let result = my_stf.execute(block);

// Send to indexer
indexer.send_chain_data(height, hash, BlockData {
    transactions: result.tx_bytes,
    receipts: result.receipts,
    logs: result.logs,
});

// Commit storage yourself
my_storage.commit()?;

// Signal state snapshot
indexer.send_state_snapshot(height, hash);
```

## Configuration Reference

### config.yaml

```yaml
chain:
  chain_id: 1
  gas:
    storage_get_charge: 10
    storage_set_charge: 10
    storage_remove_charge: 10

storage:
  path: "./data"
  cache_size: 1073741824      # 1GB
  write_buffer_size: 67108864 # 64MB
  partition_prefix: "evolve-state"

rpc:
  http_addr: "127.0.0.1:8545"
  ws_addr: "127.0.0.1:8546"
  max_connections: 100
  enabled: true

grpc:
  addr: "127.0.0.1:9545"
  enabled: false  # Disabled by default

operations:
  shutdown_timeout_secs: 30

observability:
  log_level: "info"
  log_format: "pretty"  # or "json"
  metrics_enabled: true
```

## Two-Phase Indexing

The indexer processes blocks in two phases to maximize concurrency:

### Phase 1: Chain Data (Immediate)

Happens right after `apply_block()`, before `commit()`:

- Block headers
- Transaction data
- Receipts
- Event logs

No committed state needed - can run in parallel with other work.

### Phase 2: State Snapshots (After Commit)

Happens after `commit()`:

- Account balances
- Contract storage changes
- State root updates

Requires committed state to be readable.

```
apply_block() ─────► register_block() ─────► commit()
                           │                    │
                           ▼                    ▼
                     Phase 1 Index        Phase 2 Index
                    (chain data)         (state snapshot)
```

## Error Handling

| Error | Cause | Resolution |
|-------|-------|------------|
| `NoPendingBlock` | `commit()` called without `register_block()` | Call `register_block()` first |
| `Config` | Invalid config file or values | Check YAML syntax and values |
| `Storage` | Storage backend failure | Check disk space, permissions |
| `Execution` | STF execution failure | Check transaction validity |

## Thread Safety

All public methods are thread-safe:

- `EvolveServer` is `Send + Sync`
- `register_block()` and `commit()` use `RwLock` internally
- `IndexerHandle` is `Clone` + `Send + Sync`
- STF/Storage/Codes are `Arc`-wrapped

However, **block execution is inherently sequential** - the pending block state machine enforces this.

## Shutdown

```rust
// Graceful shutdown
server.shutdown().await?;
```

Shutdown sequence:
1. Signal indexer to stop
2. Wait for in-flight work to complete
3. Signal shutdown coordinator
4. Wait for timeout (default: 30s)
