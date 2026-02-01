# Composing Custom Evolve Nodes

This guide explains how to build custom Evolve nodes by composing the available components. The `evd` binary serves as the reference implementation.

## Architecture Overview

An Evolve node consists of these core components:

```
┌─────────────────────────────────────────────────────────────┐
│                        Custom Node                          │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │   Storage   │  │  Account    │  │    State Transition │ │
│  │   (QMDB)    │  │   Codes     │  │    Function (STF)   │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │   Mempool   │  │  JSON-RPC   │  │    gRPC Server      │ │
│  │             │  │   Server    │  │    (evnode)         │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Component Reference

### 1. Storage (evolve_storage)

QMDB provides persistent, crash-safe storage with async commit.

```rust
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use commonware_runtime::tokio::{Config as TokioConfig, Runner, Context};

// Configure storage
let storage_config = StorageConfig {
    path: "./data".into(),
    ..Default::default()
};

// Create within async runtime
let storage = QmdbStorage::new(context, storage_config)
    .await
    .expect("failed to create storage");
```

Key traits:
- `ReadonlyKV` - Key-value reads (get)
- `Storage` - Batched writes and commits (batch, commit)

### 2. Account Codes (AccountsCodeStorage)

Account codes define the logic for different account types.

```rust
use evolve_stf_traits::{AccountsCodeStorage, WritableAccountsCodeStorage};
use evolve_testapp::install_account_codes;
use evolve_testing::server_mocks::AccountStorageMock;

// Create and populate code storage
let mut codes = AccountStorageMock::default();
install_account_codes(&mut codes);
```

### 3. State Transition Function (evolve_stf)

The STF processes transactions and produces state changes.

```rust
use evolve_testapp::{build_mempool_stf, default_gas_config};
use evolve_core::AccountId;

// Build STF with gas config and scheduler account
let gas_config = default_gas_config();
let scheduler_account = AccountId::new(65538); // From genesis
let stf = build_mempool_stf(gas_config, scheduler_account);
```

### 4. Mempool (evolve_mempool)

Collects and orders pending transactions.

```rust
use evolve_mempool::{new_shared_mempool, Mempool, SharedMempool};
use evolve_tx_eth::TxContext;

// Create shared mempool for ETH transactions
let mempool: SharedMempool<Mempool<TxContext>> = new_shared_mempool();
```

Transaction types:
- `TxContext` - Ethereum RLP transactions (type 0x02)
- `MicroTxContext` - Minimal fixed-layout transactions (type 0x83)

### 5. JSON-RPC Server (evolve_eth_jsonrpc)

Provides Ethereum-compatible RPC interface for queries.

```rust
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex};
use evolve_rpc_types::SyncStatus;
use alloy_primitives::U256;

// Create chain index for block/tx queries
let chain_index = Arc::new(PersistentChainIndex::new(Arc::new(storage.clone())));
chain_index.initialize()?;

// Create subscription manager for real-time events
let subscriptions = Arc::new(SubscriptionManager::new());

// Configure state provider
let state_provider_config = ChainStateProviderConfig {
    chain_id: 1,
    protocol_version: "0x1".to_string(),
    gas_price: U256::ZERO,
    sync_status: SyncStatus::NotSyncing(false),
};

let state_provider = ChainStateProvider::with_mempool(
    Arc::clone(&chain_index),
    state_provider_config,
    Arc::new(codes),
    mempool.clone(),
);

// Start server
let server_config = RpcServerConfig {
    http_addr: "127.0.0.1:8545".parse()?,
    chain_id: 1,
    client_version: "my-node/0.1.0".to_string(),
};

let handle = start_server_with_subscriptions(
    server_config,
    state_provider,
    subscriptions,
).await?;
```

### 6. gRPC Server (evolve_evnode)

Exposes the execution layer to external consensus (ev-node, rollkit).

```rust
use evolve_evnode::{EvnodeServer, EvnodeServerConfig, ExecutorServiceConfig, StateChangeCallback};

let config = EvnodeServerConfig {
    addr: "127.0.0.1:50051".parse()?,
    enable_gzip: true,
    max_message_size: 4 * 1024 * 1024,
    executor_config: ExecutorServiceConfig {
        max_gas: 30_000_000,
        max_bytes: 128 * 1024,
    },
};

// With mempool support
let server = EvnodeServer::with_mempool(
    config,
    stf,
    storage,
    codes,
    mempool,
).with_state_change_callback(callback);

server.serve().await?;
```

## Genesis Initialization

Every node needs genesis to bootstrap initial state:

```rust
use evolve_core::BlockContext;
use evolve_node::GenesisOutput;
use evolve_server::{load_chain_state, save_chain_state, ChainState, CHAIN_STATE_KEY};

// Check for existing state
if let Some(state) = load_chain_state::<GenesisAccounts, _>(&storage) {
    // Resume from existing state
    return (state.genesis_result, state.height);
}

// Run genesis
let genesis_stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
let genesis_block = BlockContext::new(0, 0);

let (accounts, state) = genesis_stf
    .system_exec(&storage, &codes, genesis_block, |env| {
        do_genesis_inner(env)
    })?;

let changes = state.into_changes()?;

// Commit genesis state
let operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();
storage.batch(operations).await?;
storage.commit().await?;
```

## Runtime Setup

Use commonware-runtime for async with proper shutdown handling:

```rust
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};

let runtime_config = TokioConfig::default()
    .with_storage_directory("./data")
    .with_worker_threads(4);

let runner = Runner::new(runtime_config);

runner.start(move |context| {
    async move {
        let context_for_shutdown = context.clone();

        // Initialize components...

        // Run with shutdown handling
        tokio::select! {
            result = server.serve() => {
                // Handle result
            }
            _ = tokio::signal::ctrl_c() => {
                context_for_shutdown
                    .stop(0, Some(Duration::from_secs(10)))
                    .await?;
            }
        }
    }
});
```

## Full Example: evd

See `bin/evd/src/main.rs` for the complete reference implementation that includes:

- QMDB persistent storage
- Account code registration
- Genesis handling with state persistence
- Shared mempool for transaction collection
- JSON-RPC server for queries
- gRPC server for external consensus integration
- Graceful shutdown with state saving

## Transaction Format Support

### ETH Transactions (default)

Standard Ethereum RLP-encoded transactions. Use `evolve_tx_eth::TxContext`.

### Micro Transactions (high throughput)

Minimal 150-byte fixed-layout format. Use `evolve_tx_micro::MicroTxContext`.

```rust
use evolve_tx_micro::{MicroGateway, MicroTxContext, SignedMicroTx};

// Decode and verify micro transaction
let gateway = MicroGateway::new(chain_id);
let tx_context = gateway.decode_and_verify(raw_bytes)?;

// Add to mempool
mempool.write().await.add(tx_context)?;
```

Micro TX format:

| Field | Size | Description |
|-------|------|-------------|
| chain_id | 8 | u64 big-endian |
| from | 32 | Ed25519 public key |
| to | 20 | Recipient address |
| amount | 16 | u128 big-endian |
| timestamp | 8 | u64 milliseconds |
| data_len | 2 | u16 calldata length |
| data | var | Optional calldata |
| signature | 64 | Ed25519 signature |

## CLI Pattern

Use clap for structured command-line interfaces:

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "my-node")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long, default_value = "./data")]
        data_dir: String,

        #[arg(long, default_value = "127.0.0.1:50051")]
        grpc_addr: String,

        #[arg(long, default_value = "127.0.0.1:8545")]
        rpc_addr: String,
    },
    Init {
        #[arg(long, default_value = "./data")]
        data_dir: String,
    },
}
```

## Crate Dependencies

Minimal Cargo.toml for a custom node:

```toml
[dependencies]
# Core
evolve_core.workspace = true
evolve_stf.workspace = true
evolve_stf_traits.workspace = true

# Storage
evolve_storage.workspace = true

# Networking
evolve_mempool.workspace = true
evolve_evnode = { workspace = true, features = ["testapp"] }
evolve_eth_jsonrpc.workspace = true
evolve_chain_index.workspace = true
evolve_rpc_types.workspace = true

# Application
evolve_testapp.workspace = true
evolve_testing.workspace = true
evolve_tx_eth.workspace = true
evolve_node.workspace = true
evolve_server.workspace = true

# External
alloy-primitives.workspace = true
commonware-runtime.workspace = true
clap.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
borsh.workspace = true
```

## Best Practices

1. Always persist chain state - Save height and genesis result on shutdown
2. Use shared mempool - Enables both RPC submission and gRPC GetTxs
3. Initialize chain index - Required for block/tx queries via JSON-RPC
4. Handle shutdown gracefully - Use tokio select with Ctrl-C handling
5. Configure logging - Use tracing-subscriber with EnvFilter for RUST_LOG support
