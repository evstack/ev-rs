//! Shared external-consensus node runner.
//!
//! This module owns the orchestration around the EVNode gRPC server:
//! storage bootstrap, optional JSON-RPC startup, serialized commit/indexing,
//! and shutdown-time chain-state persistence.

use std::fmt::Debug;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use alloy_primitives::{Address, B256, U256};
use borsh::{BorshDeserialize, BorshSerialize};
use commonware_runtime::tokio::{Config as TokioConfig, Context as TokioContext, Runner};
use commonware_runtime::{Runner as RunnerTrait, Spawner};
use evolve_chain_index::{
    build_index_data, BlockMetadata, ChainIndex, ChainStateProvider, ChainStateProviderConfig,
    PersistentChainIndex, RpcExecutionContext, StateQuerier, StorageStateQuerier,
};
use evolve_core::{AccountId, ReadonlyKV};
use evolve_eth_jsonrpc::{start_server_with_subscriptions, RpcServerConfig, SubscriptionManager};
use evolve_mempool::{new_shared_mempool, Mempool, SharedMempool};
use evolve_node::{GenesisOutput, HasTokenAccountId, NodeConfig};
use evolve_rpc_types::SyncStatus;
use evolve_server::{
    compute_block_hash, load_chain_state, save_chain_state, state_changes_to_operations,
    BlockBuilder, ChainState, StfExecutor,
};
use evolve_stf_traits::AccountsCodeStorage;
use evolve_storage::{Operation, Storage, StorageConfig};
use evolve_tx_eth::TxContext;
use tokio::sync::oneshot;

use crate::{
    BlockExecutedInfo, EvnodeError, EvnodeServer, EvnodeServerConfig, EvnodeStfExecutor,
    ExecutorServiceConfig, OnBlockExecuted,
};

type SharedChainIndex = Arc<PersistentChainIndex>;

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

struct ExternalConsensusSinkConfig {
    initial_height: u64,
    chain_id: u64,
    max_gas: u64,
    indexing_enabled: bool,
}

struct ExternalConsensusCommitSink {
    sender: Option<mpsc::SyncSender<QueuedBlockExecution>>,
    worker: Option<JoinHandle<()>>,
    current_height: Arc<AtomicU64>,
}

struct QueuedBlockExecution {
    info: BlockExecutedInfo,
    result_tx: oneshot::Sender<Result<B256, EvnodeError>>,
}

impl ExternalConsensusCommitSink {
    fn spawn<S>(
        storage: S,
        chain_index: Option<SharedChainIndex>,
        config: ExternalConsensusSinkConfig,
    ) -> Self
    where
        S: Storage + Clone + Send + 'static,
    {
        const MAX_PENDING_BLOCKS: usize = 16;

        let (sender, receiver) = mpsc::sync_channel::<QueuedBlockExecution>(MAX_PENDING_BLOCKS);
        let current_height = Arc::new(AtomicU64::new(config.initial_height));
        let current_height_for_worker = Arc::clone(&current_height);

        let worker = std::thread::spawn(move || {
            let mut parent_hash =
                resolve_initial_parent_hash(chain_index.as_ref(), config.initial_height);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build commit sink runtime");

            while let Ok(queued) = receiver.recv() {
                let QueuedBlockExecution { info, result_tx } = queued;
                let operations = state_changes_to_operations(info.state_changes);
                let result = runtime.block_on(async {
                    storage.batch(operations).await.map_err(|err| {
                        EvnodeError::Storage(format!("storage batch failed: {err:?}"))
                    })?;
                    storage.commit().await.map_err(|err| {
                        EvnodeError::Storage(format!("storage commit failed: {err:?}"))
                    })
                });

                match result {
                    Ok(commit_hash) => {
                        let committed_state_root = B256::from_slice(commit_hash.as_bytes());
                        if committed_state_root != info.state_root {
                            tracing::warn!(
                                height = info.height,
                                preview_state_root = %info.state_root,
                                committed_state_root = %committed_state_root,
                                "execution state root differed from committed storage root"
                            );
                        }
                        let block_hash =
                            compute_block_hash(info.height, info.timestamp, parent_hash);
                        let metadata = BlockMetadata::new(
                            block_hash,
                            parent_hash,
                            committed_state_root,
                            info.timestamp,
                            config.max_gas,
                            Address::ZERO,
                            config.chain_id,
                        );

                        let block = BlockBuilder::<TxContext>::new()
                            .number(info.height)
                            .timestamp(info.timestamp)
                            .transactions(info.transactions)
                            .build();
                        let (stored_block, stored_txs, stored_receipts) =
                            build_index_data(&block, &info.block_result, &metadata);

                        if config.indexing_enabled {
                            if let Some(ref index) = chain_index {
                                if let Err(err) =
                                    index.store_block(stored_block, stored_txs, stored_receipts)
                                {
                                    tracing::warn!(
                                        "Failed to index block {}: {:?}",
                                        info.height,
                                        err
                                    );
                                } else {
                                    tracing::debug!(
                                        "Indexed block {} (hash={}, state_root={})",
                                        info.height,
                                        block_hash,
                                        committed_state_root
                                    );
                                }
                            }
                        }

                        parent_hash = block_hash;
                        current_height_for_worker.store(info.height, Ordering::SeqCst);
                        let _ = result_tx.send(Ok(committed_state_root));
                    }
                    Err(err) => {
                        let _ = result_tx.send(Err(err));
                    }
                }
            }
        });

        Self {
            sender: Some(sender),
            worker: Some(worker),
            current_height,
        }
    }

    fn callback(&self) -> OnBlockExecuted {
        let sender = self
            .sender
            .as_ref()
            .expect("external consensus sink sender missing")
            .clone();

        Arc::new(move |info| {
            let sender = sender.clone();
            Box::pin(async move {
                let (result_tx, result_rx) = oneshot::channel();
                sender
                    .send(QueuedBlockExecution { info, result_tx })
                    .map_err(|_| {
                        EvnodeError::Unavailable(
                            "external consensus commit sink stopped unexpectedly".to_string(),
                        )
                    })?;
                result_rx.await.map_err(|_| {
                    EvnodeError::Unavailable(
                        "external consensus commit sink stopped before returning a root"
                            .to_string(),
                    )
                })?
            })
        })
    }

    fn current_height(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.current_height)
    }

    fn finish(mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("external consensus commit sink panicked");
        }
    }
}

fn init_persistent_chain_index(
    data_dir: &Path,
    enable_chain_index: bool,
) -> Option<SharedChainIndex> {
    if !enable_chain_index {
        return None;
    }

    let chain_index_db_path = data_dir.join("chain-index.sqlite");
    let index = Arc::new(
        PersistentChainIndex::new(&chain_index_db_path)
            .expect("failed to open chain index database"),
    );
    if let Err(err) = index.initialize() {
        tracing::warn!("Failed to initialize chain index: {:?}", err);
    }
    Some(index)
}

fn resolve_initial_parent_hash(
    chain_index: Option<&SharedChainIndex>,
    initial_height: u64,
) -> B256 {
    let Some(index) = chain_index else {
        return B256::ZERO;
    };

    match index.get_block(initial_height) {
        Ok(Some(block)) => {
            tracing::info!("Seeding parent hash from indexed block {}", initial_height);
            return block.hash;
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                "Failed to read indexed block {} while seeding parent hash: {:?}",
                initial_height,
                err
            );
        }
    }

    let latest_indexed = match index.latest_block_number() {
        Ok(latest) => latest,
        Err(err) => {
            tracing::warn!(
                "Failed to read latest indexed block while seeding parent hash: {:?}",
                err
            );
            return B256::ZERO;
        }
    };

    let Some(latest_height) = latest_indexed else {
        return B256::ZERO;
    };

    if latest_height != initial_height {
        tracing::warn!(
            "Chain state height {} does not match indexed head {}; seeding parent hash from indexed head",
            initial_height,
            latest_height
        );
    }

    match index.get_block(latest_height) {
        Ok(Some(block)) => block.hash,
        Ok(None) => {
            tracing::warn!(
                "Indexed head {} missing block payload while seeding parent hash",
                latest_height
            );
            B256::ZERO
        }
        Err(err) => {
            tracing::warn!(
                "Failed to read indexed head block {} while seeding parent hash: {:?}",
                latest_height,
                err
            );
            B256::ZERO
        }
    }
}

async fn run_server_with_shutdown<F, E>(
    serve_future: F,
    context_for_shutdown: TokioContext,
    shutdown_timeout_secs: u64,
) where
    F: Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    tokio::pin!(serve_future);
    tokio::select! {
        result = &mut serve_future => {
            if let Err(err) = result {
                tracing::error!("server error: {}", err);
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

async fn start_external_consensus_rpc_server<S, Codes, Exec, BuildCodes>(
    config: &NodeConfig,
    storage: S,
    mempool: SharedMempool<Mempool<TxContext>>,
    chain_index: &Option<SharedChainIndex>,
    token_account_id: AccountId,
    executor: Arc<Exec>,
    build_codes: &BuildCodes,
) -> Option<RpcRuntimeHandle>
where
    S: ReadonlyKV + Clone + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Exec: RpcExecutionContext + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Clone + Send + Sync + 'static,
{
    if !config.rpc.enabled {
        return None;
    }

    let chain_index = Arc::clone(chain_index.as_ref().expect("chain index required for RPC"));
    let subscriptions = Arc::new(SubscriptionManager::new());
    let codes_for_rpc = Arc::new(build_codes());
    let state_provider_config = ChainStateProviderConfig {
        chain_id: config.chain.chain_id,
        protocol_version: evolve_chain_index::DEFAULT_PROTOCOL_VERSION.to_string(),
        gas_price: U256::ZERO,
        sync_status: SyncStatus::NotSyncing(false),
    };
    let state_querier: Arc<dyn StateQuerier> = Arc::new(StorageStateQuerier::new(
        storage,
        token_account_id,
        Arc::clone(&codes_for_rpc),
        executor,
    ));
    let state_provider = ChainStateProvider::with_mempool(
        Arc::clone(&chain_index),
        state_provider_config,
        codes_for_rpc,
        mempool,
    )
    .with_state_querier(state_querier);
    state_provider
        .ensure_rpc_compatibility()
        .expect("external consensus RPC requires mempool, verifier, and state querier");

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

/// Run an external-consensus execution node for ETH transactions.
///
/// The caller supplies application-specific STF/genesis/storage builders while
/// this runner owns storage bootstrap, query-plane startup, EVNode serving,
/// serialized block commit/indexing, and shutdown persistence.
pub fn run_external_consensus_node_eth<
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
    config: NodeConfig,
    build_genesis_stf: BuildGenesisStf,
    build_stf: BuildStf,
    build_codes: BuildCodes,
    run_genesis: RunGenesis,
    build_storage: BuildStorage,
) where
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<TxContext, S, Codes>
        + EvnodeStfExecutor<S, Codes>
        + RpcExecutionContext
        + Send
        + Sync
        + 'static,
    G: BorshSerialize
        + BorshDeserialize
        + Clone
        + Debug
        + HasTokenAccountId
        + Send
        + Sync
        + 'static,
    BuildGenesisStf: Fn() -> Stf + Send + Sync + 'static,
    BuildStf: Fn(&G) -> Stf + Send + Sync + 'static,
    BuildCodes: Fn() -> Codes + Clone + Send + Sync + 'static,
    RunGenesis: Fn(&Stf, &Codes, &S) -> Result<GenesisOutput<G>, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync
        + 'static,
    BuildStorage: Fn(TokioContext, StorageConfig) -> BuildStorageFut + Send + Sync + 'static,
    BuildStorageFut:
        Future<Output = Result<S, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    tracing::info!("=== Evolve External Consensus Node ===");
    std::fs::create_dir_all(&config.storage.path).expect("failed to create data directory");

    let data_dir = Path::new(&config.storage.path);
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
        let config = config.clone();

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

                    let chain_state = ChainState {
                        height: 1,
                        genesis_result: output.genesis_result.clone(),
                    };
                    let mut operations = state_changes_to_operations(output.changes);
                    operations.push(Operation::Set {
                        key: evolve_server::CHAIN_STATE_KEY.to_vec(),
                        value: borsh::to_vec(&chain_state)
                            .map_err(|e| format!("serialize chain state: {e}"))
                            .expect("serialize chain state"),
                    });

                    storage
                        .batch(operations)
                        .await
                        .expect("genesis batch failed");
                    storage.commit().await.expect("genesis commit failed");

                    tracing::info!("Genesis complete. Result: {:?}", output.genesis_result);
                    (output.genesis_result, 1)
                }
            };

            let stf = (build_stf)(&genesis_result);
            let mempool: SharedMempool<Mempool<TxContext>> = new_shared_mempool();
            let chain_index = init_persistent_chain_index(
                Path::new(&config.storage.path),
                config.rpc.enabled || config.rpc.enable_block_indexing,
            );
            let query_executor = Arc::new((build_stf)(&genesis_result));
            let rpc_handle = start_external_consensus_rpc_server(
                &config,
                storage.clone(),
                mempool.clone(),
                &chain_index,
                genesis_result.token_account_id(),
                query_executor,
                build_codes.as_ref(),
            )
            .await;

            let executor_config = ExecutorServiceConfig::default();
            let commit_sink = ExternalConsensusCommitSink::spawn(
                storage.clone(),
                chain_index,
                ExternalConsensusSinkConfig {
                    initial_height,
                    chain_id: config.chain.chain_id,
                    max_gas: executor_config.max_gas,
                    indexing_enabled: config.rpc.enable_block_indexing,
                },
            );
            let current_height = commit_sink.current_height();

            let grpc_config = EvnodeServerConfig {
                addr: config.parsed_grpc_addr(),
                enable_gzip: config.grpc.enable_gzip,
                max_message_size: config.grpc_max_message_size_usize(),
                executor_config,
            };
            let server =
                EvnodeServer::with_mempool(grpc_config, stf, storage.clone(), codes, mempool)
                    .with_on_block_executed(commit_sink.callback());

            tracing::info!("Server ready. Press Ctrl+C to stop.");
            run_server_with_shutdown(
                server.serve(),
                context_for_shutdown,
                config.operations.shutdown_timeout_secs,
            )
            .await;

            commit_sink.finish();

            let chain_state = ChainState {
                height: current_height.load(Ordering::SeqCst),
                genesis_result,
            };
            if let Err(err) = save_chain_state(&storage, &chain_state).await {
                tracing::error!("Failed to save chain state: {}", err);
            }

            if let Some(handle) = rpc_handle {
                tracing::info!("Stopping JSON-RPC server...");
                handle.stop();
            }

            tracing::info!("Shutdown complete.");
        }
    });
}
