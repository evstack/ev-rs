//! StateProvider implementation using ChainIndex.
//!
//! This module provides `ChainStateProvider` which implements the `StateProvider`
//! trait from `evolve_eth_jsonrpc` using:
//! - `ChainIndex` for block/transaction/receipt queries
//! - `Storage` for state queries (balance, nonce, code)
//! - `Stf` for call/estimateGas execution
//! - `EthGateway` + `Mempool` for transaction submission (optional)

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, Bytes, B256, U256};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;

use crate::error::ChainIndexError;
use crate::index::ChainIndex;
use crate::querier::StateQuerier;
use evolve_core::schema::AccountSchema;
use evolve_core::AccountCode;
use evolve_eth_jsonrpc::error::RpcError;
use evolve_eth_jsonrpc::StateProvider;
use evolve_mempool::{Mempool, MempoolTx, SharedMempool};
use evolve_rpc_types::block::BlockTransactions;
use evolve_rpc_types::SyncStatus;
use evolve_rpc_types::{CallRequest, LogFilter, RpcBlock, RpcLog, RpcReceipt, RpcTransaction};
use evolve_stf_traits::AccountsCodeStorage;
use evolve_tx_eth::{EthGateway, TxContext};

struct VerifyJob {
    raw: Vec<u8>,
    response: oneshot::Sender<Result<TxContext, String>>,
}

struct IngressVerifier {
    queue: mpsc::Sender<VerifyJob>,
    response_timeout: Duration,
}

impl IngressVerifier {
    fn new(
        gateway: Arc<EthGateway>,
        workers: usize,
        queue_capacity: usize,
        response_timeout_ms: u64,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<VerifyJob>(queue_capacity);
        let shared_rx = Arc::new(Mutex::new(rx));

        for _ in 0..workers {
            let rx = Arc::clone(&shared_rx);
            let gateway = Arc::clone(&gateway);
            tokio::spawn(async move {
                loop {
                    let maybe_job = {
                        let mut guard = rx.lock().await;
                        guard.recv().await
                    };
                    let Some(job) = maybe_job else { break };

                    let raw = job.raw;
                    let result = match tokio::task::spawn_blocking({
                        let gateway = Arc::clone(&gateway);
                        move || gateway.decode_and_verify(&raw).map_err(|e| e.to_string())
                    })
                    .await
                    {
                        Ok(res) => res,
                        Err(e) => Err(format!("verify task failed: {e}")),
                    };

                    let _ = job.response.send(result);
                }
            });
        }

        Arc::new(Self {
            queue: tx,
            response_timeout: Duration::from_millis(response_timeout_ms),
        })
    }

    async fn verify(&self, raw: Vec<u8>) -> Result<TxContext, RpcError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.queue
            .send(VerifyJob {
                raw,
                response: response_tx,
            })
            .await
            .map_err(|_| RpcError::InternalError("verifier queue closed".to_string()))?;

        let result = timeout(self.response_timeout, response_rx)
            .await
            .map_err(|_| RpcError::InternalError("verifier timeout".to_string()))?
            .map_err(|_| RpcError::InternalError("verifier response dropped".to_string()))?;

        result.map_err(RpcError::InvalidTransaction)
    }
}

/// State provider configuration.
#[derive(Debug, Clone)]
pub struct ChainStateProviderConfig {
    /// Chain ID.
    pub chain_id: u64,
    /// Protocol version string.
    pub protocol_version: String,
    /// Gas price in wei.
    pub gas_price: U256,
    /// Sync status response.
    pub sync_status: SyncStatus,
}

/// State provider that combines chain index with state storage.
///
/// This is the main integration point between the RPC server and the chain data.
pub struct ChainStateProvider<
    I: ChainIndex,
    A: AccountsCodeStorage + Send + Sync = NoopAccountCodes,
> {
    /// Chain index for blocks/transactions/receipts.
    index: Arc<I>,
    /// Configuration.
    config: ChainStateProviderConfig,
    /// Account codes storage for schema introspection.
    account_codes: Arc<A>,
    /// Optional mempool for transaction submission.
    mempool: Option<SharedMempool<Mempool<TxContext>>>,
    /// Optional ingress verifier for transaction decode/verify.
    verifier: Option<Arc<IngressVerifier>>,
    /// Optional state querier for balance/nonce/call queries.
    state_querier: Option<Arc<dyn StateQuerier>>,
    /// Cached module identifiers for schema introspection endpoints.
    module_ids_cache: parking_lot::RwLock<Option<Vec<String>>>,
    /// Cached per-module schema lookups.
    module_schema_cache: parking_lot::RwLock<BTreeMap<String, Option<AccountSchema>>>,
    /// Cached list of all module schemas.
    all_schemas_cache: parking_lot::RwLock<Option<Vec<AccountSchema>>>,
}

/// No-op implementation of AccountsCodeStorage for when schema introspection is not needed.
#[derive(Default)]
pub struct NoopAccountCodes;

impl AccountsCodeStorage for NoopAccountCodes {
    fn with_code<F, R>(&self, _identifier: &str, f: F) -> Result<R, evolve_core::ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R,
    {
        Ok(f(None))
    }

    fn list_identifiers(&self) -> Vec<String> {
        Vec::new()
    }
}

impl<I: ChainIndex> ChainStateProvider<I, NoopAccountCodes> {
    /// Create a new chain state provider without account codes.
    pub fn new(index: Arc<I>, config: ChainStateProviderConfig) -> Self {
        Self {
            index,
            config,
            account_codes: Arc::new(NoopAccountCodes),
            mempool: None,
            verifier: None,
            state_querier: None,
            module_ids_cache: parking_lot::RwLock::new(None),
            module_schema_cache: parking_lot::RwLock::new(BTreeMap::new()),
            all_schemas_cache: parking_lot::RwLock::new(None),
        }
    }
}

impl<I: ChainIndex, A: AccountsCodeStorage + Send + Sync> ChainStateProvider<I, A> {
    /// Create a new chain state provider with account codes for schema introspection.
    pub fn with_account_codes(
        index: Arc<I>,
        config: ChainStateProviderConfig,
        account_codes: Arc<A>,
    ) -> Self {
        Self {
            index,
            config,
            account_codes,
            mempool: None,
            verifier: None,
            state_querier: None,
            module_ids_cache: parking_lot::RwLock::new(None),
            module_schema_cache: parking_lot::RwLock::new(BTreeMap::new()),
            all_schemas_cache: parking_lot::RwLock::new(None),
        }
    }

    /// Create a new chain state provider with account codes and mempool.
    ///
    /// When a mempool is provided, `send_raw_transaction` will decode and
    /// add transactions to the mempool for block inclusion.
    pub fn with_mempool(
        index: Arc<I>,
        config: ChainStateProviderConfig,
        account_codes: Arc<A>,
        mempool: SharedMempool<Mempool<TxContext>>,
    ) -> Self {
        let default_parallelism = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let configured_parallelism = std::env::var("EVOLVE_RPC_VERIFY_PARALLELISM")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(default_parallelism);
        let gateway = Arc::new(EthGateway::new(config.chain_id));
        let queue_capacity = std::env::var("EVOLVE_RPC_VERIFY_QUEUE_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(4096);
        let response_timeout_ms = std::env::var("EVOLVE_RPC_VERIFY_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(10_000);
        let verifier = IngressVerifier::new(
            gateway,
            configured_parallelism,
            queue_capacity,
            response_timeout_ms,
        );
        tracing::info!(
            "RPC ingress verification: workers={}, queue_capacity={}, timeout_ms={}",
            configured_parallelism,
            queue_capacity,
            response_timeout_ms
        );
        Self {
            index,
            config,
            account_codes,
            mempool: Some(mempool),
            verifier: Some(verifier),
            state_querier: None,
            module_ids_cache: parking_lot::RwLock::new(None),
            module_schema_cache: parking_lot::RwLock::new(BTreeMap::new()),
            all_schemas_cache: parking_lot::RwLock::new(None),
        }
    }

    /// Get the chain index.
    pub fn index(&self) -> &Arc<I> {
        &self.index
    }

    /// Get the configuration.
    pub fn config(&self) -> &ChainStateProviderConfig {
        &self.config
    }

    /// Get the account codes storage.
    pub fn account_codes(&self) -> &Arc<A> {
        &self.account_codes
    }

    /// Get the mempool, if present.
    pub fn mempool(&self) -> Option<&SharedMempool<Mempool<TxContext>>> {
        self.mempool.as_ref()
    }

    /// Attach a state querier for balance/nonce/call queries.
    pub fn with_state_querier(mut self, querier: Arc<dyn StateQuerier>) -> Self {
        self.state_querier = Some(querier);
        self
    }
}

impl From<ChainIndexError> for RpcError {
    fn from(err: ChainIndexError) -> Self {
        match err {
            ChainIndexError::BlockNotFound(_) => RpcError::BlockNotFound,
            ChainIndexError::BlockHashNotFound(_) => RpcError::BlockNotFound,
            ChainIndexError::TransactionNotFound(_) => RpcError::TransactionNotFound,
            ChainIndexError::ReceiptNotFound(_) => RpcError::ReceiptNotFound,
            ChainIndexError::EmptyIndex => RpcError::BlockNotFound,
            _ => RpcError::InternalError(err.to_string()),
        }
    }
}

#[async_trait]
impl<I: ChainIndex + 'static, A: AccountsCodeStorage + Send + Sync + 'static> StateProvider
    for ChainStateProvider<I, A>
{
    async fn block_number(&self) -> Result<u64, RpcError> {
        self.index
            .latest_block_number()
            .map_err(|e| e.into())
            .and_then(|opt| opt.ok_or(RpcError::BlockNotFound))
    }

    async fn get_block_by_number(
        &self,
        number: u64,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError> {
        let block = match self.index.get_block(number)? {
            Some(b) => b,
            None => return Ok(None),
        };

        let transactions = if full_transactions {
            // Get full transaction objects
            let tx_hashes = self.index.get_block_transactions(number)?;
            let mut txs = Vec::with_capacity(tx_hashes.len());
            for hash in tx_hashes {
                if let Some(tx) = self.index.get_transaction(hash)? {
                    txs.push(tx.to_rpc_transaction());
                }
            }
            Some(BlockTransactions::Full(txs))
        } else {
            // Just transaction hashes
            let tx_hashes = self.index.get_block_transactions(number)?;
            Some(BlockTransactions::Hashes(tx_hashes))
        };

        Ok(Some(block.to_rpc_block(transactions)))
    }

    async fn get_block_by_hash(
        &self,
        hash: B256,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError> {
        let block = match self.index.get_block_by_hash(hash)? {
            Some(b) => b,
            None => return Ok(None),
        };

        let number = block.number;
        let transactions = if full_transactions {
            let tx_hashes = self.index.get_block_transactions(number)?;
            let mut txs = Vec::with_capacity(tx_hashes.len());
            for h in tx_hashes {
                if let Some(tx) = self.index.get_transaction(h)? {
                    txs.push(tx.to_rpc_transaction());
                }
            }
            Some(BlockTransactions::Full(txs))
        } else {
            let tx_hashes = self.index.get_block_transactions(number)?;
            Some(BlockTransactions::Hashes(tx_hashes))
        };

        Ok(Some(block.to_rpc_block(transactions)))
    }

    async fn get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<RpcTransaction>, RpcError> {
        match self.index.get_transaction(hash)? {
            Some(tx) => Ok(Some(tx.to_rpc_transaction())),
            None => Ok(None),
        }
    }

    async fn get_transaction_receipt(&self, hash: B256) -> Result<Option<RpcReceipt>, RpcError> {
        match self.index.get_receipt(hash)? {
            Some(receipt) => Ok(Some(receipt.to_rpc_receipt())),
            None => Ok(None),
        }
    }

    async fn get_balance(&self, address: Address, _block: Option<u64>) -> Result<U256, RpcError> {
        let querier = self
            .state_querier
            .as_ref()
            .ok_or_else(|| RpcError::NotImplemented("state_querier not configured".to_string()))?;
        querier.get_balance(address).await
    }

    async fn get_transaction_count(
        &self,
        address: Address,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        let querier = self
            .state_querier
            .as_ref()
            .ok_or_else(|| RpcError::NotImplemented("state_querier not configured".to_string()))?;
        querier.get_transaction_count(address).await
    }

    async fn call(&self, request: &CallRequest, _block: Option<u64>) -> Result<Bytes, RpcError> {
        let querier = self
            .state_querier
            .as_ref()
            .ok_or_else(|| RpcError::NotImplemented("state_querier not configured".to_string()))?;
        querier.call(request).await
    }

    async fn estimate_gas(
        &self,
        request: &CallRequest,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        let querier = self
            .state_querier
            .as_ref()
            .ok_or_else(|| RpcError::NotImplemented("state_querier not configured".to_string()))?;
        querier.estimate_gas(request).await
    }

    async fn get_logs(&self, filter: &LogFilter) -> Result<Vec<RpcLog>, RpcError> {
        use evolve_rpc_types::{BlockNumberOrTag, BlockTag};

        // Determine block range
        let latest = self.block_number().await.unwrap_or(0);

        let from_block = match &filter.from_block {
            Some(BlockNumberOrTag::Number(n)) => n.to::<u64>(),
            Some(BlockNumberOrTag::Tag(BlockTag::Earliest)) => 0,
            Some(BlockNumberOrTag::Tag(_)) => latest,
            None => latest,
        };

        let to_block = match &filter.to_block {
            Some(BlockNumberOrTag::Number(n)) => n.to::<u64>(),
            Some(BlockNumberOrTag::Tag(BlockTag::Earliest)) => 0,
            Some(BlockNumberOrTag::Tag(_)) => latest,
            None => latest,
        };

        // Handle block_hash filter (mutually exclusive with from/to)
        if let Some(block_hash) = filter.block_hash {
            if let Some(number) = self.index.get_block_number(block_hash)? {
                return self.get_logs_for_block_range(number, number, filter);
            } else {
                return Ok(vec![]);
            }
        }

        self.get_logs_for_block_range(from_block, to_block, filter)
    }

    async fn send_raw_transaction(&self, data: &[u8]) -> Result<B256, RpcError> {
        // Check if mempool and gateway are available
        let mempool = match &self.mempool {
            Some(m) => m,
            None => {
                return Err(RpcError::NotImplemented(
                    "sendRawTransaction: no mempool configured".to_string(),
                ))
            }
        };

        let verifier = match &self.verifier {
            Some(v) => v,
            None => {
                return Err(RpcError::NotImplemented(
                    "sendRawTransaction: no verifier configured".to_string(),
                ))
            }
        };

        let tx_context = verifier.verify(data.to_vec()).await?;

        // Get the transaction hash before adding to mempool
        let hash = B256::from(tx_context.tx_id());

        // Add to mempool
        mempool
            .write()
            .await
            .add(tx_context)
            .map_err(|e| RpcError::InvalidTransaction(e.to_string()))?;

        tracing::info!("Transaction submitted to mempool: {}", hash);

        Ok(hash)
    }

    async fn get_code(&self, _address: Address, _block: Option<u64>) -> Result<Bytes, RpcError> {
        // TODO: Implement via Storage
        Ok(Bytes::new())
    }

    async fn get_storage_at(
        &self,
        _address: Address,
        _position: U256,
        _block: Option<u64>,
    ) -> Result<B256, RpcError> {
        // TODO: Implement via Storage
        Ok(B256::ZERO)
    }

    async fn list_module_identifiers(&self) -> Result<Vec<String>, RpcError> {
        if let Some(cached) = self.module_ids_cache.read().as_ref() {
            return Ok(cached.clone());
        }

        let identifiers = self.account_codes.list_identifiers();
        *self.module_ids_cache.write() = Some(identifiers.clone());
        Ok(identifiers)
    }

    async fn get_module_schema(&self, id: &str) -> Result<Option<AccountSchema>, RpcError> {
        if let Some(cached) = self.module_schema_cache.read().get(id) {
            return Ok(cached.clone());
        }

        self.account_codes
            .with_code(id, |code| code.map(|c| c.schema()))
            .map_err(|e| RpcError::InternalError(format!("Failed to get schema: {:?}", e)))
            .inspect(|schema| {
                self.module_schema_cache
                    .write()
                    .insert(id.to_string(), schema.clone());
            })
    }

    async fn get_all_schemas(&self) -> Result<Vec<AccountSchema>, RpcError> {
        if let Some(cached) = self.all_schemas_cache.read().as_ref() {
            return Ok(cached.clone());
        }

        let identifiers = if let Some(cached) = self.module_ids_cache.read().as_ref() {
            cached.clone()
        } else {
            let ids = self.account_codes.list_identifiers();
            *self.module_ids_cache.write() = Some(ids.clone());
            ids
        };

        let mut schemas = Vec::with_capacity(identifiers.len());
        for id in identifiers {
            let maybe_schema = if let Some(cached) = self.module_schema_cache.read().get(&id) {
                cached.clone()
            } else {
                let schema = self
                    .account_codes
                    .with_code(&id, |code| code.map(|c| c.schema()))
                    .map_err(|e| {
                        RpcError::InternalError(format!("Failed to get schema: {:?}", e))
                    })?;
                self.module_schema_cache
                    .write()
                    .insert(id.clone(), schema.clone());
                schema
            };

            if let Some(schema) = maybe_schema {
                schemas.push(schema);
            }
        }

        *self.all_schemas_cache.write() = Some(schemas.clone());
        Ok(schemas)
    }

    async fn protocol_version(&self) -> Result<String, RpcError> {
        Ok(self.config.protocol_version.clone())
    }

    async fn gas_price(&self) -> Result<U256, RpcError> {
        Ok(self.config.gas_price)
    }

    async fn syncing_status(&self) -> Result<SyncStatus, RpcError> {
        Ok(self.config.sync_status.clone())
    }
}

impl<I: ChainIndex, A: AccountsCodeStorage + Send + Sync> ChainStateProvider<I, A> {
    /// Get logs for a block range with filtering.
    fn get_logs_for_block_range(
        &self,
        from_block: u64,
        to_block: u64,
        filter: &LogFilter,
    ) -> Result<Vec<RpcLog>, RpcError> {
        let mut result = Vec::new();

        // Limit range to prevent DoS
        let max_blocks = 1000u64;
        let actual_to = to_block.min(from_block.saturating_add(max_blocks));

        for block_num in from_block..=actual_to {
            // Get block for hash
            let block = match self.index.get_block(block_num)? {
                Some(b) => b,
                None => continue,
            };

            // Get receipts for this block
            let tx_hashes = self.index.get_block_transactions(block_num)?;

            let mut log_index = 0u64;
            for (tx_idx, tx_hash) in tx_hashes.iter().enumerate() {
                if let Some(receipt) = self.index.get_receipt(*tx_hash)? {
                    for stored_log in &receipt.logs {
                        // Apply address filter
                        if let Some(ref addr_filter) = filter.address {
                            let matches = match addr_filter {
                                evolve_rpc_types::FilterAddress::Single(addr) => {
                                    stored_log.address == *addr
                                }
                                evolve_rpc_types::FilterAddress::Multiple(addrs) => {
                                    addrs.contains(&stored_log.address)
                                }
                            };
                            if !matches {
                                log_index += 1;
                                continue;
                            }
                        }

                        // Apply topic filters
                        if let Some(ref topics) = filter.topics {
                            let mut matches = true;
                            for (i, topic_filter) in topics.iter().enumerate() {
                                if let Some(tf) = topic_filter {
                                    if i >= stored_log.topics.len() {
                                        matches = false;
                                        break;
                                    }
                                    let topic_matches = match tf {
                                        evolve_rpc_types::FilterTopic::Single(t) => {
                                            stored_log.topics[i] == *t
                                        }
                                        evolve_rpc_types::FilterTopic::Multiple(ts) => {
                                            ts.contains(&stored_log.topics[i])
                                        }
                                    };
                                    if !topic_matches {
                                        matches = false;
                                        break;
                                    }
                                }
                            }
                            if !matches {
                                log_index += 1;
                                continue;
                            }
                        }

                        result.push(stored_log.to_rpc_log(
                            block_num,
                            block.hash,
                            *tx_hash,
                            tx_idx as u32,
                            log_index,
                        ));
                        log_index += 1;
                    }
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::types::{StoredBlock, StoredLog, StoredReceipt, StoredTransaction, TxLocation};
    use evolve_mempool::new_shared_mempool;
    use evolve_rpc_types::block::BlockTransactions;

    #[derive(Default)]
    struct MockChainIndex {
        latest: Option<u64>,
        blocks_by_number: BTreeMap<u64, StoredBlock>,
        blocks_by_hash: BTreeMap<B256, StoredBlock>,
        block_number_by_hash: BTreeMap<B256, u64>,
        block_transactions: BTreeMap<u64, Vec<B256>>,
        transactions: BTreeMap<B256, StoredTransaction>,
        receipts: BTreeMap<B256, StoredReceipt>,
        fail_latest_block_number: Mutex<Option<ChainIndexError>>,
        fail_get_block: Mutex<Option<ChainIndexError>>,
        fail_get_block_by_hash: Mutex<Option<ChainIndexError>>,
        fail_get_block_transactions: Mutex<Option<ChainIndexError>>,
        fail_get_transaction: Mutex<Option<ChainIndexError>>,
        fail_get_receipt: Mutex<Option<ChainIndexError>>,
    }

    impl MockChainIndex {
        fn take_failure(slot: &Mutex<Option<ChainIndexError>>) -> Option<ChainIndexError> {
            slot.lock()
                .expect("failure lock should not be poisoned")
                .take()
        }
    }

    impl ChainIndex for MockChainIndex {
        fn latest_block_number(&self) -> Result<Option<u64>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_latest_block_number) {
                return Err(err);
            }
            Ok(self.latest)
        }

        fn get_block(&self, number: u64) -> Result<Option<StoredBlock>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_get_block) {
                return Err(err);
            }
            Ok(self.blocks_by_number.get(&number).cloned())
        }

        fn get_block_by_hash(&self, hash: B256) -> Result<Option<StoredBlock>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_get_block_by_hash) {
                return Err(err);
            }
            Ok(self.blocks_by_hash.get(&hash).cloned())
        }

        fn get_block_number(&self, hash: B256) -> Result<Option<u64>, ChainIndexError> {
            Ok(self.block_number_by_hash.get(&hash).copied())
        }

        fn get_block_transactions(&self, number: u64) -> Result<Vec<B256>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_get_block_transactions) {
                return Err(err);
            }
            Ok(self
                .block_transactions
                .get(&number)
                .cloned()
                .unwrap_or_default())
        }

        fn get_transaction(
            &self,
            hash: B256,
        ) -> Result<Option<StoredTransaction>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_get_transaction) {
                return Err(err);
            }
            Ok(self.transactions.get(&hash).cloned())
        }

        fn get_transaction_location(
            &self,
            _hash: B256,
        ) -> Result<Option<TxLocation>, ChainIndexError> {
            Ok(None)
        }

        fn get_receipt(&self, hash: B256) -> Result<Option<StoredReceipt>, ChainIndexError> {
            if let Some(err) = Self::take_failure(&self.fail_get_receipt) {
                return Err(err);
            }
            Ok(self.receipts.get(&hash).cloned())
        }

        fn get_logs_by_block(&self, _number: u64) -> Result<Vec<StoredLog>, ChainIndexError> {
            Ok(vec![])
        }

        fn store_block(
            &self,
            _block: StoredBlock,
            _transactions: Vec<StoredTransaction>,
            _receipts: Vec<StoredReceipt>,
        ) -> Result<(), ChainIndexError> {
            Ok(())
        }
    }

    fn provider_config() -> ChainStateProviderConfig {
        ChainStateProviderConfig {
            chain_id: 1,
            protocol_version: "test".to_string(),
            gas_price: U256::from(1u64),
            sync_status: SyncStatus::NotSyncing(false),
        }
    }

    fn default_provider(
        index: Arc<MockChainIndex>,
    ) -> ChainStateProvider<MockChainIndex, NoopAccountCodes> {
        ChainStateProvider::new(index, provider_config())
    }

    fn block(number: u64, hash: B256) -> StoredBlock {
        StoredBlock {
            number,
            hash,
            parent_hash: B256::repeat_byte(0x10),
            state_root: B256::repeat_byte(0x11),
            transactions_root: B256::repeat_byte(0x12),
            receipts_root: B256::repeat_byte(0x13),
            timestamp: 1_700_000_000,
            gas_used: 42_000,
            gas_limit: 30_000_000,
            transaction_count: 2,
            miner: Address::repeat_byte(0x22),
            extra_data: Bytes::new(),
        }
    }

    fn transaction(
        hash: B256,
        block_number: u64,
        block_hash: B256,
        transaction_index: u32,
    ) -> StoredTransaction {
        StoredTransaction {
            hash,
            block_number,
            block_hash,
            transaction_index,
            from: Address::repeat_byte(0x31),
            to: Some(Address::repeat_byte(0x32)),
            value: U256::from(100u64),
            gas: 21_000,
            gas_price: U256::from(1u64),
            input: Bytes::new(),
            nonce: 7,
            v: 27,
            r: U256::from(1u64),
            s: U256::from(2u64),
            tx_type: 0,
            chain_id: Some(1),
        }
    }

    fn receipt(
        transaction_hash: B256,
        block_number: u64,
        block_hash: B256,
        transaction_index: u32,
    ) -> StoredReceipt {
        StoredReceipt {
            transaction_hash,
            transaction_index,
            block_hash,
            block_number,
            from: Address::repeat_byte(0x41),
            to: Some(Address::repeat_byte(0x42)),
            cumulative_gas_used: 21_000,
            gas_used: 21_000,
            contract_address: None,
            logs: vec![],
            status: 1,
            tx_type: 0,
        }
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        fn nibble(b: u8) -> u8 {
            match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => panic!("invalid hex byte: {b}"),
            }
        }

        assert_eq!(input.len() % 2, 0, "hex input must have even length");
        let mut out = Vec::with_capacity(input.len() / 2);
        for pair in input.as_bytes().chunks_exact(2) {
            out.push((nibble(pair[0]) << 4) | nibble(pair[1]));
        }
        out
    }

    const LEGACY_TX_RLP: &str = concat!(
        "f86c098504a817c800825208943535353535353535353535353535353535353535880de0",
        "b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590",
        "620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
    );

    #[tokio::test]
    async fn send_raw_transaction_without_mempool_is_not_implemented() {
        let provider = default_provider(Arc::new(MockChainIndex::default()));

        let error = provider
            .send_raw_transaction(&[0x01, 0x02, 0x03])
            .await
            .expect_err("provider without mempool should reject submission");

        assert!(matches!(
            error,
            RpcError::NotImplemented(message)
            if message.contains("no mempool configured")
        ));
    }

    #[tokio::test]
    async fn send_raw_transaction_valid_payload_enters_mempool() {
        let mempool = new_shared_mempool::<TxContext>();
        let provider = ChainStateProvider::with_mempool(
            Arc::new(MockChainIndex::default()),
            provider_config(),
            Arc::new(NoopAccountCodes),
            mempool.clone(),
        );

        let raw = decode_hex(LEGACY_TX_RLP);
        let hash = provider
            .send_raw_transaction(&raw)
            .await
            .expect("valid signed transaction should be accepted");

        assert_eq!(
            hash,
            "0x33469b22e9f636356c4160a87eb19df52b7412e8eac32a4a55ffe88ea8350788"
                .parse::<B256>()
                .expect("hash literal should parse")
        );
        assert_eq!(mempool.read().await.len(), 1);
        assert!(mempool.read().await.contains(&hash.0));
    }

    #[tokio::test]
    async fn send_raw_transaction_invalid_payload_maps_to_invalid_transaction() {
        let mempool = new_shared_mempool::<TxContext>();
        let provider = ChainStateProvider::with_mempool(
            Arc::new(MockChainIndex::default()),
            provider_config(),
            Arc::new(NoopAccountCodes),
            mempool,
        );

        let error = provider
            .send_raw_transaction(&[0xFF, 0xFF, 0xFF])
            .await
            .expect_err("invalid bytes should fail ingress verification");

        assert!(matches!(
            error,
            RpcError::InvalidTransaction(message) if !message.is_empty()
        ));
    }

    #[tokio::test]
    async fn send_raw_transaction_duplicate_maps_to_invalid_transaction() {
        let mempool = new_shared_mempool::<TxContext>();
        let provider = ChainStateProvider::with_mempool(
            Arc::new(MockChainIndex::default()),
            provider_config(),
            Arc::new(NoopAccountCodes),
            mempool,
        );
        let raw = decode_hex(LEGACY_TX_RLP);

        provider
            .send_raw_transaction(&raw)
            .await
            .expect("first insertion should succeed");
        let error = provider
            .send_raw_transaction(&raw)
            .await
            .expect_err("duplicate insertion should fail");

        assert!(matches!(
            error,
            RpcError::InvalidTransaction(message) if message.contains("already in mempool")
        ));
    }

    #[tokio::test]
    async fn get_block_queries_preserve_hashes_and_skip_missing_full_transactions() {
        let block_hash = B256::repeat_byte(0x55);
        let tx_hash_present = B256::repeat_byte(0x61);
        let tx_hash_missing = B256::repeat_byte(0x62);
        let mut index = MockChainIndex {
            latest: Some(7),
            ..Default::default()
        };
        index.blocks_by_number.insert(7, block(7, block_hash));
        index
            .blocks_by_hash
            .insert(block_hash, block(7, block_hash));
        index
            .block_transactions
            .insert(7, vec![tx_hash_present, tx_hash_missing]);
        index.transactions.insert(
            tx_hash_present,
            transaction(tx_hash_present, 7, block_hash, 0),
        );
        let provider = default_provider(Arc::new(index));

        let by_number_hashes = provider
            .get_block_by_number(7, false)
            .await
            .expect("hash-only block query should succeed")
            .expect("block should exist");
        let by_number_full = provider
            .get_block_by_number(7, true)
            .await
            .expect("full block query should succeed")
            .expect("block should exist");
        let by_hash_full = provider
            .get_block_by_hash(block_hash, true)
            .await
            .expect("block-by-hash query should succeed")
            .expect("block should exist");

        match by_number_hashes.transactions {
            Some(BlockTransactions::Hashes(hashes)) => {
                assert_eq!(hashes, vec![tx_hash_present, tx_hash_missing]);
            }
            _ => panic!("expected hash transactions"),
        }

        match by_number_full.transactions {
            Some(BlockTransactions::Full(txs)) => {
                assert_eq!(txs.len(), 1);
                assert_eq!(txs[0].hash, tx_hash_present);
            }
            _ => panic!("expected full transactions"),
        }

        match by_hash_full.transactions {
            Some(BlockTransactions::Full(txs)) => {
                assert_eq!(txs.len(), 1);
                assert_eq!(txs[0].hash, tx_hash_present);
            }
            _ => panic!("expected full transactions"),
        }
    }

    #[tokio::test]
    async fn transaction_and_receipt_queries_reflect_index_presence() {
        let block_hash = B256::repeat_byte(0x71);
        let tx_hash = B256::repeat_byte(0x72);
        let missing_hash = B256::repeat_byte(0x73);
        let mut index = MockChainIndex::default();
        index
            .transactions
            .insert(tx_hash, transaction(tx_hash, 3, block_hash, 1));
        index
            .receipts
            .insert(tx_hash, receipt(tx_hash, 3, block_hash, 1));
        let provider = default_provider(Arc::new(index));

        let found_tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .expect("transaction query should succeed");
        let missing_tx = provider
            .get_transaction_by_hash(missing_hash)
            .await
            .expect("missing transaction query should succeed");
        let found_receipt = provider
            .get_transaction_receipt(tx_hash)
            .await
            .expect("receipt query should succeed");
        let missing_receipt = provider
            .get_transaction_receipt(missing_hash)
            .await
            .expect("missing receipt query should succeed");

        assert_eq!(found_tx.expect("transaction should exist").hash, tx_hash);
        assert!(missing_tx.is_none());
        assert_eq!(
            found_receipt
                .expect("receipt should exist")
                .transaction_hash,
            tx_hash
        );
        assert!(missing_receipt.is_none());
    }

    #[tokio::test]
    async fn chain_index_failures_map_to_expected_rpc_errors() {
        let provider_latest = default_provider(Arc::new(MockChainIndex {
            fail_latest_block_number: Mutex::new(Some(ChainIndexError::EmptyIndex)),
            ..Default::default()
        }));
        let latest_error = provider_latest
            .block_number()
            .await
            .expect_err("latest block query should fail");
        assert!(matches!(latest_error, RpcError::BlockNotFound));

        let provider_tx = default_provider(Arc::new(MockChainIndex {
            fail_get_transaction: Mutex::new(Some(ChainIndexError::TransactionNotFound(
                "0xdeadbeef".to_string(),
            ))),
            ..Default::default()
        }));
        let tx_error = provider_tx
            .get_transaction_by_hash(B256::repeat_byte(0xA1))
            .await
            .expect_err("transaction lookup should fail");
        assert!(matches!(tx_error, RpcError::TransactionNotFound));

        let provider_receipt = default_provider(Arc::new(MockChainIndex {
            fail_get_receipt: Mutex::new(Some(ChainIndexError::ReceiptNotFound(
                "0xbeef".to_string(),
            ))),
            ..Default::default()
        }));
        let receipt_error = provider_receipt
            .get_transaction_receipt(B256::repeat_byte(0xA2))
            .await
            .expect_err("receipt lookup should fail");
        assert!(matches!(receipt_error, RpcError::ReceiptNotFound));

        let provider_internal = default_provider(Arc::new(MockChainIndex {
            fail_get_block_by_hash: Mutex::new(Some(ChainIndexError::Sqlite(
                "db down".to_string(),
            ))),
            ..Default::default()
        }));
        let internal_error = provider_internal
            .get_block_by_hash(B256::repeat_byte(0xA3), false)
            .await
            .expect_err("block-by-hash lookup should fail");
        assert!(matches!(
            internal_error,
            RpcError::InternalError(message) if message.contains("sqlite error: db down")
        ));
    }
}
