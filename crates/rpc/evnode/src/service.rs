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

/// Information passed to the `OnBlockExecuted` callback after a block is executed.
///
/// Contains all data needed for storage commit and chain indexing.
pub struct BlockExecutedInfo {
    /// Block height.
    pub height: u64,
    /// Block timestamp (Unix seconds).
    pub timestamp: u64,
    /// State changes produced by execution.
    pub state_changes: Vec<StateChange>,
    /// Full block execution result.
    pub block_result: BlockResult,
    /// State root computed from changes (keccak-based).
    pub state_root: B256,
    /// Transactions that were included in the block.
    pub transactions: Vec<TxContext>,
}

/// Callback invoked after a block is fully executed.
///
/// This provides all data needed for storage commit + chain indexing in one call.
/// When set, this replaces the `StateChangeCallback` for `execute_txs` — the caller
/// takes full responsibility for persisting state changes.
pub type OnBlockExecuted = Arc<dyn Fn(BlockExecutedInfo) + Send + Sync>;

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
    /// Optional callback invoked after block execution with full block data.
    on_block_executed: Option<OnBlockExecuted>,
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
            on_block_executed: None,
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
            on_block_executed: None,
        }
    }

    /// Set a callback for state changes.
    ///
    /// This callback is invoked whenever state changes are produced by execution.
    pub fn with_state_change_callback(mut self, callback: StateChangeCallback) -> Self {
        self.on_state_change = Some(callback);
        self
    }

    /// Set a callback for block execution.
    ///
    /// When set, this is called after `execute_txs` with the full block data
    /// (state changes, block result, transactions). The caller takes responsibility
    /// for persisting state changes and indexing the block.
    pub fn with_on_block_executed(mut self, callback: OnBlockExecuted) -> Self {
        self.on_block_executed = Some(callback);
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
        ts.map_or(0, |t| t.seconds as u64)
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

                // Select txs for the block proposal and mark them as in-flight.
                // Executed txs are confirmed via finalize in execute_txs;
                // unexecuted ones are returned to the priority queue.
                let (selected, _total_gas) = pool.propose(self.config.max_gas, 0);

                // Encode selected transactions
                selected
                    .iter()
                    .filter_map(|tx| match tx.encode() {
                        Ok(bytes) => Some(bytes),
                        Err(e) => {
                            tracing::warn!("Failed to encode tx: {:?}", e);
                            None
                        }
                    })
                    .collect()
            }
            None => vec![],
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
            .gas_limit(self.config.max_gas)
            .transactions(transactions)
            .build();

        // Execute through STF
        let (result, exec_state) = self.stf.execute_block(&self.storage, &self.codes, &block);

        let changes = exec_state
            .into_changes()
            .map_err(|e| Status::internal(format!("failed to get state changes: {:?}", e)))?;

        // Compute updated state root
        let updated_state_root = compute_state_root(&changes);

        // Log execution results (before moving ownership)
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

        // Finalize the block proposal: remove executed txs, return unexecuted in-flight txs.
        if let Some(ref mempool) = self.mempool {
            let tx_hashes: Vec<[u8; 32]> =
                block.transactions.iter().map(|tx| tx.hash().0).collect();
            if !tx_hashes.is_empty() {
                let mut pool = mempool.write().await;
                pool.finalize(&tx_hashes);
            }
        }

        // Handle state changes / notify callback
        if let Some(ref callback) = self.on_block_executed {
            let info = BlockExecutedInfo {
                height: req.block_height,
                timestamp,
                state_changes: changes,
                block_result: result,
                state_root: updated_state_root,
                transactions: block.transactions,
            };
            callback(info);
        } else {
            self.handle_state_changes(changes).await;
        }

        // Update state
        self.state
            .current_height
            .store(req.block_height, Ordering::SeqCst);
        *self.state.last_state_root.write().await = updated_state_root;

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

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use evolve_core::{InvokeResponse, Message};
    use evolve_mempool::shared_mempool_from;
    use evolve_stf::results::TxResult;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use tonic::Code;

    use crate::proto::evnode::v1::executor_service_server::ExecutorService;

    #[derive(Default)]
    struct MockStorage;

    impl ReadonlyKV for MockStorage {
        fn get(&self, _key: &[u8]) -> Result<Option<Vec<u8>>, evolve_core::ErrorCode> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct MockCodes;

    impl AccountsCodeStorage for MockCodes {
        fn with_code<F, R>(&self, _identifier: &str, f: F) -> Result<R, evolve_core::ErrorCode>
        where
            F: FnOnce(Option<&dyn evolve_core::AccountCode>) -> R,
        {
            Ok(f(None))
        }

        fn list_identifiers(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[derive(Debug, Clone)]
    struct ObservedBlock {
        number: u64,
        timestamp: u64,
        tx_count: usize,
        tx_hashes: Vec<B256>,
    }

    struct MockStf {
        emit_genesis_change: bool,
        run_genesis_calls: AtomicUsize,
        execute_changes: Vec<StateChange>,
        execute_block_calls: AtomicUsize,
        last_executed_block: Mutex<Option<ObservedBlock>>,
    }

    impl MockStf {
        fn new(emit_genesis_change: bool, execute_changes: Vec<StateChange>) -> Self {
            Self {
                emit_genesis_change,
                run_genesis_calls: AtomicUsize::new(0),
                execute_changes,
                execute_block_calls: AtomicUsize::new(0),
                last_executed_block: Mutex::new(None),
            }
        }
    }

    impl EvnodeStfExecutor<MockStorage, MockCodes> for MockStf {
        fn execute_block<'a>(
            &self,
            storage: &'a MockStorage,
            _codes: &'a MockCodes,
            block: &ExecutorBlock,
        ) -> (BlockResult, ExecutionState<'a, MockStorage>) {
            self.execute_block_calls.fetch_add(1, Ordering::SeqCst);

            let observed = ObservedBlock {
                number: block.number(),
                timestamp: block.timestamp(),
                tx_count: block.transactions.len(),
                tx_hashes: block.transactions.iter().map(|tx| tx.hash()).collect(),
            };
            *self
                .last_executed_block
                .lock()
                .expect("last_executed_block lock should not be poisoned") = Some(observed);

            let mut exec_state = ExecutionState::new(storage);
            for change in &self.execute_changes {
                match change {
                    StateChange::Set { key, value } => exec_state
                        .set(key, Message::from_bytes(value.clone()))
                        .expect("set change should be valid"),
                    StateChange::Remove { key } => exec_state
                        .remove(key)
                        .expect("remove change should be valid"),
                }
            }

            let tx_results: Vec<TxResult> = block
                .transactions
                .iter()
                .map(|tx| TxResult {
                    events: vec![],
                    gas_used: tx.envelope().gas_limit(),
                    response: Ok(InvokeResponse::new(&()).expect("unit response should encode")),
                })
                .collect();
            let gas_used = tx_results.iter().map(|r| r.gas_used).sum();

            (
                BlockResult {
                    begin_block_events: vec![],
                    tx_results,
                    end_block_events: vec![],
                    gas_used,
                    txs_skipped: 0,
                },
                exec_state,
            )
        }

        fn run_genesis<'a>(
            &self,
            _storage: &'a MockStorage,
            _codes: &'a MockCodes,
            _initial_height: u64,
            _genesis_time: u64,
        ) -> Result<(Vec<StateChange>, B256), EvnodeError> {
            self.run_genesis_calls.fetch_add(1, Ordering::SeqCst);
            let changes = if self.emit_genesis_change {
                vec![StateChange::Set {
                    key: b"k".to_vec(),
                    value: b"v".to_vec(),
                }]
            } else {
                Vec::new()
            };
            Ok((changes, B256::ZERO))
        }
    }

    fn mk_service(
        emit_genesis_change: bool,
    ) -> ExecutorServiceImpl<MockStf, MockStorage, MockCodes> {
        ExecutorServiceImpl::new(
            MockStf::new(emit_genesis_change, Vec::new()),
            MockStorage,
            MockCodes,
            ExecutorServiceConfig::default(),
        )
    }

    fn mk_service_with_execute_changes(
        execute_changes: Vec<StateChange>,
    ) -> ExecutorServiceImpl<MockStf, MockStorage, MockCodes> {
        ExecutorServiceImpl::new(
            MockStf::new(false, execute_changes),
            MockStorage,
            MockCodes,
            ExecutorServiceConfig::default(),
        )
    }

    async fn init_test_chain(service: &ExecutorServiceImpl<MockStf, MockStorage, MockCodes>) {
        service
            .init_chain(Request::new(InitChainRequest {
                genesis_time: None,
                initial_height: 1,
                chain_id: "chain-test".to_string(),
            }))
            .await
            .expect("init should succeed");
    }

    fn decode_hex(s: &str) -> Vec<u8> {
        assert!(
            s.len() % 2 == 0,
            "hex input length must be even, got {}",
            s.len()
        );
        let mut out = Vec::with_capacity(s.len() / 2);
        for idx in (0..s.len()).step_by(2) {
            let byte = u8::from_str_radix(&s[idx..idx + 2], 16)
                .expect("hex input should contain only valid hex characters");
            out.push(byte);
        }
        out
    }

    fn sample_legacy_tx_bytes() -> Vec<u8> {
        decode_hex(concat!(
            "f86c098504a817c800825208943535353535353535353535353535353535353535880de0",
            "b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590",
            "620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
        ))
    }

    #[tokio::test]
    async fn init_chain_rejects_zero_initial_height() {
        let service = mk_service(false);
        let req = InitChainRequest {
            genesis_time: None,
            initial_height: 0,
            chain_id: "test-chain".to_string(),
        };

        let err = service
            .init_chain(Request::new(req))
            .await
            .expect_err("init_chain must reject zero initial_height");

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().contains("initial_height must be > 0"));
    }

    #[tokio::test]
    async fn init_chain_sets_state_and_pending_changes() {
        let changes = vec![StateChange::Set {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        }];
        let expected_root = compute_state_root(&changes);
        let service = mk_service(true);
        let req = InitChainRequest {
            genesis_time: Some(Timestamp {
                seconds: 123,
                nanos: 0,
            }),
            initial_height: 7,
            chain_id: "chain-A".to_string(),
        };

        let resp = service
            .init_chain(Request::new(req))
            .await
            .expect("init_chain should succeed")
            .into_inner();

        assert_eq!(resp.state_root, expected_root.to_vec());
        assert!(service.state.initialized.load(Ordering::SeqCst));
        assert_eq!(service.state.initial_height.load(Ordering::SeqCst), 7);
        assert_eq!(service.state.current_height.load(Ordering::SeqCst), 7);
        assert_eq!(service.state.genesis_time.load(Ordering::SeqCst), 123);
        assert_eq!(*service.state.chain_id.read().await, "chain-A".to_string());
        assert_eq!(*service.state.last_state_root.read().await, expected_root);
        assert_eq!(service.state.pending_changes.read().await.len(), 1);
    }

    #[tokio::test]
    async fn init_chain_cannot_be_called_twice() {
        let service = mk_service(false);
        let req = InitChainRequest {
            genesis_time: None,
            initial_height: 1,
            chain_id: "test-chain".to_string(),
        };

        service
            .init_chain(Request::new(req.clone()))
            .await
            .expect("first init should succeed");
        let err = service
            .init_chain(Request::new(req))
            .await
            .expect_err("second init should fail");

        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().contains("chain already initialized"));
    }

    #[tokio::test]
    async fn set_final_requires_initialized_chain() {
        let service = mk_service(false);

        let err = service
            .set_final(Request::new(SetFinalRequest { block_height: 1 }))
            .await
            .expect_err("set_final should fail before initialization");

        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().contains("chain not initialized"));
    }

    #[tokio::test]
    async fn set_final_rejects_height_above_current() {
        let service = mk_service(false);
        service
            .init_chain(Request::new(InitChainRequest {
                genesis_time: None,
                initial_height: 5,
                chain_id: "chain-final".to_string(),
            }))
            .await
            .expect("init should succeed");

        let err = service
            .set_final(Request::new(SetFinalRequest { block_height: 6 }))
            .await
            .expect_err("set_final should reject higher-than-current height");

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err
            .message()
            .contains("cannot finalize height 6 > current height 5"));
    }

    #[tokio::test]
    async fn set_final_updates_finalized_height() {
        let service = mk_service(false);
        service
            .init_chain(Request::new(InitChainRequest {
                genesis_time: None,
                initial_height: 3,
                chain_id: "chain-final-ok".to_string(),
            }))
            .await
            .expect("init should succeed");

        service
            .set_final(Request::new(SetFinalRequest { block_height: 3 }))
            .await
            .expect("set_final should succeed");

        assert_eq!(service.state.finalized_height.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn filter_txs_marks_invalid_transactions_for_removal() {
        let service = mk_service(false);
        service
            .init_chain(Request::new(InitChainRequest {
                genesis_time: None,
                initial_height: 1,
                chain_id: "chain-filter".to_string(),
            }))
            .await
            .expect("init should succeed");

        let req = FilterTxsRequest {
            txs: vec![vec![0x01], vec![0x02, 0x03], vec![]],
            max_bytes: 0,
            max_gas: 0,
            has_force_included_transaction: false,
        };

        let resp = service
            .filter_txs(Request::new(req))
            .await
            .expect("filter_txs should succeed")
            .into_inner();

        assert_eq!(
            resp.statuses,
            vec![
                FilterStatus::FilterRemove as i32,
                FilterStatus::FilterRemove as i32,
                FilterStatus::FilterRemove as i32
            ]
        );
    }

    #[tokio::test]
    async fn execute_txs_updates_state_and_pending_changes_and_skips_invalid_input() {
        let expected_changes = vec![
            StateChange::Set {
                key: b"foo".to_vec(),
                value: b"bar".to_vec(),
            },
            StateChange::Remove {
                key: b"gone".to_vec(),
            },
        ];
        let expected_root = compute_state_root(&expected_changes);
        let service = mk_service_with_execute_changes(expected_changes);
        init_test_chain(&service).await;

        let valid_tx = sample_legacy_tx_bytes();
        let valid_tx_hash = TxContext::decode(&valid_tx)
            .expect("sample tx should decode")
            .hash();
        let req = ExecuteTxsRequest {
            block_height: 2,
            timestamp: Some(Timestamp {
                seconds: 777,
                nanos: 0,
            }),
            prev_state_root: vec![],
            txs: vec![vec![0x01, 0x02], valid_tx],
        };

        let resp = service
            .execute_txs(Request::new(req))
            .await
            .expect("execute_txs should succeed")
            .into_inner();

        assert_eq!(resp.updated_state_root, expected_root.to_vec());
        assert_eq!(
            service.state.current_height.load(Ordering::SeqCst),
            2,
            "block height should update after execution"
        );
        assert_eq!(*service.state.last_state_root.read().await, expected_root);
        assert_eq!(
            service.state.pending_changes.read().await.len(),
            2,
            "execute changes should be persisted when no block callback is set"
        );

        let observed = service
            .stf
            .last_executed_block
            .lock()
            .expect("last_executed_block lock should not be poisoned")
            .clone()
            .expect("execute_block should have been called");
        assert_eq!(observed.number, 2);
        assert_eq!(observed.timestamp, 777);
        assert_eq!(observed.tx_count, 1, "invalid tx bytes should be skipped");
        assert_eq!(observed.tx_hashes, vec![valid_tx_hash]);
    }

    #[tokio::test]
    async fn execute_txs_prefers_block_callback_over_state_change_callback() {
        let execute_changes = vec![StateChange::Set {
            key: b"k1".to_vec(),
            value: b"v1".to_vec(),
        }];
        let state_change_calls = Arc::new(AtomicUsize::new(0));
        let block_callback_calls = Arc::new(AtomicUsize::new(0));

        let service = mk_service_with_execute_changes(execute_changes)
            .with_state_change_callback({
                let calls = Arc::clone(&state_change_calls);
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                })
            })
            .with_on_block_executed({
                let calls = Arc::clone(&block_callback_calls);
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                })
            });
        init_test_chain(&service).await;

        let req = ExecuteTxsRequest {
            block_height: 2,
            timestamp: None,
            prev_state_root: vec![],
            txs: vec![sample_legacy_tx_bytes()],
        };
        let state_calls_before_execute = state_change_calls.load(Ordering::SeqCst);
        service
            .execute_txs(Request::new(req))
            .await
            .expect("execute_txs should succeed");

        assert_eq!(
            state_change_calls.load(Ordering::SeqCst),
            state_calls_before_execute,
            "state change callback should be bypassed when block callback is set"
        );
        assert_eq!(block_callback_calls.load(Ordering::SeqCst), 1);
        assert!(
            service.state.pending_changes.read().await.is_empty(),
            "pending changes should not be accumulated when block callback handles execution output"
        );
    }

    #[tokio::test]
    async fn execute_txs_block_callback_receives_expected_payload() {
        let execute_changes = vec![StateChange::Set {
            key: b"payload".to_vec(),
            value: b"ok".to_vec(),
        }];
        let expected_root = compute_state_root(&execute_changes);
        let callback_snapshot = Arc::new(Mutex::new(None::<(u64, u64, usize, usize, B256)>));
        let service = mk_service_with_execute_changes(execute_changes).with_on_block_executed({
            let snapshot = Arc::clone(&callback_snapshot);
            Arc::new(move |info| {
                *snapshot
                    .lock()
                    .expect("callback snapshot lock should not be poisoned") = Some((
                    info.height,
                    info.timestamp,
                    info.transactions.len(),
                    info.block_result.tx_results.len(),
                    info.state_root,
                ));
            })
        });
        init_test_chain(&service).await;

        service
            .execute_txs(Request::new(ExecuteTxsRequest {
                block_height: 3,
                timestamp: Some(Timestamp {
                    seconds: 456,
                    nanos: 0,
                }),
                prev_state_root: vec![],
                txs: vec![sample_legacy_tx_bytes()],
            }))
            .await
            .expect("execute_txs should succeed");

        let snapshot = callback_snapshot
            .lock()
            .expect("callback snapshot lock should not be poisoned")
            .expect("block callback should have been invoked");
        assert_eq!(snapshot.0, 3);
        assert_eq!(snapshot.1, 456);
        assert_eq!(snapshot.2, 1);
        assert_eq!(snapshot.3, 1);
        assert_eq!(snapshot.4, expected_root);
    }

    #[tokio::test]
    async fn get_txs_filter_limits_and_finalize_interact_consistently() {
        let tx_bytes = sample_legacy_tx_bytes();
        let tx = TxContext::decode(&tx_bytes).expect("sample tx should decode");
        let tx_gas = tx.envelope().gas_limit();
        let tx_len = tx_bytes.len() as u64;

        let mut pool = Mempool::<TxContext>::new();
        pool.add(tx).expect("tx should be added to mempool");
        let mempool = shared_mempool_from(pool);

        let service = ExecutorServiceImpl::with_mempool(
            MockStf::new(false, Vec::new()),
            MockStorage,
            MockCodes,
            ExecutorServiceConfig {
                max_gas: tx_gas,
                max_bytes: DEFAULT_MAX_BYTES,
            },
            mempool,
        );
        init_test_chain(&service).await;

        let proposed = service
            .get_txs(Request::new(GetTxsRequest {}))
            .await
            .expect("get_txs should succeed")
            .into_inner()
            .txs;
        assert_eq!(proposed.len(), 1, "one tx should be proposed from mempool");

        let filter_resp = service
            .filter_txs(Request::new(FilterTxsRequest {
                txs: vec![proposed[0].clone(), proposed[0].clone()],
                max_bytes: tx_len,
                max_gas: tx_gas,
                has_force_included_transaction: false,
            }))
            .await
            .expect("filter_txs should succeed")
            .into_inner();
        assert_eq!(
            filter_resp.statuses,
            vec![
                FilterStatus::FilterOk as i32,
                FilterStatus::FilterPostpone as i32
            ],
            "second tx should be postponed when cumulative bytes/gas exceed limits"
        );

        service
            .execute_txs(Request::new(ExecuteTxsRequest {
                block_height: 2,
                timestamp: None,
                prev_state_root: vec![],
                txs: proposed,
            }))
            .await
            .expect("execute_txs should succeed");

        let after_finalize = service
            .get_txs(Request::new(GetTxsRequest {}))
            .await
            .expect("second get_txs should succeed")
            .into_inner();
        assert!(
            after_finalize.txs.is_empty(),
            "executed tx should be finalized out of mempool"
        );
    }
}
