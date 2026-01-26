//! Reusable dev-node runner for Evolve applications.
//!
//! This module provides a complete dev-node implementation that includes:
//! - Block production with configurable intervals
//! - JSON-RPC server for Ethereum-compatible queries
//! - Chain indexing for block/transaction/receipt queries
//! - Persistent storage across restarts

use std::fmt::Debug;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use borsh::{BorshDeserialize, BorshSerialize};
use commonware_runtime::tokio::{Config as TokioConfig, Context as TokioContext, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};
use evolve_chain_index::{ChainStateProvider, ChainStateProviderConfig, PersistentChainIndex};
use evolve_core::ReadonlyKV;
use evolve_eth_jsonrpc::{RpcServerConfig, SubscriptionManager};
use evolve_rpc_types::SyncStatus;
use evolve_server::StfExecutor;
use evolve_server::{
    load_chain_state, save_chain_state, ChainState, DevConfig, DevConsensus, CHAIN_STATE_KEY,
};
use evolve_stf_traits::{AccountsCodeStorage, StateChange, Transaction};
use evolve_storage::{Operation, Storage, StorageConfig};
use std::future::Future;

/// Default data directory for persistent storage.
pub const DEFAULT_DATA_DIR: &str = "./data";

/// Default RPC server address.
pub const DEFAULT_RPC_ADDR: &str = "127.0.0.1:8545";

/// Configuration for the dev node RPC server.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// Address to bind the HTTP server to.
    pub http_addr: SocketAddr,
    /// Chain ID for eth_chainId.
    pub chain_id: u64,
    /// Client version string.
    pub client_version: String,
    /// Whether RPC is enabled.
    pub enabled: bool,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            http_addr: DEFAULT_RPC_ADDR.parse().unwrap(),
            chain_id: 1,
            client_version: "evolve-dev/0.1.0".to_string(),
            enabled: true,
        }
    }
}

impl RpcConfig {
    /// Create a disabled RPC config.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Set the HTTP address.
    pub fn with_addr(mut self, addr: SocketAddr) -> Self {
        self.http_addr = addr;
        self
    }

    /// Set the chain ID.
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }
}

/// Result of a genesis run, including the state changes to commit.
pub struct GenesisOutput<G> {
    /// Application-specific genesis result.
    pub genesis_result: G,
    /// State changes produced by genesis.
    pub changes: Vec<StateChange>,
}

type RuntimeContext = TokioContext;

/// Run the dev node with default settings (RPC enabled).
pub fn run_dev_node<
    Stf,
    Codes,
    Tx,
    G,
    S,
    BuildGenesisStf,
    BuildStf,
    BuildCodes,
    RunGenesis,
    BuildStorage,
    BuildStorageFut,
>(
    data_dir: impl AsRef<Path>,
    build_genesis_stf: BuildGenesisStf,
    build_stf: BuildStf,
    build_codes: BuildCodes,
    run_genesis: RunGenesis,
    build_storage: BuildStorage,
) where
    Tx: Transaction + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + 'static,
    G: BorshSerialize + BorshDeserialize + Clone + Debug + Send + Sync + 'static,
    BuildGenesisStf: Fn() -> Stf + Send + Sync + 'static,
    BuildStf: Fn(&G) -> Stf + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Clone + Send + Sync + 'static,
    RunGenesis: Fn(&Stf, &Codes, &S) -> Result<GenesisOutput<G>, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
    BuildStorage: Fn(RuntimeContext, StorageConfig) -> BuildStorageFut + Send + Sync + 'static,
    BuildStorageFut:
        Future<Output = Result<S, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    run_dev_node_with_rpc(
        data_dir,
        build_genesis_stf,
        build_stf,
        build_codes,
        run_genesis,
        build_storage,
        RpcConfig::default(),
    )
}

/// Run the dev node with custom RPC configuration.
pub fn run_dev_node_with_rpc<
    Stf,
    Codes,
    Tx,
    G,
    S,
    BuildGenesisStf,
    BuildStf,
    BuildCodes,
    RunGenesis,
    BuildStorage,
    BuildStorageFut,
>(
    data_dir: impl AsRef<Path>,
    build_genesis_stf: BuildGenesisStf,
    build_stf: BuildStf,
    build_codes: BuildCodes,
    run_genesis: RunGenesis,
    build_storage: BuildStorage,
    rpc_config: RpcConfig,
) where
    Tx: Transaction + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + 'static,
    G: BorshSerialize + BorshDeserialize + Clone + Debug + Send + Sync + 'static,
    BuildGenesisStf: Fn() -> Stf + Send + Sync + 'static,
    BuildStf: Fn(&G) -> Stf + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Clone + Send + Sync + 'static,
    RunGenesis: Fn(&Stf, &Codes, &S) -> Result<GenesisOutput<G>, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
    BuildStorage: Fn(RuntimeContext, StorageConfig) -> BuildStorageFut + Send + Sync + 'static,
    BuildStorageFut:
        Future<Output = Result<S, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    tracing::info!("=== Evolve Dev Node ===");
    let data_dir = data_dir.as_ref();
    std::fs::create_dir_all(data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.to_path_buf(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(data_dir)
        .with_worker_threads(4); // More threads for RPC handling

    let runner = Runner::new(runtime_config);

    let build_genesis_stf = Arc::new(build_genesis_stf);
    let build_stf = Arc::new(build_stf);
    let build_codes = Arc::new(build_codes);
    let run_genesis = Arc::new(run_genesis);
    let build_storage = Arc::new(build_storage);

    runner.start(move |context| {
        let build_genesis_stf = Arc::clone(&build_genesis_stf);
        let build_stf = Arc::clone(&build_stf);
        let build_codes = Arc::clone(&build_codes);
        let run_genesis = Arc::clone(&run_genesis);
        let build_storage = Arc::clone(&build_storage);
        let rpc_config = rpc_config.clone();

        async move {
            // Clone context early since build_storage takes ownership
            let context_for_shutdown = context.clone();
            let storage = (build_storage)(context, storage_config)
                .await
                .expect("failed to create storage");

            // Create account codes
            let codes = build_codes();

            let (genesis_result, initial_height) = match load_chain_state::<G, _>(&storage) {
                Some(state) => {
                    tracing::info!("Resuming from existing state at height {}", state.height);
                    tracing::info!("Genesis state: {:?}", state.genesis_result);
                    (state.genesis_result, state.height)
                }
                None => {
                    tracing::info!("No existing state found, running genesis...");
                    let bootstrap_stf = (build_genesis_stf)();
                    let output =
                        (run_genesis)(&bootstrap_stf, &codes, &storage).expect("genesis failed");

                    commit_genesis(&storage, output.changes, &output.genesis_result)
                        .await
                        .expect("genesis commit failed");

                    tracing::info!("Genesis complete. Result: {:?}", output.genesis_result);
                    (output.genesis_result, 1)
                }
            };

            // Build STF for normal execution
            let stf = (build_stf)(&genesis_result);

            // Create DevConsensus config
            let block_interval = Duration::from_secs(1);
            let dev_config = DevConfig {
                block_interval: Some(block_interval),
                initial_height,
                chain_id: rpc_config.chain_id,
                ..Default::default()
            };

            // Set up RPC infrastructure if enabled
            let rpc_handle = if rpc_config.enabled {
                // Create chain index for RPC queries
                // Clone storage since both DevConsensus and ChainIndex need access
                let storage_for_index = storage.clone();
                let chain_index = Arc::new(PersistentChainIndex::new(Arc::new(storage_for_index)));

                // Initialize from existing data
                if let Err(e) = chain_index.initialize() {
                    tracing::warn!("Failed to initialize chain index: {:?}", e);
                }

                // Create subscription manager for real-time events
                let subscriptions = Arc::new(SubscriptionManager::new());

                // Create state provider for RPC
                let codes_for_rpc = Arc::new(build_codes());
                let state_provider_config = ChainStateProviderConfig {
                    chain_id: rpc_config.chain_id,
                    protocol_version: "0x1".to_string(),
                    gas_price: U256::ZERO,
                    sync_status: SyncStatus::NotSyncing(false),
                };
                let state_provider = ChainStateProvider::with_account_codes(
                    Arc::clone(&chain_index),
                    state_provider_config,
                    codes_for_rpc,
                );

                // Start JSON-RPC server
                let server_config = RpcServerConfig {
                    http_addr: rpc_config.http_addr,
                    chain_id: rpc_config.chain_id,
                    client_version: rpc_config.client_version.clone(),
                };

                tracing::info!("Starting JSON-RPC server on {}", rpc_config.http_addr);
                let handle = evolve_eth_jsonrpc::start_server(server_config, state_provider)
                    .await
                    .expect("failed to start RPC server");

                // Create DevConsensus with RPC support
                let dev: Arc<DevConsensus<Stf, S, Codes, Tx, PersistentChainIndex<S>>> =
                    Arc::new(DevConsensus::with_rpc(
                        stf,
                        storage,
                        codes,
                        dev_config,
                        chain_index,
                        subscriptions,
                    ));

                tracing::info!(
                    "Block interval: {:?}, starting at height {}",
                    block_interval,
                    initial_height
                );

                tracing::info!("Starting block production... (Ctrl+C to stop)");

                // Run block production and Ctrl+C handling concurrently using Spawner pattern.
                // When Ctrl+C is received, stop() triggers shutdown signal via context.stopped().
                tokio::select! {
                    _ = dev.run_block_production(context_for_shutdown.clone()) => {
                        // Block production exited
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
                        context_for_shutdown
                            .stop(0, Some(Duration::from_secs(10)))
                            .await
                            .expect("shutdown failed");
                    }
                }

                // Save chain state
                let final_height = dev.height();
                tracing::info!("Stopped at height: {}", final_height);

                let chain_state = ChainState {
                    height: final_height,
                    genesis_result,
                };
                if let Err(e) = save_chain_state(dev.storage(), &chain_state).await {
                    tracing::error!("Failed to save chain state: {}", e);
                } else {
                    tracing::info!("Saved chain state at height {}", final_height);
                }

                Some(handle)
            } else {
                // No RPC - use simple DevConsensus
                let dev: Arc<DevConsensus<Stf, S, Codes, Tx, evolve_server::NoopChainIndex>> =
                    Arc::new(DevConsensus::new(stf, storage, codes, dev_config));

                tracing::info!(
                    "Block interval: {:?}, starting at height {}",
                    block_interval,
                    initial_height
                );

                tracing::info!("Starting block production... (Ctrl+C to stop)");

                // Run block production and Ctrl+C handling concurrently using Spawner pattern
                tokio::select! {
                    _ = dev.run_block_production(context_for_shutdown.clone()) => {
                        // Block production exited
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
                        context_for_shutdown
                            .stop(0, Some(Duration::from_secs(10)))
                            .await
                            .expect("shutdown failed");
                    }
                }

                let final_height = dev.height();
                tracing::info!("Stopped at height: {}", final_height);

                let chain_state = ChainState {
                    height: final_height,
                    genesis_result,
                };
                if let Err(e) = save_chain_state(dev.storage(), &chain_state).await {
                    tracing::error!("Failed to save chain state: {}", e);
                } else {
                    tracing::info!("Saved chain state at height {}", final_height);
                }

                None
            };

            // Stop RPC server if running
            if let Some(handle) = rpc_handle {
                tracing::info!("Stopping RPC server...");
                handle.stop().expect("failed to stop RPC server");
                tracing::info!("RPC server stopped");
            }
        }
    });
}

/// Initialize genesis in the data directory and exit.
pub fn init_dev_node<
    Stf,
    Codes,
    Tx,
    G,
    S,
    BuildGenesisStf,
    BuildCodes,
    RunGenesis,
    BuildStorage,
    BuildStorageFut,
>(
    data_dir: impl AsRef<Path>,
    build_genesis_stf: BuildGenesisStf,
    build_codes: BuildCodes,
    run_genesis: RunGenesis,
    build_storage: BuildStorage,
) where
    Tx: Transaction + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + 'static,
    G: BorshSerialize + BorshDeserialize + Clone + Debug + Send + Sync + 'static,
    BuildGenesisStf: Fn() -> Stf + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Send + Sync + 'static,
    RunGenesis: Fn(&Stf, &Codes, &S) -> Result<GenesisOutput<G>, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
    BuildStorage: Fn(RuntimeContext, StorageConfig) -> BuildStorageFut + Send + Sync + 'static,
    BuildStorageFut:
        Future<Output = Result<S, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    tracing::info!("=== Evolve Dev Node (init) ===");
    let data_dir = data_dir.as_ref();
    std::fs::create_dir_all(data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.to_path_buf(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(data_dir)
        .with_worker_threads(1);

    let runner = Runner::new(runtime_config);

    let build_genesis_stf = Arc::new(build_genesis_stf);
    let build_codes = Arc::new(build_codes);
    let run_genesis = Arc::new(run_genesis);
    let build_storage = Arc::new(build_storage);

    runner.start(move |context| {
        let build_genesis_stf = Arc::clone(&build_genesis_stf);
        let build_codes = Arc::clone(&build_codes);
        let run_genesis = Arc::clone(&run_genesis);
        let build_storage = Arc::clone(&build_storage);

        async move {
            let storage = (build_storage)(context, storage_config)
                .await
                .expect("failed to create storage");

            if load_chain_state::<G, _>(&storage).is_some() {
                tracing::error!("State already initialized; refusing to re-run genesis");
                return;
            }

            let codes = build_codes();
            let bootstrap_stf = (build_genesis_stf)();
            let output = (run_genesis)(&bootstrap_stf, &codes, &storage).expect("genesis failed");

            commit_genesis(&storage, output.changes, &output.genesis_result)
                .await
                .expect("genesis commit failed");

            tracing::info!("Genesis complete. Result: {:?}", output.genesis_result);
        }
    });
}

async fn commit_genesis<G: BorshSerialize + Clone, S: Storage>(
    storage: &S,
    changes: Vec<StateChange>,
    genesis_result: &G,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();

    let chain_state: ChainState<G> = ChainState {
        height: 1,
        genesis_result: genesis_result.clone(),
    };
    operations.push(Operation::Set {
        key: CHAIN_STATE_KEY.to_vec(),
        value: borsh::to_vec(&chain_state).map_err(|e| format!("serialize chain state: {e}"))?,
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
