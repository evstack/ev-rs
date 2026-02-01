//! ExecutorService gRPC implementation for EVNode.
//!
//! This module provides the gRPC server implementation for the EVNode ExecutorService,
//! which allows ev-node to interact with the Evolve execution layer.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use alloy_primitives::B256;
use async_trait::async_trait;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::ReadonlyKV;
use evolve_mempool::{Mempool, SharedMempool};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{AccountsCodeStorage, StateChange};
use evolve_tx_eth::{TxContext, TypedTransaction};
use prost_types::Timestamp;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

use crate::error::EvnodeError;
use crate::proto::evnode::v1::{
    executor_service_server::ExecutorService, ExecuteTxsRequest, ExecuteTxsResponse, FilterStatus,
    FilterTxsRequest, FilterTxsResponse, GetExecutionInfoRequest, GetExecutionInfoResponse,
    GetTxsRequest, GetTxsResponse, InitChainRequest, InitChainResponse, SetFinalRequest,
    SetFinalResponse,
};

/// Compute a state root from changes.
///
/// This is a simple hash of all changes for MVP.
/// In production, this would be a proper Merkle root.
pub fn compute_state_root(changes: &[StateChange]) -> B256 {
    use alloy_primitives::keccak256;

    if changes.is_empty() {
        return B256::ZERO;
    }

    let mut data = Vec::new();
    for change in changes {
        match change {
            StateChange::Set { key, value } => {
                data.push(0x01);
                data.extend_from_slice(&(key.len() as u32).to_le_bytes());
                data.extend_from_slice(key);
                data.extend_from_slice(&(value.len() as u32).to_le_bytes());
                data.extend_from_slice(value);
            }
            StateChange::Remove { key } => {
                data.push(0x02);
                data.extend_from_slice(&(key.len() as u32).to_le_bytes());
                data.extend_from_slice(key);
            }
        }
    }

    keccak256(&data)
}

/// Default maximum gas per block.
pub const DEFAULT_MAX_GAS: u64 = 30_000_000;

/// Default maximum bytes per transaction.
pub const DEFAULT_MAX_BYTES: u64 = 128 * 1024; // 128KB

/// Block type used by the executor.
pub type ExecutorBlock = evolve_server::Block<TxContext>;

/// Trait for STF execution in the evnode context.
///
/// This abstracts the STF's execution methods so that the executor service
/// can work with any compatible STF implementation.
pub trait EvnodeStfExecutor<S, Codes>: Send + Sync
where
    S: ReadonlyKV,
    Codes: AccountsCodeStorage,
{
    /// Execute a block and return the result with execution state.
    fn execute_block<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        block: &ExecutorBlock,
    ) -> (BlockResult, ExecutionState<'a, S>);

    /// Run genesis initialization.
    ///
    /// This should set up the initial chain state without any transactions.
    fn run_genesis<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        initial_height: u64,
        genesis_time: u64,
    ) -> Result<(Vec<StateChange>, B256), EvnodeError>;
}

/// Configuration for the executor service.
#[derive(Debug, Clone)]
pub struct ExecutorServiceConfig {
    /// Maximum gas allowed per block.
    pub max_gas: u64,
    /// Maximum bytes per transaction.
    pub max_bytes: u64,
}

impl Default for ExecutorServiceConfig {
    fn default() -> Self {
        Self {
            max_gas: DEFAULT_MAX_GAS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// Internal state of the executor service.
struct ExecutorState {
    /// Whether the chain has been initialized.
    initialized: AtomicBool,
    /// Chain ID string.
    chain_id: RwLock<String>,
    /// Initial block height.
    initial_height: AtomicU64,
    /// Current block height.
    current_height: AtomicU64,
    /// Genesis timestamp (Unix seconds).
    genesis_time: AtomicU64,
    /// Last state root.
    last_state_root: RwLock<B256>,
    /// Last finalized height.
    finalized_height: AtomicU64,
    /// Pending state changes (applied but not yet committed).
    pending_changes: RwLock<Vec<StateChange>>,
}

impl ExecutorState {
    fn new() -> Self {
        Self {
            initialized: AtomicBool::new(false),
            chain_id: RwLock::new(String::new()),
            initial_height: AtomicU64::new(1),
            current_height: AtomicU64::new(0),
            genesis_time: AtomicU64::new(0),
            last_state_root: RwLock::new(B256::ZERO),
            finalized_height: AtomicU64::new(0),
            pending_changes: RwLock::new(Vec::new()),
        }
    }
}

/// Callback for handling state changes.
///
/// This is called when state changes are produced by execution.
/// The implementation should persist these changes appropriately.
/// Note: The callback receives a reference to the state changes.
pub type StateChangeCallback = Arc<dyn Fn(&[StateChange]) + Send + Sync>;

/// ExecutorService implementation for EVNode.
///
/// This service implements the gRPC interface that ev-node uses to interact
/// with the Evolve execution layer.
///
/// Note: This implementation keeps state changes in memory. For production use,
/// you should implement proper storage commit logic using the `pending_changes` field
/// or the state change callback.
pub struct ExecutorServiceImpl<Stf, S, Codes>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    /// The state transition function.
    stf: Arc<Stf>,
    /// Storage backend (for reads).
    storage: Arc<S>,
    /// Account codes storage.
    codes: Arc<Codes>,
    /// Optional mempool for transaction sourcing.
    mempool: Option<SharedMempool<Mempool<TxContext>>>,
    /// Service configuration.
    config: ExecutorServiceConfig,
    /// Internal state.
    state: ExecutorState,
    /// Optional callback for state changes.
    on_state_change: Option<StateChangeCallback>,
}

impl<Stf, S, Codes> ExecutorServiceImpl<Stf, S, Codes>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    /// Create a new executor service.
    pub fn new(stf: Stf, storage: S, codes: Codes, config: ExecutorServiceConfig) -> Self {
        Self {
            stf: Arc::new(stf),
            storage: Arc::new(storage),
            codes: Arc::new(codes),
            mempool: None,
            config,
            state: ExecutorState::new(),
            on_state_change: None,
        }
    }

    /// Create a new executor service with a mempool.
    pub fn with_mempool(
        stf: Stf,
        storage: S,
        codes: Codes,
        config: ExecutorServiceConfig,
        mempool: SharedMempool<Mempool<TxContext>>,
    ) -> Self {
        Self {
            stf: Arc::new(stf),
            storage: Arc::new(storage),
            codes: Arc::new(codes),
            mempool: Some(mempool),
            config,
            state: ExecutorState::new(),
            on_state_change: None,
        }
    }

    /// Set a callback for state changes.
    ///
    /// This callback is invoked whenever state changes are produced by execution.
    pub fn with_state_change_callback(mut self, callback: StateChangeCallback) -> Self {
        self.on_state_change = Some(callback);
        self
    }

    /// Get pending state changes.
    ///
    /// This can be used by external code to commit changes to storage.
    pub async fn take_pending_changes(&self) -> Vec<StateChange> {
        std::mem::take(&mut *self.state.pending_changes.write().await)
    }

    /// Check if the chain is initialized.
    fn ensure_initialized(&self) -> Result<(), EvnodeError> {
        if !self.state.initialized.load(Ordering::SeqCst) {
            return Err(EvnodeError::ChainNotInitialized);
        }
        Ok(())
    }

    /// Convert a protobuf timestamp to Unix seconds.
    fn timestamp_to_unix(ts: Option<&Timestamp>) -> u64 {
        ts.map(|t| t.seconds as u64).unwrap_or(0)
    }

    /// Decode a raw transaction into a TxContext.
    fn decode_tx(raw: &[u8]) -> Result<TxContext, EvnodeError> {
        TxContext::decode(raw)
            .map_err(|e| EvnodeError::Transaction(format!("failed to decode transaction: {:?}", e)))
    }

    /// Estimate gas for a transaction.
    fn estimate_tx_gas(tx: &TxContext) -> u64 {
        tx.envelope().gas_limit()
    }

    /// Handle produced state changes.
    async fn handle_state_changes(&self, changes: Vec<StateChange>) {
        // Call the callback if set
        if let Some(ref callback) = self.on_state_change {
            callback(&changes);
        }

        // Store pending changes
        self.state.pending_changes.write().await.extend(changes);
    }
}

#[async_trait]
impl<Stf, S, Codes> ExecutorService for ExecutorServiceImpl<Stf, S, Codes>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    async fn init_chain(
        &self,
        request: Request<InitChainRequest>,
    ) -> Result<Response<InitChainResponse>, Status> {
        // Check if already initialized
        if self.state.initialized.load(Ordering::SeqCst) {
            return Err(EvnodeError::ChainAlreadyInitialized.into());
        }

        let req = request.into_inner();

        // Validate request
        if req.initial_height == 0 {
            return Err(
                EvnodeError::InvalidArgument("initial_height must be > 0".to_string()).into(),
            );
        }

        let genesis_time = Self::timestamp_to_unix(req.genesis_time.as_ref());

        // Run genesis
        let (changes, _) = self
            .stf
            .run_genesis(&self.storage, &self.codes, req.initial_height, genesis_time)
            .map_err(|e| Status::internal(format!("genesis failed: {}", e)))?;

        // Compute state root
        let state_root = compute_state_root(&changes);

        // Handle state changes (store pending, call callback)
        self.handle_state_changes(changes).await;

        // Update state
        *self.state.chain_id.write().await = req.chain_id.clone();
        self.state
            .initial_height
            .store(req.initial_height, Ordering::SeqCst);
        self.state
            .current_height
            .store(req.initial_height, Ordering::SeqCst);
        self.state
            .genesis_time
            .store(genesis_time, Ordering::SeqCst);
        *self.state.last_state_root.write().await = state_root;
        self.state.initialized.store(true, Ordering::SeqCst);

        tracing::info!(
            "Chain initialized: chain_id={}, initial_height={}, genesis_time={}",
            req.chain_id,
            req.initial_height,
            genesis_time
        );

        Ok(Response::new(InitChainResponse {
            state_root: state_root.to_vec(),
        }))
    }

    async fn get_txs(
        &self,
        _request: Request<GetTxsRequest>,
    ) -> Result<Response<GetTxsResponse>, Status> {
        self.ensure_initialized()?;

        let txs = match &self.mempool {
            Some(mempool) => {
                let mut pool = mempool.write().await;
                // Select transactions up to block gas limit (no count limit)
                let (selected, _total_gas) = pool.select_with_gas_budget(self.config.max_gas, 0);

                // Encode selected transactions
                selected.iter().filter_map(|tx| tx.encode().ok()).collect()
            }
            None => {
                // No mempool configured - return empty
                vec![]
            }
        };

        Ok(Response::new(GetTxsResponse { txs }))
    }

    async fn execute_txs(
        &self,
        request: Request<ExecuteTxsRequest>,
    ) -> Result<Response<ExecuteTxsResponse>, Status> {
        self.ensure_initialized()?;

        let req = request.into_inner();

        // Validate request
        if req.block_height == 0 {
            return Err(
                EvnodeError::InvalidArgument("block_height must be > 0".to_string()).into(),
            );
        }

        let timestamp = Self::timestamp_to_unix(req.timestamp.as_ref());

        // Decode transactions
        let mut transactions = Vec::with_capacity(req.txs.len());
        for (i, raw_tx) in req.txs.iter().enumerate() {
            match Self::decode_tx(raw_tx) {
                Ok(tx) => transactions.push(tx),
                Err(e) => {
                    tracing::warn!("Failed to decode transaction {}: {}", i, e);
                    // Skip invalid transactions
                    continue;
                }
            }
        }

        // Build the block
        let block = evolve_server::BlockBuilder::<TxContext>::new()
            .number(req.block_height)
            .timestamp(timestamp)
            .transactions(transactions)
            .build();

        // Execute through STF
        let (result, exec_state) = self.stf.execute_block(&self.storage, &self.codes, &block);

        let changes = exec_state
            .into_changes()
            .map_err(|e| Status::internal(format!("failed to get state changes: {:?}", e)))?;

        // Compute updated state root
        let updated_state_root = compute_state_root(&changes);

        // Handle state changes
        self.handle_state_changes(changes).await;

        // Update state
        self.state
            .current_height
            .store(req.block_height, Ordering::SeqCst);
        *self.state.last_state_root.write().await = updated_state_root;

        // Log execution results
        let successful = result
            .tx_results
            .iter()
            .filter(|r| r.response.is_ok())
            .count();
        let failed = result.tx_results.len() - successful;
        let gas_used: u64 = result.tx_results.iter().map(|r| r.gas_used).sum();

        tracing::info!(
            "Executed block {}: {} txs ({} ok, {} failed), {} gas used",
            req.block_height,
            result.tx_results.len(),
            successful,
            failed,
            gas_used
        );

        Ok(Response::new(ExecuteTxsResponse {
            updated_state_root: updated_state_root.to_vec(),
            max_bytes: self.config.max_bytes,
        }))
    }

    async fn set_final(
        &self,
        request: Request<SetFinalRequest>,
    ) -> Result<Response<SetFinalResponse>, Status> {
        self.ensure_initialized()?;

        let req = request.into_inner();

        // Validate height
        let current = self.state.current_height.load(Ordering::SeqCst);
        if req.block_height > current {
            return Err(EvnodeError::InvalidArgument(format!(
                "cannot finalize height {} > current height {}",
                req.block_height, current
            ))
            .into());
        }

        // Update finalized height
        self.state
            .finalized_height
            .store(req.block_height, Ordering::SeqCst);

        tracing::debug!("Finalized block {}", req.block_height);

        Ok(Response::new(SetFinalResponse {}))
    }

    async fn get_execution_info(
        &self,
        _request: Request<GetExecutionInfoRequest>,
    ) -> Result<Response<GetExecutionInfoResponse>, Status> {
        // This can be called before init to get execution parameters
        Ok(Response::new(GetExecutionInfoResponse {
            max_gas: self.config.max_gas,
        }))
    }

    async fn filter_txs(
        &self,
        request: Request<FilterTxsRequest>,
    ) -> Result<Response<FilterTxsResponse>, Status> {
        self.ensure_initialized()?;

        let req = request.into_inner();

        let mut statuses = Vec::with_capacity(req.txs.len());
        let mut cumulative_bytes: u64 = 0;
        let mut cumulative_gas: u64 = 0;

        for raw_tx in &req.txs {
            let tx_bytes = raw_tx.len() as u64;

            // Try to decode the transaction
            let tx = match Self::decode_tx(raw_tx) {
                Ok(tx) => tx,
                Err(_) => {
                    // Invalid transaction - remove it
                    statuses.push(FilterStatus::FilterRemove as i32);
                    continue;
                }
            };

            let tx_gas = Self::estimate_tx_gas(&tx);

            // Check size limit
            let would_exceed_bytes =
                req.max_bytes > 0 && cumulative_bytes + tx_bytes > req.max_bytes;
            // Check gas limit
            let would_exceed_gas = req.max_gas > 0 && cumulative_gas + tx_gas > req.max_gas;

            if would_exceed_bytes || would_exceed_gas {
                // Transaction is valid but doesn't fit - postpone it
                statuses.push(FilterStatus::FilterPostpone as i32);
            } else {
                // Transaction fits - include it
                cumulative_bytes += tx_bytes;
                cumulative_gas += tx_gas;
                statuses.push(FilterStatus::FilterOk as i32);
            }
        }

        Ok(Response::new(FilterTxsResponse { statuses }))
    }
}
