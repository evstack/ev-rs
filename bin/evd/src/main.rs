//! Evolve Node Daemon (evd)
//!
//! A full node implementation that exposes the Evolve execution layer via gRPC
//! for external consensus integration, with JSON-RPC for queries and mempool
//! for transaction collection.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────┐     gRPC      ┌──────────────────────────────────┐
//! │  External       │◄─────────────►│              evd                 │
//! │  Consensus      │               │                                  │
//! │  (ev-node)      │               │  ┌────────┐  ┌────────┐          │
//! └─────────────────┘               │  │  STF   │  │Mempool │          │
//!                                   │  └────────┘  └────────┘          │
//!        ┌──────────┐  JSON-RPC     │  ┌────────┐  ┌────────┐          │
//!        │  Client  │◄─────────────►│  │  RPC   │  │  QMDB  │          │
//!        └──────────┘               │  │ Server │  │Storage │          │
//!                                   │  └────────┘  └────────┘          │
//!                                   └──────────────────────────────────┘
//! ```
//!
//! ## Transaction Formats
//!
//! - **ETH**: Standard Ethereum RLP-encoded transactions (type 0x02)
//! - **Micro**: Minimal fixed-layout transactions (type 0x83) for high throughput
//!
//! ## Usage
//!
//! ```bash
//! # Start with default testapp genesis
//! evd run
//!
//! # Start with a custom genesis file (e.g. for x402 demo)
//! evd run --genesis-file examples/x402-demo/genesis.json
//!
//! # Custom addresses
//! evd run --grpc-addr 0.0.0.0:50051 --rpc-addr 0.0.0.0:8545
//!
//! # Initialize genesis only
//! evd init
//! ```
//!
//! ## Custom Genesis JSON Format
//!
//! When `--genesis-file` is provided, accounts are pre-registered as ETH EOAs
//! with deterministic IDs derived from their addresses.
//!
//! ```json
//! {
//!   "token": {
//!     "name": "evolve",
//!     "symbol": "ev",
//!     "decimals": 6,
//!     "icon_url": "https://lol.wtf",
//!     "description": "The evolve coin"
//!   },
//!   "minter_id": 100002,
//!   "accounts": [
//!     { "eth_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "balance": 1000000000 }
//!   ]
//! }
//! ```

mod genesis_config;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256, U256};
use clap::{Args, Parser, Subcommand};
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};
use evolve_chain_index::{
    build_index_data, BlockMetadata, ChainIndex, ChainStateProvider, ChainStateProviderConfig,
    PersistentChainIndex,
};
use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
use evolve_core::{AccountId, Environment, Message, ReadonlyKV, SdkResult};
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_evnode::{EvnodeServer, EvnodeServerConfig, ExecutorServiceConfig, OnBlockExecuted};
use evolve_mempool::{new_shared_mempool, Mempool, SharedMempool};
use evolve_node::{
    init_tracing as init_node_tracing, resolve_node_config, resolve_node_config_init,
    GenesisOutput, InitArgs, NodeConfig, RunArgs,
};
use evolve_rpc_types::SyncStatus;
use evolve_scheduler::scheduler_account::SchedulerRef;
use evolve_server::{
    load_chain_state, save_chain_state, BlockBuilder, ChainState, CHAIN_STATE_KEY,
};
use evolve_stf_traits::{AccountsCodeStorage, StateChange};
use evolve_storage::{Operation, QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_genesis_inner, install_account_codes,
    PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::TokenRef;
use evolve_tx_eth::address_to_account_id;
use evolve_tx_eth::TxContext;

use genesis_config::{EvdGenesisConfig, EvdGenesisResult};

#[derive(Parser)]
#[command(name = "evd")]
#[command(about = "Evolve node daemon with gRPC execution layer")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the node with gRPC and JSON-RPC servers
    Run(EvdRunArgs),
    /// Initialize genesis state without running
    Init(EvdInitArgs),
}

type EvdRunArgs = RunArgs<EvdRunCustom>;
type EvdInitArgs = InitArgs<EvdInitCustom>;

#[derive(Args)]
struct EvdRunCustom {
    /// Path to a genesis JSON file with ETH accounts (uses default testapp genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

#[derive(Args)]
struct EvdInitCustom {
    /// Path to a genesis JSON file with ETH accounts (uses default testapp genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            let config = resolve_node_config(&args.common, &args.native);
            init_node_tracing(&config.observability.log_level);
            let genesis_config = load_genesis_config(args.custom.genesis_file.as_deref());
            run_node(config, genesis_config);
        }
        Commands::Init(args) => {
            let config = resolve_node_config_init(&args.common);
            init_node_tracing(&config.observability.log_level);
            let genesis_config = load_genesis_config(args.custom.genesis_file.as_deref());
            init_genesis(&config.storage.path, genesis_config);
        }
    }
}

fn load_genesis_config(path: Option<&str>) -> Option<EvdGenesisConfig> {
    path.map(|p| {
        tracing::info!("Loading genesis config from: {}", p);
        EvdGenesisConfig::load(p).expect("failed to load genesis config")
    })
}

fn run_node(config: NodeConfig, genesis_config: Option<EvdGenesisConfig>) {
    tracing::info!("=== Evolve Node Daemon (evd) ===");

    std::fs::create_dir_all(&config.storage.path).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: config.storage.path.clone().into(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(&config.storage.path)
        .with_worker_threads(4);

    let runner = Runner::new(runtime_config);

    runner.start(move |context| {
        async move {
            let context_for_shutdown = context.clone();

            // Initialize QMDB storage
            let storage = QmdbStorage::new(context, storage_config)
                .await
                .expect("failed to create storage");

            // Set up account codes
            let codes = build_codes();
            tracing::info!("Installed account codes: {:?}", codes.list_identifiers());

            // Load or run genesis
            let (genesis_result, initial_height) =
                match load_chain_state::<EvdGenesisResult, _>(&storage) {
                    Some(state) => {
                        tracing::info!("Resuming from existing state at height {}", state.height);
                        (state.genesis_result, state.height)
                    }
                    None => {
                        tracing::info!("No existing state found, running genesis...");
                        let output = run_genesis(&storage, &codes, genesis_config.as_ref()).await;
                        commit_genesis(&storage, output.changes, &output.genesis_result)
                            .await
                            .expect("genesis commit failed");
                        tracing::info!("Genesis complete: {:?}", output.genesis_result);
                        (output.genesis_result, 1)
                    }
                };

            // Build STF with scheduler from genesis
            let gas_config = default_gas_config();
            let stf = build_mempool_stf(gas_config, genesis_result.scheduler);

            // Create shared mempool
            let mempool: SharedMempool<Mempool<TxContext>> = new_shared_mempool();
            // Create chain index (shared between RPC and block callback)
            let chain_index = Arc::new(PersistentChainIndex::new(Arc::new(storage.clone())));
            if let Err(e) = chain_index.initialize() {
                tracing::warn!("Failed to initialize chain index: {:?}", e);
            }

            // Set up JSON-RPC server if enabled
            let rpc_handle = if config.rpc.enabled {
                let subscriptions = Arc::new(SubscriptionManager::new());
                let codes_for_rpc = Arc::new(build_codes());

                let state_provider_config = ChainStateProviderConfig {
                    chain_id: config.chain.chain_id,
                    protocol_version: "0x1".to_string(),
                    gas_price: U256::ZERO,
                    sync_status: SyncStatus::NotSyncing(false),
                };

                let state_provider = ChainStateProvider::with_mempool(
                    Arc::clone(&chain_index),
                    state_provider_config,
                    codes_for_rpc,
                    mempool.clone(),
                );

                let rpc_addr = config.parsed_rpc_addr();
                let server_config = RpcServerConfig {
                    http_addr: rpc_addr,
                    chain_id: config.chain.chain_id,
                };

                tracing::info!("Starting JSON-RPC server on {}", rpc_addr);
                let handle = start_server_with_subscriptions(
                    server_config,
                    state_provider,
                    Arc::clone(&subscriptions),
                )
                .await
                .expect("failed to start RPC server");

                Some(handle)
            } else {
                None
            };

            // Shared state for the block callback
            let parent_hash = Arc::new(std::sync::RwLock::new(B256::ZERO));
            let current_height = Arc::new(AtomicU64::new(initial_height));

            // Build the OnBlockExecuted callback: commits state to storage + indexes blocks
            let storage_for_callback = storage.clone();
            let chain_index_for_callback = Arc::clone(&chain_index);
            let parent_hash_for_callback = Arc::clone(&parent_hash);
            let current_height_for_callback = Arc::clone(&current_height);
            let callback_chain_id = config.chain.chain_id;
            let executor_config = ExecutorServiceConfig::default();
            let callback_max_gas = executor_config.max_gas;
            let callback_indexing_enabled = config.rpc.enable_block_indexing;

            let on_block_executed: OnBlockExecuted = Arc::new(move |info| {
                // 1. Commit state changes to QmdbStorage
                let operations = state_changes_to_operations(info.state_changes);

                let commit_hash = futures::executor::block_on(async {
                    storage_for_callback
                        .batch(operations)
                        .await
                        .expect("storage batch failed");
                    storage_for_callback
                        .commit()
                        .await
                        .expect("storage commit failed")
                });
                let state_root = B256::from_slice(commit_hash.as_bytes());

                // 2. Compute block hash and build metadata
                let prev_parent = *parent_hash_for_callback.read().unwrap();
                let block_hash = compute_block_hash(info.height, info.timestamp, prev_parent);

                let metadata = BlockMetadata::new(
                    block_hash,
                    prev_parent,
                    state_root,
                    info.timestamp,
                    callback_max_gas,
                    Address::ZERO,
                    callback_chain_id,
                );

                // 3. Reconstruct block and index it
                let block = BlockBuilder::<TxContext>::new()
                    .number(info.height)
                    .timestamp(info.timestamp)
                    .transactions(info.transactions)
                    .build();

                let (stored_block, stored_txs, stored_receipts) =
                    build_index_data(&block, &info.block_result, &metadata);

                if callback_indexing_enabled {
                    if let Err(e) = chain_index_for_callback.store_block(
                        stored_block,
                        stored_txs,
                        stored_receipts,
                    ) {
                        tracing::warn!("Failed to index block {}: {:?}", info.height, e);
                    } else {
                        tracing::debug!(
                            "Indexed block {} (hash={}, state_root={})",
                            info.height,
                            block_hash,
                            state_root
                        );
                    }
                }

                // 4. Update parent hash and height for next block
                *parent_hash_for_callback.write().unwrap() = block_hash;
                current_height_for_callback.store(info.height, Ordering::SeqCst);
            });

            // Configure gRPC server
            let grpc_config = EvnodeServerConfig {
                addr: config.parsed_grpc_addr(),
                enable_gzip: config.grpc.enable_gzip,
                max_message_size: config.grpc_max_message_size_usize(),
                executor_config,
            };

            let grpc_addr = config.parsed_grpc_addr();
            tracing::info!("Starting gRPC server on {}", grpc_addr);
            tracing::info!("Configuration:");
            tracing::info!("  - Chain ID: {}", config.chain.chain_id);
            tracing::info!("  - gRPC compression: {}", config.grpc.enable_gzip);
            tracing::info!("  - JSON-RPC: {}", config.rpc.enabled);
            tracing::info!("  - Block indexing: {}", config.rpc.enable_block_indexing);
            tracing::info!("  - Initial height: {}", initial_height);

            // Create gRPC server with mempool and block callback
            let server = EvnodeServer::with_mempool(
                grpc_config,
                stf,
                storage.clone(),
                build_codes(),
                mempool,
            )
            .with_on_block_executed(on_block_executed);

            tracing::info!("Server ready. Press Ctrl+C to stop.");

            // Run gRPC server with shutdown handling
            tokio::select! {
                result = server.serve() => {
                    if let Err(e) = result {
                        tracing::error!("gRPC server error: {}", e);
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received Ctrl+C, shutting down...");
                    context_for_shutdown
                        .stop(0, Some(Duration::from_secs(config.operations.shutdown_timeout_secs)))
                        .await
                        .expect("shutdown failed");
                }
            }

            // Save chain state with actual committed height
            let chain_state = ChainState {
                height: current_height.load(Ordering::SeqCst),
                genesis_result,
            };
            if let Err(e) = save_chain_state(&storage, &chain_state).await {
                tracing::error!("Failed to save chain state: {}", e);
            }

            // Stop RPC server
            if let Some(handle) = rpc_handle {
                tracing::info!("Stopping JSON-RPC server...");
                handle.stop().expect("failed to stop RPC server");
            }

            tracing::info!("Shutdown complete.");
        }
    });
}

fn init_genesis(data_dir: &str, genesis_config: Option<EvdGenesisConfig>) {
    tracing::info!("=== Evolve Node Daemon - Genesis Init ===");

    std::fs::create_dir_all(data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.into(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(data_dir)
        .with_worker_threads(1);

    let runner = Runner::new(runtime_config);

    runner.start(move |context| async move {
        let storage = QmdbStorage::new(context, storage_config)
            .await
            .expect("failed to create storage");

        if load_chain_state::<EvdGenesisResult, _>(&storage).is_some() {
            tracing::error!("State already initialized; refusing to re-run genesis");
            return;
        }

        let codes = build_codes();
        let output = run_genesis(&storage, &codes, genesis_config.as_ref()).await;

        commit_genesis(&storage, output.changes, &output.genesis_result)
            .await
            .expect("genesis commit failed");

        tracing::info!("Genesis complete!");
        tracing::info!("  Token: {:?}", output.genesis_result.token);
        tracing::info!("  Scheduler: {:?}", output.genesis_result.scheduler);
    });
}

fn build_codes() -> AccountStorageMock {
    let mut codes = AccountStorageMock::default();
    install_account_codes(&mut codes);
    codes
}

/// Pre-register an EOA account in storage so genesis can reference it.
fn build_eoa_registration(account_id: AccountId, eth_address: [u8; 20]) -> Vec<Operation> {
    let mut ops = Vec::with_capacity(3);

    // 1. Register account code identifier
    let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
    key.extend_from_slice(&account_id.as_bytes());
    let value = Message::new(&"EthEoaAccount".to_string())
        .unwrap()
        .into_bytes()
        .unwrap();
    ops.push(Operation::Set { key, value });

    // 2. Set nonce = 0 (Item prefix 0)
    let mut nonce_key = account_id.as_bytes().to_vec();
    nonce_key.push(0u8);
    let nonce_value = Message::new(&0u64).unwrap().into_bytes().unwrap();
    ops.push(Operation::Set {
        key: nonce_key,
        value: nonce_value,
    });

    // 3. Set eth_address (Item prefix 1)
    let mut addr_key = account_id.as_bytes().to_vec();
    addr_key.push(1u8);
    let addr_value = Message::new(&eth_address).unwrap().into_bytes().unwrap();
    ops.push(Operation::Set {
        key: addr_key,
        value: addr_value,
    });

    ops
}

/// Run genesis using the default testapp genesis or a custom genesis config.
async fn run_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
    genesis_config: Option<&EvdGenesisConfig>,
) -> GenesisOutput<EvdGenesisResult> {
    match genesis_config {
        Some(config) => run_custom_genesis(storage, codes, config).await,
        None => run_default_genesis(storage, codes),
    }
}

/// Default genesis using testapp's `do_genesis_inner` (sequential account IDs).
fn run_default_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
) -> GenesisOutput<EvdGenesisResult> {
    use evolve_core::BlockContext;

    tracing::info!("Running default testapp genesis...");

    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| do_genesis_inner(env))
        .expect("genesis failed");

    let changes = state.into_changes().expect("failed to get state changes");

    let genesis_result = EvdGenesisResult {
        token: accounts.atom,
        scheduler: accounts.scheduler,
    };

    GenesisOutput {
        genesis_result,
        changes,
    }
}

/// Custom genesis with ETH EOA accounts from a genesis JSON file.
async fn run_custom_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
    genesis_config: &EvdGenesisConfig,
) -> GenesisOutput<EvdGenesisResult> {
    use evolve_core::BlockContext;

    // Parse accounts that have a non-zero balance (need pre-registration for genesis funding).
    // Other accounts are auto-registered by the STF on their first transaction.
    let funded_accounts: Vec<(Address, u128)> = genesis_config
        .accounts
        .iter()
        .filter(|acc| acc.balance > 0)
        .map(|acc| {
            let addr = acc
                .parse_address()
                .expect("invalid address in genesis config");
            (addr, acc.balance)
        })
        .collect();

    // Pre-register only funded EOA accounts in storage
    let mut pre_ops = Vec::new();
    for (addr, _) in &funded_accounts {
        let id = address_to_account_id(*addr);
        let addr_bytes: [u8; 20] = addr.into_array();
        pre_ops.extend(build_eoa_registration(id, addr_bytes));
    }

    storage
        .batch(pre_ops)
        .await
        .expect("pre-register EOAs failed");
    storage.commit().await.expect("pre-register commit failed");

    tracing::info!(
        "Pre-registered {} funded EOA accounts:",
        funded_accounts.len()
    );
    for (i, (addr, balance)) in funded_accounts.iter().enumerate() {
        let id = address_to_account_id(*addr);
        tracing::info!("  #{:02}: {:?} (0x{:x}) balance={}", i, id, addr, balance);
    }

    // Build balances list for genesis token initialization
    let balances: Vec<(AccountId, u128)> = funded_accounts
        .iter()
        .map(|(addr, balance)| (address_to_account_id(*addr), *balance))
        .collect();

    let minter = AccountId::new(genesis_config.minter_id);
    let metadata = genesis_config.token.to_metadata();

    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
    let genesis_block = BlockContext::new(0, 0);

    let (genesis_result, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            do_custom_genesis(metadata.clone(), balances.clone(), minter, env)
        })
        .expect("genesis failed");

    let changes = state.into_changes().expect("failed to get state changes");

    GenesisOutput {
        genesis_result,
        changes,
    }
}

fn do_custom_genesis(
    metadata: evolve_fungible_asset::FungibleAssetMetadata,
    balances: Vec<(AccountId, u128)>,
    minter: AccountId,
    env: &mut dyn Environment,
) -> SdkResult<EvdGenesisResult> {
    let token = TokenRef::initialize(metadata, balances, Some(minter), env)?.0;

    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
    scheduler_acc.update_begin_blockers(vec![], env)?;

    Ok(EvdGenesisResult {
        token: token.0,
        scheduler: scheduler_acc.0,
    })
}

fn compute_block_hash(height: u64, timestamp: u64, parent_hash: B256) -> B256 {
    let mut data = Vec::with_capacity(48);
    data.extend_from_slice(&height.to_le_bytes());
    data.extend_from_slice(&timestamp.to_le_bytes());
    data.extend_from_slice(parent_hash.as_slice());
    keccak256(&data)
}

async fn commit_genesis<S: Storage>(
    storage: &S,
    changes: Vec<StateChange>,
    genesis_result: &EvdGenesisResult,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut operations = state_changes_to_operations(changes);

    let chain_state = ChainState {
        height: 1,
        genesis_result: *genesis_result,
    };
    operations.push(Operation::Set {
        key: CHAIN_STATE_KEY.to_vec(),
        value: borsh::to_vec(&chain_state).map_err(|e| format!("serialize: {e}"))?,
    });

    storage
        .batch(operations)
        .await
        .map_err(|e| format!("batch failed: {:?}", e))?;

    storage
        .commit()
        .await
        .map_err(|e| format!("commit failed: {:?}", e))?;

    Ok(())
}

fn state_changes_to_operations(changes: Vec<StateChange>) -> Vec<Operation> {
    changes
        .into_iter()
        .map(|change| match change {
            StateChange::Set { key, value } => Operation::Set { key, value },
            StateChange::Remove { key } => Operation::Remove { key },
        })
        .collect()
}
