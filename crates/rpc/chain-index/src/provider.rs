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

    async fn get_balance(&self, _address: Address, _block: Option<u64>) -> Result<U256, RpcError> {
        // TODO: Implement state queries via Storage + STF
        // For now, return zero
        Ok(U256::ZERO)
    }

    async fn get_transaction_count(
        &self,
        _address: Address,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        // TODO: Implement nonce queries via Storage + STF
        // For now, return zero
        Ok(0)
    }

    async fn call(&self, _request: &CallRequest, _block: Option<u64>) -> Result<Bytes, RpcError> {
        // TODO: Implement via STF::query()
        // For now, return empty
        Ok(Bytes::new())
    }

    async fn estimate_gas(
        &self,
        _request: &CallRequest,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        // TODO: Implement via STF with gas tracking
        // For now, return default gas
        Ok(21000)
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
