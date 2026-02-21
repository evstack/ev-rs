//! Reusable dev-node runner for Evolve applications.
//!
//! This module provides a complete dev-node implementation that includes:
//! - Block production with configurable intervals
//! - JSON-RPC server for Ethereum-compatible queries
//! - Chain indexing for block/transaction/receipt queries
//! - Persistent storage across restarts

pub mod cli;
pub mod config;

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
use evolve_core::encoding::Encodable;
use evolve_core::ReadonlyKV;
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_mempool::{new_shared_mempool, Mempool, MempoolTx, SharedMempool};
use evolve_rpc_types::SyncStatus;
use evolve_server::{
    load_chain_state, save_chain_state, ChainState, DevConfig, DevConsensus, CHAIN_STATE_KEY,
};
use evolve_server::{OnBlockArchive, StfExecutor};
use evolve_stf_traits::{AccountsCodeStorage, StateChange, Transaction};
use evolve_storage::types::BlockHash as ArchiveBlockHash;
use evolve_storage::{
    BlockStorage, BlockStorageConfig, MockStorage, Operation, Storage, StorageConfig,
};
use evolve_tx_eth::TxContext;
use std::future::Future;

pub use cli::*;
pub use config::*;

/// Default data directory for persistent storage.
pub const DEFAULT_DATA_DIR: &str = "./data";

/// Default RPC server address.
pub const DEFAULT_RPC_ADDR: &str = "127.0.0.1:8545";

fn parse_env_u64(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn parse_env_usize(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn configured_block_interval() -> Duration {
    Duration::from_millis(parse_env_u64("EVOLVE_BLOCK_INTERVAL_MS", 1_000))
}

fn configured_max_txs_per_block() -> usize {
    parse_env_usize("EVOLVE_MAX_TXS_PER_BLOCK", 1_000)
}

/// Convenience handles for a dev node wired with a mempool.
pub struct DevNodeMempoolHandles<Stf, S, Codes, Tx: MempoolTx> {
    /// Dev consensus engine wired to mempool transactions.
    pub dev: Arc<DevConsensus<Stf, S, Codes, Tx, evolve_server::NoopChainIndex>>,
    /// Shared mempool instance.
    pub mempool: SharedMempool<Mempool<Tx>>,
}

/// Build a dev consensus + mempool pair for testing and tools.
///
/// Generic over transaction type `Tx`. For ETH transactions, use `TxContext`.
pub fn build_dev_node_with_mempool<Stf, Codes, S, Tx>(
    stf: Stf,
    storage: S,
    codes: Codes,
    config: DevConfig,
) -> DevNodeMempoolHandles<Stf, S, Codes, Tx>
where
    Tx: Transaction + MempoolTx + Encodable + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + 'static,
{
    let mempool: SharedMempool<Mempool<Tx>> = new_shared_mempool();
    let dev = Arc::new(DevConsensus::with_mempool(
        stf,
        storage,
        codes,
        config,
        mempool.clone(),
    ));

    DevNodeMempoolHandles { dev, mempool }
}

/// Configuration for the dev node RPC server.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// Address to bind the HTTP server to.
    pub http_addr: SocketAddr,
    /// Chain ID for eth_chainId.
    pub chain_id: u64,
    /// Whether RPC is enabled.
    pub enabled: bool,
    /// Whether block indexing is enabled while producing blocks.
    pub enable_block_indexing: bool,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            http_addr: DEFAULT_RPC_ADDR.parse().unwrap(),
            chain_id: 1,
            enabled: true,
            enable_block_indexing: true,
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

    /// Enable or disable block indexing while keeping RPC enabled.
    pub fn with_block_indexing(mut self, enabled: bool) -> Self {
        self.enable_block_indexing = enabled;
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

/// Build the block archive callback.
///
/// Creates a `BlockStorage` backed by the commonware archive and returns
/// an `OnBlockArchive` callback that writes each produced block into it.
///
/// # Panics
///
/// Panics if block storage initialization fails. Block archival is a required
/// subsystem — all produced blocks must be persisted.
async fn build_block_archive(context: TokioContext) -> OnBlockArchive {
    let config = BlockStorageConfig::default();
    let store = BlockStorage::new(context, config)
        .await
        .expect("failed to initialize block archive storage");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<(u64, ArchiveBlockHash, bytes::Bytes)>(64);

    // Single consumer task ensures blocks are written in order.
    tokio::spawn(async move {
        let mut store = store;
        while let Some((block_number, block_hash, block_bytes)) = rx.recv().await {
            if let Err(e) = store.put_sync(block_number, block_hash, block_bytes).await {
                tracing::warn!("Failed to archive block {}: {:?}", block_number, e);
            }
        }
    });

    tracing::info!("Block archive storage enabled");

    Arc::new(move |block_number, block_hash, block_bytes| {
        let hash_bytes = ArchiveBlockHash::new(block_hash.0);
        if let Err(e) = tx.try_send((block_number, hash_bytes, block_bytes)) {
            tracing::warn!(
                "Block archive channel full or closed for block {}: {}",
                block_number,
                e
            );
        }
    })
}

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
    Tx: Transaction + MempoolTx + Encodable + Send + Sync + 'static,
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
    Tx: Transaction + MempoolTx + Encodable + Send + Sync + 'static,
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
    let chain_index_db_path = data_dir.join("chain-index.sqlite");

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
        let chain_index_db_path = chain_index_db_path.clone();

        async move {
            // Clone context early since build_storage takes ownership
            let context_for_shutdown = context.clone();
            let context_for_archive = context.clone();
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
            let block_interval = configured_block_interval();
            let dev_config = DevConfig {
                block_interval: Some(block_interval),
                initial_height,
                chain_id: rpc_config.chain_id,
                ..Default::default()
            };

            // Build block archive callback (always on)
            let archive_cb = build_block_archive(context_for_archive).await;

            // Set up RPC infrastructure if enabled
            let rpc_handle = if rpc_config.enabled {
                // Create chain index backed by SQLite
                let chain_index = Arc::new(
                    PersistentChainIndex::new(&chain_index_db_path)
                        .expect("failed to open chain index database"),
                );

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
                };

                tracing::info!("Starting JSON-RPC server on {}", rpc_config.http_addr);
                let handle = start_server_with_subscriptions(
                    server_config,
                    state_provider,
                    Arc::clone(&subscriptions),
                )
                .await
                .expect("failed to start RPC server");

                // Create DevConsensus with RPC support
                let consensus = DevConsensus::with_rpc(
                    stf,
                    storage,
                    codes,
                    dev_config,
                    chain_index,
                    subscriptions,
                )
                .with_indexing_enabled(rpc_config.enable_block_indexing)
                .with_block_archive(archive_cb);
                let dev: Arc<DevConsensus<Stf, S, Codes, Tx, PersistentChainIndex>> =
                    Arc::new(consensus);

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
                let consensus = DevConsensus::new(stf, storage, codes, dev_config)
                    .with_block_archive(archive_cb);
                let dev: Arc<DevConsensus<Stf, S, Codes, Tx, evolve_server::NoopChainIndex>> =
                    Arc::new(consensus);

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

/// Run the dev node with RPC and mempool-enabled transaction ingestion.
///
/// Generic over transaction type `Tx`. For ETH transactions, use
/// `run_dev_node_with_rpc_and_mempool_eth` for convenience.
///
/// Note: When using a custom `Tx` type with RPC enabled, the RPC layer
/// will still use `TxContext` for `eth_sendRawTransaction`. For custom
/// transaction types, consider disabling RPC or providing a custom gateway.
pub fn run_dev_node_with_rpc_and_mempool<
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
    Tx: Transaction + MempoolTx + Encodable + Send + Sync + 'static,
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
    tracing::info!("=== Evolve Dev Node (mempool) ===");
    let data_dir = data_dir.as_ref();
    std::fs::create_dir_all(data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.to_path_buf(),
        ..Default::default()
    };

    let runtime_config = TokioConfig::default()
        .with_storage_directory(data_dir)
        .with_worker_threads(4);

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
            let context_for_shutdown = context.clone();
            let storage = (build_storage)(context, storage_config)
                .await
                .expect("failed to create storage");

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

            let stf = (build_stf)(&genesis_result);

            let block_interval = configured_block_interval();
            let max_txs_per_block = configured_max_txs_per_block();
            let dev_config = DevConfig {
                block_interval: Some(block_interval),
                initial_height,
                chain_id: rpc_config.chain_id,
                ..Default::default()
            };

            let mempool: SharedMempool<Mempool<Tx>> = new_shared_mempool();

            // Note: RPC with custom Tx types is not fully supported.
            // The RPC layer requires TxContext for eth_sendRawTransaction.
            // For custom Tx types, use run_dev_node_with_rpc_and_mempool_eth instead.
            if rpc_config.enabled {
                tracing::warn!(
                    "RPC enabled with generic Tx type. eth_sendRawTransaction will not work. \
                    Use run_dev_node_with_rpc_and_mempool_eth for ETH transactions with full RPC support."
                );
            }

            let dev: Arc<DevConsensus<Stf, S, Codes, Tx, evolve_server::NoopChainIndex>> =
                Arc::new(DevConsensus::with_mempool(stf, storage, codes, dev_config, mempool));

            tracing::info!(
                "Block interval: {:?}, max_txs_per_block: {}, starting at height {}",
                block_interval,
                max_txs_per_block,
                initial_height
            );

            tracing::info!("Starting block production... (Ctrl+C to stop)");

            tokio::select! {
                _ = dev.run_block_production_with_mempool(context_for_shutdown.clone(), max_txs_per_block) => {
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
        }
    });
}

/// Run the dev node with RPC and mempool for ETH transactions (TxContext).
///
/// This is a convenience wrapper around `run_dev_node_with_rpc_and_mempool`
/// that uses `TxContext` as the transaction type and sets up the full
/// ETH JSON-RPC server with `eth_sendRawTransaction` support.
pub fn run_dev_node_with_rpc_and_mempool_eth<
    Stf,
    Codes,
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
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<TxContext, S, Codes> + Send + Sync + 'static,
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
    tracing::info!("=== Evolve Dev Node (ETH mempool) ===");
    let data_dir = data_dir.as_ref();
    std::fs::create_dir_all(data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.to_path_buf(),
        ..Default::default()
    };
    let chain_index_db_path = data_dir.join("chain-index.sqlite");

    let runtime_config = TokioConfig::default()
        .with_storage_directory(data_dir)
        .with_worker_threads(4);

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
        let chain_index_db_path = chain_index_db_path.clone();

        async move {
            let context_for_shutdown = context.clone();
            let context_for_archive = context.clone();
            let storage = (build_storage)(context, storage_config)
                .await
                .expect("failed to create storage");

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

            let stf = (build_stf)(&genesis_result);

            let block_interval = configured_block_interval();
            let max_txs_per_block = configured_max_txs_per_block();
            let dev_config = DevConfig {
                block_interval: Some(block_interval),
                initial_height,
                chain_id: rpc_config.chain_id,
                ..Default::default()
            };

            let mempool: SharedMempool<Mempool<TxContext>> = new_shared_mempool();

            // Build block archive callback (always on)
            let archive_cb = build_block_archive(context_for_archive).await;

            let rpc_handle = if rpc_config.enabled {
                let chain_index = Arc::new(
                    PersistentChainIndex::new(&chain_index_db_path)
                        .expect("failed to open chain index database"),
                );

                if let Err(e) = chain_index.initialize() {
                    tracing::warn!("Failed to initialize chain index: {:?}", e);
                }

                let subscriptions = Arc::new(SubscriptionManager::new());

                let codes_for_rpc = Arc::new(build_codes());
                let state_provider_config = ChainStateProviderConfig {
                    chain_id: rpc_config.chain_id,
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
                    http_addr: rpc_config.http_addr,
                    chain_id: rpc_config.chain_id,
                };

                tracing::info!("Starting JSON-RPC server on {}", rpc_config.http_addr);
                let handle = start_server_with_subscriptions(
                    server_config,
                    state_provider,
                    Arc::clone(&subscriptions),
                )
                .await
                .expect("failed to start RPC server");

                let consensus = DevConsensus::with_rpc_and_mempool(
                    stf,
                    storage,
                    codes,
                    dev_config,
                    chain_index,
                    subscriptions,
                    mempool.clone(),
                )
                .with_indexing_enabled(rpc_config.enable_block_indexing)
                .with_block_archive(archive_cb);
                let dev: Arc<DevConsensus<Stf, S, Codes, TxContext, PersistentChainIndex>> =
                    Arc::new(consensus);

                tracing::info!(
                    "Block interval: {:?}, max_txs_per_block: {}, starting at height {}",
                    block_interval,
                    max_txs_per_block,
                    initial_height
                );

                tracing::info!("Starting block production... (Ctrl+C to stop)");

                tokio::select! {
                    _ = dev.run_block_production_with_mempool(context_for_shutdown.clone(), max_txs_per_block) => {
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

                Some(handle)
            } else {
                let consensus = DevConsensus::with_mempool(stf, storage, codes, dev_config, mempool)
                    .with_block_archive(archive_cb);
                let dev: Arc<DevConsensus<Stf, S, Codes, TxContext, evolve_server::NoopChainIndex>> =
                    Arc::new(consensus);

                tracing::info!(
                    "Block interval: {:?}, max_txs_per_block: {}, starting at height {}",
                    block_interval,
                    max_txs_per_block,
                    initial_height
                );

                tracing::info!("Starting block production... (Ctrl+C to stop)");

                tokio::select! {
                    _ = dev.run_block_production_with_mempool(context_for_shutdown.clone(), max_txs_per_block) => {
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

            if let Some(handle) = rpc_handle {
                tracing::info!("Stopping RPC server...");
                handle.stop().expect("failed to stop RPC server");
                tracing::info!("RPC server stopped");
            }
        }
    });
}

/// Run the dev node with RPC and ETH mempool using in-memory mock storage.
pub fn run_dev_node_with_rpc_and_mempool_mock_storage<
    Stf,
    Codes,
    G,
    BuildGenesisStf,
    BuildStf,
    BuildCodes,
    RunGenesis,
>(
    data_dir: impl AsRef<Path>,
    build_genesis_stf: BuildGenesisStf,
    build_stf: BuildStf,
    build_codes: BuildCodes,
    run_genesis: RunGenesis,
    rpc_config: RpcConfig,
) where
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: StfExecutor<TxContext, MockStorage, Codes> + Send + Sync + 'static,
    G: BorshSerialize + BorshDeserialize + Clone + Debug + Send + Sync + 'static,
    BuildGenesisStf: Fn() -> Stf + Send + Sync + 'static,
    BuildStf: Fn(&G) -> Stf + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Clone + Send + Sync + 'static,
    RunGenesis: Fn(
            &Stf,
            &Codes,
            &MockStorage,
        ) -> Result<GenesisOutput<G>, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
{
    run_dev_node_with_rpc_and_mempool_eth(
        data_dir,
        build_genesis_stf,
        build_stf,
        build_codes,
        run_genesis,
        |_context, _config| async { Ok(MockStorage::new()) },
        rpc_config,
    )
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
    Tx: Transaction + MempoolTx + Encodable + Send + Sync + 'static,
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
    let mut operations = state_changes_to_operations(changes);

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

fn state_changes_to_operations(changes: Vec<StateChange>) -> Vec<Operation> {
    changes
        .into_iter()
        .map(|change| match change {
            StateChange::Set { key, value } => Operation::Set { key, value },
            StateChange::Remove { key } => Operation::Remove { key },
        })
        .collect()
}
