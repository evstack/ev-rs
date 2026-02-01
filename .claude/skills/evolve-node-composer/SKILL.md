---
name: evolve-node-composer
description: Create custom Evolve blockchain nodes by composing storage, STF, mempool, RPC, and gRPC components. Use when the user wants to create a new node binary, build a custom chain, compose node components, or asks about evd/testapp architecture.
---

# Evolve Node Composer

Guide for creating custom Evolve nodes by composing the available components.

## Quick Reference

Reference implementation: `bin/evd/src/main.rs`

## Core Components

### 1. Storage (evolve_storage)

QMDB provides persistent storage with async commit.

```rust
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use commonware_runtime::tokio::{Config as TokioConfig, Runner, Context};

let storage_config = StorageConfig {
    path: "./data".into(),
    ..Default::default()
};

let storage = QmdbStorage::new(context, storage_config)
    .await
    .expect("failed to create storage");
```

### 2. Account Codes (AccountsCodeStorage)

```rust
use evolve_stf_traits::{AccountsCodeStorage, WritableAccountsCodeStorage};
use evolve_testapp::install_account_codes;
use evolve_testing::server_mocks::AccountStorageMock;

let mut codes = AccountStorageMock::default();
install_account_codes(&mut codes);
```

### 3. State Transition Function (evolve_stf)

```rust
use evolve_testapp::{build_mempool_stf, default_gas_config};
use evolve_core::AccountId;

let gas_config = default_gas_config();
let scheduler_account = AccountId::new(65538);
let stf = build_mempool_stf(gas_config, scheduler_account);
```

### 4. Mempool (evolve_mempool)

```rust
use evolve_mempool::{new_shared_mempool, Mempool, SharedMempool};
use evolve_tx_eth::TxContext;

let mempool: SharedMempool<Mempool<TxContext>> = new_shared_mempool();
```

### 5. JSON-RPC Server (evolve_eth_jsonrpc)

```rust
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex};

let chain_index = Arc::new(PersistentChainIndex::new(Arc::new(storage.clone())));
chain_index.initialize()?;

let subscriptions = Arc::new(SubscriptionManager::new());

let state_provider = ChainStateProvider::with_mempool(
    Arc::clone(&chain_index),
    state_provider_config,
    Arc::new(codes),
    mempool.clone(),
);

let handle = start_server_with_subscriptions(
    server_config,
    state_provider,
    subscriptions,
).await?;
```

### 6. gRPC Server (evolve_evnode)

```rust
use evolve_evnode::{EvnodeServer, EvnodeServerConfig, ExecutorServiceConfig};

let config = EvnodeServerConfig {
    addr: "127.0.0.1:50051".parse()?,
    enable_gzip: true,
    max_message_size: 4 * 1024 * 1024,
    executor_config: ExecutorServiceConfig {
        max_gas: 30_000_000,
        max_bytes: 128 * 1024,
    },
};

let server = EvnodeServer::with_mempool(config, stf, storage, codes, mempool);
server.serve().await?;
```

## Instructions

When creating a new node binary:

1. Create crate in `bin/my-node/` with Cargo.toml and src/main.rs

2. Add to workspace members in root Cargo.toml

3. Required dependencies - see bin/evd/Cargo.toml for complete list

4. Implement main.rs with:
   - CLI using clap with subcommands (run, init)
   - Runtime setup with commonware-runtime
   - Genesis handling (load or create)
   - Component initialization in order:
     a. Storage (QmdbStorage)
     b. Account codes
     c. Load/run genesis
     d. Build STF from genesis result
     e. Create mempool
     f. Start JSON-RPC server (optional)
     g. Start gRPC server
   - Graceful shutdown with Ctrl-C handling

5. Add justfile commands for build and run

## Genesis Flow

```rust
use evolve_server::{load_chain_state, save_chain_state, ChainState, CHAIN_STATE_KEY};
use evolve_node::GenesisOutput;

// Check for existing state
match load_chain_state::<GenesisAccounts, _>(&storage) {
    Some(state) => {
        // Resume from existing state
        (state.genesis_result, state.height)
    }
    None => {
        // Run genesis
        let genesis_stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
        let genesis_block = BlockContext::new(0, 0);

        let (accounts, state) = genesis_stf
            .system_exec(&storage, &codes, genesis_block, |env| {
                do_genesis_inner(env)
            })?;

        let changes = state.into_changes()?;

        // Commit to storage
        let operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();
        storage.batch(operations).await?;
        storage.commit().await?;

        (accounts, 1)
    }
}
```

## Runtime Pattern

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

        tokio::select! {
            result = server.serve() => { /* handle */ }
            _ = tokio::signal::ctrl_c() => {
                context_for_shutdown
                    .stop(0, Some(Duration::from_secs(10)))
                    .await?;
            }
        }
    }
});
```

## Transaction Formats

| Format | Type ID | Crate | Use Case |
|--------|---------|-------|----------|
| ETH | 0x02 | evolve_tx_eth | Standard Ethereum RLP |
| Micro | 0x83 | evolve_tx_micro | High throughput (150 bytes) |

## Checklist

When creating a node, verify:

- Crate created in bin directory
- Added to workspace members
- CLI with run and init subcommands
- QMDB storage configured
- Account codes installed
- Genesis handling (load or create)
- STF built from genesis result
- Mempool created (SharedMempool)
- JSON-RPC server (optional)
- gRPC server for external consensus
- Graceful shutdown handling
- State saved on shutdown
- Justfile commands added
- Clippy passes with no warnings
