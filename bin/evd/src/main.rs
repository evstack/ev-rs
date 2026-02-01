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
//! # Start with default settings
//! evd run
//!
//! # Custom addresses
//! evd run --grpc-addr 0.0.0.0:50051 --rpc-addr 0.0.0.0:8545
//!
//! # Initialize genesis only
//! evd init
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use clap::{Parser, Subcommand};
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex};
use evolve_core::ReadonlyKV;
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_evnode::{EvnodeServer, EvnodeServerConfig, ExecutorServiceConfig, StateChangeCallback};
use evolve_mempool::{new_shared_mempool, Mempool, SharedMempool};
use evolve_node::GenesisOutput;
use evolve_rpc_types::SyncStatus;
use evolve_server::{load_chain_state, save_chain_state, ChainState, CHAIN_STATE_KEY};
use evolve_stf_traits::{AccountsCodeStorage, StateChange};
use evolve_storage::{Operation, QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_genesis_inner, install_account_codes, GenesisAccounts,
};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_tx_eth::TxContext;
use tracing_subscriber::{fmt, EnvFilter};

/// Default gRPC server address.
const DEFAULT_GRPC_ADDR: &str = "127.0.0.1:50051";

/// Default JSON-RPC server address.
const DEFAULT_RPC_ADDR: &str = "127.0.0.1:8545";

/// Default data directory.
const DEFAULT_DATA_DIR: &str = "./data";

/// Default chain ID.
const DEFAULT_CHAIN_ID: u64 = 1;

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
    Run {
        /// Data directory for persistent storage
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,

        /// gRPC server bind address (for external consensus)
        #[arg(long, default_value = DEFAULT_GRPC_ADDR)]
        grpc_addr: String,

        /// JSON-RPC server bind address (for queries)
        #[arg(long, default_value = DEFAULT_RPC_ADDR)]
        rpc_addr: String,

        /// Chain ID
        #[arg(long, default_value_t = DEFAULT_CHAIN_ID)]
        chain_id: u64,

        /// Maximum gas per block
        #[arg(long, default_value_t = 30_000_000)]
        max_gas: u64,

        /// Maximum transaction size in bytes
        #[arg(long, default_value_t = 128 * 1024)]
        max_tx_bytes: u64,

        /// Disable gzip compression for gRPC
        #[arg(long)]
        disable_gzip: bool,

        /// Disable JSON-RPC server
        #[arg(long)]
        disable_rpc: bool,

        /// Log level (trace, debug, info, warn, error)
        #[arg(long, default_value = "info")]
        log_level: String,
    },
    /// Initialize genesis state without running
    Init {
        /// Data directory for persistent storage
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,

        /// Log level
        #[arg(long, default_value = "info")]
        log_level: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            data_dir,
            grpc_addr,
            rpc_addr,
            chain_id,
            max_gas,
            max_tx_bytes,
            disable_gzip,
            disable_rpc,
            log_level,
        } => {
            init_tracing(&log_level);

            let grpc: SocketAddr = grpc_addr.parse().expect("invalid gRPC address");
            let rpc: SocketAddr = rpc_addr.parse().expect("invalid RPC address");

            run_node(RunConfig {
                data_dir,
                grpc_addr: grpc,
                rpc_addr: rpc,
                chain_id,
                max_gas,
                max_tx_bytes,
                enable_gzip: !disable_gzip,
                enable_rpc: !disable_rpc,
            });
        }
        Commands::Init {
            data_dir,
            log_level,
        } => {
            init_tracing(&log_level);
            init_genesis(&data_dir);
        }
    }
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    fmt().with_env_filter(filter).init();
}

struct RunConfig {
    data_dir: String,
    grpc_addr: SocketAddr,
    rpc_addr: SocketAddr,
    chain_id: u64,
    max_gas: u64,
    max_tx_bytes: u64,
    enable_gzip: bool,
    enable_rpc: bool,
}

fn run_node(config: RunConfig) {
    tracing::info!("=== Evolve Node Daemon (evd) ===");

    std::fs::create_dir_all(&config.data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: config.data_dir.clone().into(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(&config.data_dir)
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
                match load_chain_state::<GenesisAccounts, _>(&storage) {
                    Some(state) => {
                        tracing::info!("Resuming from existing state at height {}", state.height);
                        (state.genesis_result, state.height)
                    }
                    None => {
                        tracing::info!("No existing state found, running genesis...");
                        let output = run_genesis(&storage, &codes);
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

            // Set up JSON-RPC server if enabled
            let rpc_handle = if config.enable_rpc {
                let storage_for_index = storage.clone();
                let chain_index = Arc::new(PersistentChainIndex::new(Arc::new(storage_for_index)));

                if let Err(e) = chain_index.initialize() {
                    tracing::warn!("Failed to initialize chain index: {:?}", e);
                }

                let subscriptions = Arc::new(SubscriptionManager::new());
                let codes_for_rpc = Arc::new(build_codes());

                let state_provider_config = ChainStateProviderConfig {
                    chain_id: config.chain_id,
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

                let server_config = RpcServerConfig {
                    http_addr: config.rpc_addr,
                    chain_id: config.chain_id,
                    client_version: "evd/0.1.0".to_string(),
                };

                tracing::info!("Starting JSON-RPC server on {}", config.rpc_addr);
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

            // Storage wrapper for state changes
            let storage_for_callback = storage.clone();
            let state_change_callback: StateChangeCallback = Arc::new(move |changes| {
                // Note: In production, you'd batch these and commit periodically
                tracing::debug!("Received {} state changes", changes.len());
                // Changes are handled by the storage layer directly
                let _ = &storage_for_callback; // Keep reference alive
            });

            // Configure gRPC server
            let grpc_config = EvnodeServerConfig {
                addr: config.grpc_addr,
                enable_gzip: config.enable_gzip,
                max_message_size: 4 * 1024 * 1024,
                executor_config: ExecutorServiceConfig {
                    max_gas: config.max_gas,
                    max_bytes: config.max_tx_bytes,
                },
            };

            tracing::info!("Starting gRPC server on {}", config.grpc_addr);
            tracing::info!("Configuration:");
            tracing::info!("  - Chain ID: {}", config.chain_id);
            tracing::info!("  - Max gas per block: {}", config.max_gas);
            tracing::info!("  - Max bytes per tx: {}", config.max_tx_bytes);
            tracing::info!("  - gRPC compression: {}", config.enable_gzip);
            tracing::info!("  - JSON-RPC: {}", config.enable_rpc);
            tracing::info!("  - Initial height: {}", initial_height);

            // Create gRPC server with mempool
            let server = EvnodeServer::with_mempool(
                grpc_config,
                stf,
                storage.clone(),
                build_codes(),
                mempool,
            )
            .with_state_change_callback(state_change_callback);

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
                        .stop(0, Some(Duration::from_secs(10)))
                        .await
                        .expect("shutdown failed");
                }
            }

            // Save chain state
            let chain_state = ChainState {
                height: initial_height, // Would be updated by consensus
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

fn init_genesis(data_dir: &str) {
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

        if load_chain_state::<GenesisAccounts, _>(&storage).is_some() {
            tracing::error!("State already initialized; refusing to re-run genesis");
            return;
        }

        let codes = build_codes();
        let output = run_genesis(&storage, &codes);

        commit_genesis(&storage, output.changes, &output.genesis_result)
            .await
            .expect("genesis commit failed");

        tracing::info!("Genesis complete!");
        tracing::info!("  - Alice: {:?}", output.genesis_result.alice);
        tracing::info!("  - Bob: {:?}", output.genesis_result.bob);
        tracing::info!("  - Scheduler: {:?}", output.genesis_result.scheduler);
    });
}

fn build_codes() -> AccountStorageMock {
    let mut codes = AccountStorageMock::default();
    install_account_codes(&mut codes);
    codes
}

fn run_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
) -> GenesisOutput<GenesisAccounts> {
    use evolve_core::BlockContext;

    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, evolve_testapp::PLACEHOLDER_ACCOUNT);

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| do_genesis_inner(env))
        .expect("genesis failed");

    let changes = state.into_changes().expect("failed to get state changes");

    GenesisOutput {
        genesis_result: accounts,
        changes,
    }
}

async fn commit_genesis<S: Storage>(
    storage: &S,
    changes: Vec<StateChange>,
    genesis_result: &GenesisAccounts,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();

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
