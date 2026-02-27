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
//! # Start with a custom genesis file
//! evd run --genesis-file path/to/genesis.json
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256, U256};
use clap::{Args, Parser, Subcommand};
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};
use evolve_chain_index::{
    build_index_data, BlockMetadata, ChainIndex, ChainStateProvider, ChainStateProviderConfig,
    PersistentChainIndex, StateQuerier, StorageStateQuerier,
};
use evolve_core::{AccountId, ReadonlyKV};
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
use evolve_testapp::genesis_config::{load_genesis_config, EvdGenesisConfig, EvdGenesisResult};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis_inner, install_account_codes,
    PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::TokenRef;
use evolve_tx_eth::TxContext;
use evolve_tx_eth::{register_runtime_contract_account, resolve_or_create_eoa_account};
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
            let genesis_config = match load_genesis_config(args.custom.genesis_file.as_deref()) {
                Ok(genesis_config) => genesis_config,
                Err(err) => {
                    tracing::error!("{err}");
                    std::process::exit(2);
                }
            };
            run_node(config, genesis_config);
        }
        Commands::Init(args) => {
            let config = resolve_node_config_init(&args.common);
            init_node_tracing(&config.observability.log_level);
            let genesis_config = match load_genesis_config(args.custom.genesis_file.as_deref()) {
                Ok(genesis_config) => genesis_config,
                Err(err) => {
                    tracing::error!("{err}");
                    std::process::exit(2);
                }
            };
            init_genesis(&config.storage.path, genesis_config);
        }
    }
}

type TokioContext = commonware_runtime::tokio::Context;
type NodeStorage = QmdbStorage<TokioContext>;
type SharedChainIndex = Arc<PersistentChainIndex>;
type RpcMempool = SharedMempool<Mempool<TxContext>>;

struct RpcRuntimeHandle {
    stop_fn: Option<Box<dyn FnOnce() + Send>>,
}

impl RpcRuntimeHandle {
    fn new(stop_fn: impl FnOnce() + Send + 'static) -> Self {
        Self {
            stop_fn: Some(Box::new(stop_fn)),
        }
    }

    fn stop(mut self) {
        if let Some(stop_fn) = self.stop_fn.take() {
            stop_fn();
        }
    }
}

async fn init_storage_and_genesis(
    context: TokioContext,
    storage_config: StorageConfig,
    genesis_config: Option<EvdGenesisConfig>,
) -> (NodeStorage, EvdGenesisResult, u64) {
    let storage = QmdbStorage::new(context, storage_config)
        .await
        .expect("failed to create storage");

    let codes = build_codes();
    tracing::info!("Installed account codes: {:?}", codes.list_identifiers());

    match load_chain_state::<EvdGenesisResult, _>(&storage) {
        Some(state) => {
            tracing::info!("Resuming from existing state at height {}", state.height);
            (storage, state.genesis_result, state.height)
        }
        None => {
            tracing::info!("No existing state found, running genesis...");
            let output = run_genesis(&storage, &codes, genesis_config.as_ref());
            commit_genesis(&storage, output.changes, &output.genesis_result)
                .await
                .expect("genesis commit failed");
            tracing::info!("Genesis complete: {:?}", output.genesis_result);
            (storage, output.genesis_result, 1)
        }
    }
}

fn init_chain_index(config: &NodeConfig) -> Option<SharedChainIndex> {
    if !config.rpc.enabled && !config.rpc.enable_block_indexing {
        return None;
    }

    let chain_index_db_path =
        std::path::PathBuf::from(&config.storage.path).join("chain-index.sqlite");
    let index = Arc::new(
        PersistentChainIndex::new(&chain_index_db_path)
            .expect("failed to open chain index database"),
    );
    if let Err(err) = index.initialize() {
        tracing::warn!("Failed to initialize chain index: {:?}", err);
    }
    Some(index)
}

async fn start_rpc_server(
    config: &NodeConfig,
    storage: NodeStorage,
    mempool: RpcMempool,
    chain_index: &Option<SharedChainIndex>,
    token_account_id: AccountId,
) -> Option<RpcRuntimeHandle> {
    if !config.rpc.enabled {
        return None;
    }

    let subscriptions = Arc::new(SubscriptionManager::new());
    let codes_for_rpc = Arc::new(build_codes());
    let state_provider_config = ChainStateProviderConfig {
        chain_id: config.chain.chain_id,
        protocol_version: "0x1".to_string(),
        gas_price: U256::ZERO,
        sync_status: SyncStatus::NotSyncing(false),
    };

    let state_querier: Arc<dyn StateQuerier> =
        Arc::new(StorageStateQuerier::new(storage, token_account_id));
    let state_provider = ChainStateProvider::with_mempool(
        Arc::clone(chain_index.as_ref().expect("chain index required for RPC")),
        state_provider_config,
        codes_for_rpc,
        mempool,
    )
    .with_state_querier(state_querier);

    let rpc_addr = config.parsed_rpc_addr();
    let server_config = RpcServerConfig {
        http_addr: rpc_addr,
        chain_id: config.chain.chain_id,
    };

    tracing::info!("Starting JSON-RPC server on {}", rpc_addr);
    let handle =
        start_server_with_subscriptions(server_config, state_provider, Arc::clone(&subscriptions))
            .await
            .expect("failed to start RPC server");

    Some(RpcRuntimeHandle::new(move || {
        handle.stop().expect("failed to stop RPC server");
    }))
}

fn build_on_block_executed(
    storage: NodeStorage,
    chain_index: Option<SharedChainIndex>,
    initial_height: u64,
    callback_chain_id: u64,
    callback_max_gas: u64,
    callback_indexing_enabled: bool,
) -> (OnBlockExecuted, Arc<AtomicU64>) {
    let parent_hash = Arc::new(std::sync::RwLock::new(B256::ZERO));
    let current_height = Arc::new(AtomicU64::new(initial_height));
    let parent_hash_for_callback = Arc::clone(&parent_hash);
    let current_height_for_callback = Arc::clone(&current_height);

    let on_block_executed: OnBlockExecuted = Arc::new(move |info| {
        let operations = state_changes_to_operations(info.state_changes);
        let commit_hash = futures::executor::block_on(async {
            storage
                .batch(operations)
                .await
                .expect("storage batch failed");
            storage.commit().await.expect("storage commit failed")
        });
        let state_root = B256::from_slice(commit_hash.as_bytes());

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

        let block = BlockBuilder::<TxContext>::new()
            .number(info.height)
            .timestamp(info.timestamp)
            .transactions(info.transactions)
            .build();
        let (stored_block, stored_txs, stored_receipts) =
            build_index_data(&block, &info.block_result, &metadata);

        if callback_indexing_enabled {
            if let Some(ref index) = chain_index {
                if let Err(err) = index.store_block(stored_block, stored_txs, stored_receipts) {
                    tracing::warn!("Failed to index block {}: {:?}", info.height, err);
                } else {
                    tracing::debug!(
                        "Indexed block {} (hash={}, state_root={})",
                        info.height,
                        block_hash,
                        state_root
                    );
                }
            }
        }

        *parent_hash_for_callback.write().unwrap() = block_hash;
        current_height_for_callback.store(info.height, Ordering::SeqCst);
    });

    (on_block_executed, current_height)
}

fn log_server_configuration(config: &NodeConfig, initial_height: u64) {
    let grpc_addr = config.parsed_grpc_addr();
    tracing::info!("Starting gRPC server on {}", grpc_addr);
    tracing::info!("Configuration:");
    tracing::info!("  - Chain ID: {}", config.chain.chain_id);
    tracing::info!("  - gRPC compression: {}", config.grpc.enable_gzip);
    tracing::info!("  - JSON-RPC: {}", config.rpc.enabled);
    tracing::info!("  - Block indexing: {}", config.rpc.enable_block_indexing);
    tracing::info!("  - Initial height: {}", initial_height);
}

async fn run_server_with_shutdown<F, E>(
    serve_future: F,
    context_for_shutdown: TokioContext,
    shutdown_timeout_secs: u64,
) where
    F: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    tokio::pin!(serve_future);
    tokio::select! {
        result = &mut serve_future => {
            if let Err(err) = result {
                tracing::error!("gRPC server error: {}", err);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received Ctrl+C, shutting down...");
            context_for_shutdown
                .stop(0, Some(Duration::from_secs(shutdown_timeout_secs)))
                .await
                .expect("shutdown failed");
        }
    }
}

async fn persist_chain_state(
    storage: &NodeStorage,
    current_height: &Arc<AtomicU64>,
    genesis_result: EvdGenesisResult,
) {
    let chain_state = ChainState {
        height: current_height.load(Ordering::SeqCst),
        genesis_result,
    };
    if let Err(err) = save_chain_state(storage, &chain_state).await {
        tracing::error!("Failed to save chain state: {}", err);
    }
}

fn stop_rpc_server(rpc_handle: Option<RpcRuntimeHandle>) {
    if let Some(handle) = rpc_handle {
        tracing::info!("Stopping JSON-RPC server...");
        handle.stop();
    }
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

    runner.start(move |context| async move {
        let context_for_shutdown = context.clone();
        let (storage, genesis_result, initial_height) =
            init_storage_and_genesis(context, storage_config, genesis_config).await;

        let stf = build_mempool_stf(default_gas_config(), genesis_result.scheduler);
        let mempool: RpcMempool = new_shared_mempool();
        let chain_index = init_chain_index(&config);
        let rpc_handle = start_rpc_server(
            &config,
            storage.clone(),
            mempool.clone(),
            &chain_index,
            genesis_result.token,
        )
        .await;

        let executor_config = ExecutorServiceConfig::default();
        let (on_block_executed, current_height) = build_on_block_executed(
            storage.clone(),
            chain_index,
            initial_height,
            config.chain.chain_id,
            executor_config.max_gas,
            config.rpc.enable_block_indexing,
        );
        log_server_configuration(&config, initial_height);

        let grpc_config = EvnodeServerConfig {
            addr: config.parsed_grpc_addr(),
            enable_gzip: config.grpc.enable_gzip,
            max_message_size: config.grpc_max_message_size_usize(),
            executor_config,
        };
        let server =
            EvnodeServer::with_mempool(grpc_config, stf, storage.clone(), build_codes(), mempool)
                .with_on_block_executed(on_block_executed);

        tracing::info!("Server ready. Press Ctrl+C to stop.");
        run_server_with_shutdown(
            server.serve(),
            context_for_shutdown,
            config.operations.shutdown_timeout_secs,
        )
        .await;

        persist_chain_state(&storage, &current_height, genesis_result).await;
        stop_rpc_server(rpc_handle);
        tracing::info!("Shutdown complete.");
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
        let output = run_genesis(&storage, &codes, genesis_config.as_ref());

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

/// Run genesis using the default testapp genesis or a custom genesis config.
fn run_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
    genesis_config: Option<&EvdGenesisConfig>,
) -> GenesisOutput<EvdGenesisResult> {
    match genesis_config {
        Some(config) => run_custom_genesis(storage, codes, config),
        None => run_default_genesis(storage, codes),
    }
}

/// Default genesis using ETH-address-derived AccountIds for EOA balances.
fn run_default_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
) -> GenesisOutput<EvdGenesisResult> {
    use evolve_core::BlockContext;
    use std::str::FromStr;

    tracing::info!("Running default ETH-mapped genesis...");

    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
    let genesis_block = BlockContext::new(0, 0);
    let alice_eth_address = std::env::var("GENESIS_ALICE_ETH_ADDRESS")
        .ok()
        .and_then(|s| Address::from_str(s.trim()).ok())
        .map(Into::into)
        .unwrap_or([0xAA; 20]);
    let bob_eth_address = std::env::var("GENESIS_BOB_ETH_ADDRESS")
        .ok()
        .and_then(|s| Address::from_str(s.trim()).ok())
        .map(Into::into)
        .unwrap_or([0xBB; 20]);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            do_eth_genesis_inner(alice_eth_address, bob_eth_address, env)
        })
        .expect("genesis failed");

    let changes = state.into_changes().expect("failed to get state changes");

    let genesis_result = EvdGenesisResult {
        token: accounts.evolve,
        scheduler: accounts.scheduler,
    };

    GenesisOutput {
        genesis_result,
        changes,
    }
}

/// Custom genesis with ETH EOA accounts from a genesis JSON file.
///
/// Funds balances at ETH-address-derived AccountIds.
fn run_custom_genesis<S: ReadonlyKV + Storage>(
    storage: &S,
    codes: &AccountStorageMock,
    genesis_config: &EvdGenesisConfig,
) -> GenesisOutput<EvdGenesisResult> {
    use evolve_core::BlockContext;

    let funded_accounts: Vec<([u8; 20], u128)> = genesis_config
        .accounts
        .iter()
        .filter(|acc| acc.balance > 0)
        .map(|acc| {
            let addr = acc
                .parse_address()
                .expect("invalid address in genesis config");
            (addr.into_array(), acc.balance)
        })
        .collect();
    let minter = AccountId::new(genesis_config.minter_id);
    let metadata = genesis_config.token.to_metadata();

    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, PLACEHOLDER_ACCOUNT);
    let genesis_block = BlockContext::new(0, 0);

    let (genesis_result, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            let balances: Vec<(AccountId, u128)> = funded_accounts
                .iter()
                .map(
                    |(eth_addr, balance)| -> evolve_core::SdkResult<(AccountId, u128)> {
                        let addr = Address::from(*eth_addr);
                        Ok((resolve_or_create_eoa_account(addr, env)?, *balance))
                    },
                )
                .collect::<evolve_core::SdkResult<Vec<_>>>()?;

            let token = TokenRef::initialize(metadata.clone(), balances, Some(minter), env)?.0;
            let _token_eth_addr = register_runtime_contract_account(token.0, env)?;

            let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
            let _scheduler_eth_addr = register_runtime_contract_account(scheduler_acc.0, env)?;
            scheduler_acc.update_begin_blockers(vec![], env)?;

            Ok(EvdGenesisResult {
                token: token.0,
                scheduler: scheduler_acc.0,
            })
        })
        .expect("genesis failed");

    let changes = state.into_changes().expect("failed to get state changes");

    GenesisOutput {
        genesis_result,
        changes,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::encoding::Encodable;
    use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
    use evolve_core::Message;
    use evolve_storage::MockStorage;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard};

    static ENV_VAR_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
        _guard: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let guard = ENV_VAR_LOCK.lock().expect("env var lock poisoned");
            let old = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key,
                old,
                _guard: guard,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.old {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn apply_changes_to_map(changes: Vec<StateChange>) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let mut out = BTreeMap::new();
        for change in changes {
            match change {
                StateChange::Set { key, value } => {
                    out.insert(key, value);
                }
                StateChange::Remove { key } => {
                    out.remove(&key);
                }
            }
        }
        out
    }

    fn read_token_balance(
        state: &BTreeMap<Vec<u8>, Vec<u8>>,
        token_account_id: AccountId,
        account_id: AccountId,
    ) -> u128 {
        let mut key = token_account_id.as_bytes().to_vec();
        key.push(1u8); // Token::balances storage prefix
        key.extend(account_id.encode().expect("encode account id"));

        match state.get(&key) {
            Some(value) => Message::from_bytes(value.clone())
                .get::<u128>()
                .expect("decode balance"),
            None => 0,
        }
    }

    fn eoa_account_ids(state: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<AccountId> {
        state
            .iter()
            .filter_map(|(key, value)| {
                if key.len() != 33 || key[0] != ACCOUNT_IDENTIFIER_PREFIX {
                    return None;
                }
                let code_id = Message::from_bytes(value.clone()).get::<String>().ok()?;
                if code_id != "EthEoaAccount" {
                    return None;
                }
                let account_bytes: [u8; 32] = key[1..33].try_into().ok()?;
                Some(AccountId::from_bytes(account_bytes))
            })
            .collect()
    }

    #[test]
    fn default_genesis_funds_eth_mapped_sender_account() {
        let _alice_addr = EnvVarGuard::set(
            "GENESIS_ALICE_ETH_ADDRESS",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        );
        let _bob_addr = EnvVarGuard::set(
            "GENESIS_BOB_ETH_ADDRESS",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        );
        let _alice_bal = EnvVarGuard::set("GENESIS_ALICE_TOKEN_BALANCE", "1234");
        let _bob_bal = EnvVarGuard::set("GENESIS_BOB_TOKEN_BALANCE", "5678");

        let storage = MockStorage::new();
        let codes = build_codes();
        let output = run_default_genesis(&storage, &codes);
        let state = apply_changes_to_map(output.changes);

        let eoa_ids = eoa_account_ids(&state);
        assert_eq!(eoa_ids.len(), 2);
        assert!(eoa_ids.iter().any(|id| read_token_balance(
            &state,
            output.genesis_result.token,
            *id
        ) == 1234));
    }
}
