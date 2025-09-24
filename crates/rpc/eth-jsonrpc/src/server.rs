//! JSON-RPC server implementation.
//!
//! This module provides the RPC server that implements the Ethereum-compatible API.

use std::net::SocketAddr;
use std::sync::Arc;

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use async_trait::async_trait;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{PendingSubscriptionSink, RpcModule, SubscriptionMessage, SubscriptionSink};

use crate::api::{EthApiServer, EthPubSubApiServer, NetApiServer, Web3ApiServer};
use crate::error::RpcError;
use crate::subscriptions::{
    LogSubscriptionParams, SharedSubscriptionManager, SubscriptionKind, SubscriptionManager,
};
use evolve_rpc_types::{
    BlockNumberOrTag, CallRequest, FeeHistory, LogFilter, RpcBlock, RpcLog, RpcReceipt,
    RpcTransaction, SyncStatus,
};

/// Configuration for the RPC server.
#[derive(Debug, Clone)]
pub struct RpcServerConfig {
    /// Address to bind the HTTP server to.
    pub http_addr: SocketAddr,
    /// Chain ID to return for eth_chainId.
    pub chain_id: u64,
    /// Client version string.
    pub client_version: String,
}

impl Default for RpcServerConfig {
    fn default() -> Self {
        Self {
            http_addr: "127.0.0.1:8545".parse().unwrap(),
            chain_id: 1,
            client_version: "evolve/0.1.0".to_string(),
        }
    }
}

/// Trait for providing chain state to the RPC server.
///
/// This is the integration point between the RPC layer and the execution client.
/// Implement this trait to provide actual chain data.
#[async_trait]
pub trait StateProvider: Send + Sync + 'static {
    /// Get the current block number.
    async fn block_number(&self) -> Result<u64, RpcError>;

    /// Get a block by number.
    async fn get_block_by_number(
        &self,
        number: u64,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError>;

    /// Get a block by hash.
    async fn get_block_by_hash(
        &self,
        hash: B256,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError>;

    /// Get a transaction by hash.
    async fn get_transaction_by_hash(&self, hash: B256)
        -> Result<Option<RpcTransaction>, RpcError>;

    /// Get a transaction receipt by hash.
    async fn get_transaction_receipt(&self, hash: B256) -> Result<Option<RpcReceipt>, RpcError>;

    /// Get the balance of an account.
    async fn get_balance(&self, address: Address, block: Option<u64>) -> Result<U256, RpcError>;

    /// Get the nonce (transaction count) of an account.
    async fn get_transaction_count(
        &self,
        address: Address,
        block: Option<u64>,
    ) -> Result<u64, RpcError>;

    /// Execute a call without creating a transaction.
    async fn call(&self, request: &CallRequest, block: Option<u64>) -> Result<Bytes, RpcError>;

    /// Estimate gas for a transaction.
    async fn estimate_gas(
        &self,
        request: &CallRequest,
        block: Option<u64>,
    ) -> Result<u64, RpcError>;

    /// Get logs matching a filter.
    async fn get_logs(&self, filter: &LogFilter) -> Result<Vec<RpcLog>, RpcError>;

    /// Submit a raw transaction. Returns the transaction hash.
    async fn send_raw_transaction(&self, data: &[u8]) -> Result<B256, RpcError>;

    /// Get code at an address.
    async fn get_code(&self, address: Address, block: Option<u64>) -> Result<Bytes, RpcError>;

    /// Get storage at an address and position.
    async fn get_storage_at(
        &self,
        address: Address,
        position: U256,
        block: Option<u64>,
    ) -> Result<B256, RpcError>;
}

/// A no-op state provider for testing and development.
///
/// Returns empty/default values for all queries.
pub struct NoopStateProvider;

#[async_trait]
impl StateProvider for NoopStateProvider {
    async fn block_number(&self) -> Result<u64, RpcError> {
        Ok(0)
    }

    async fn get_block_by_number(
        &self,
        _number: u64,
        _full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError> {
        Ok(None)
    }

    async fn get_block_by_hash(
        &self,
        _hash: B256,
        _full_transactions: bool,
    ) -> Result<Option<RpcBlock>, RpcError> {
        Ok(None)
    }

    async fn get_transaction_by_hash(
        &self,
        _hash: B256,
    ) -> Result<Option<RpcTransaction>, RpcError> {
        Ok(None)
    }

    async fn get_transaction_receipt(&self, _hash: B256) -> Result<Option<RpcReceipt>, RpcError> {
        Ok(None)
    }

    async fn get_balance(&self, _address: Address, _block: Option<u64>) -> Result<U256, RpcError> {
        Ok(U256::ZERO)
    }

    async fn get_transaction_count(
        &self,
        _address: Address,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        Ok(0)
    }

    async fn call(&self, _request: &CallRequest, _block: Option<u64>) -> Result<Bytes, RpcError> {
        Ok(Bytes::new())
    }

    async fn estimate_gas(
        &self,
        _request: &CallRequest,
        _block: Option<u64>,
    ) -> Result<u64, RpcError> {
        Ok(21000) // Default gas for simple transfer
    }

    async fn get_logs(&self, _filter: &LogFilter) -> Result<Vec<RpcLog>, RpcError> {
        Ok(vec![])
    }

    async fn send_raw_transaction(&self, _data: &[u8]) -> Result<B256, RpcError> {
        // No-op: return a dummy hash
        Err(RpcError::NotImplemented("sendRawTransaction".to_string()))
    }

    async fn get_code(&self, _address: Address, _block: Option<u64>) -> Result<Bytes, RpcError> {
        Ok(Bytes::new())
    }

    async fn get_storage_at(
        &self,
        _address: Address,
        _position: U256,
        _block: Option<u64>,
    ) -> Result<B256, RpcError> {
        Ok(B256::ZERO)
    }
}

/// The RPC server implementation.
pub struct EthRpcServer<S: StateProvider> {
    config: RpcServerConfig,
    state: Arc<S>,
    subscriptions: SharedSubscriptionManager,
}

impl<S: StateProvider> EthRpcServer<S> {
    /// Create a new RPC server with the given configuration and state provider.
    pub fn new(config: RpcServerConfig, state: S) -> Self {
        Self {
            config,
            state: Arc::new(state),
            subscriptions: Arc::new(SubscriptionManager::new()),
        }
    }

    /// Create a new RPC server with a shared subscription manager.
    ///
    /// Use this when you need to publish events from outside the RPC server.
    pub fn with_subscription_manager(
        config: RpcServerConfig,
        state: S,
        subscriptions: SharedSubscriptionManager,
    ) -> Self {
        Self {
            config,
            state: Arc::new(state),
            subscriptions,
        }
    }

    /// Get a reference to the subscription manager.
    ///
    /// Use this to publish events (new blocks, logs, etc.) to subscribers.
    pub fn subscription_manager(&self) -> &SharedSubscriptionManager {
        &self.subscriptions
    }

    /// Resolve a block number or tag to an actual block number.
    async fn resolve_block_number(
        &self,
        block: Option<BlockNumberOrTag>,
    ) -> Result<Option<u64>, RpcError> {
        match block {
            None => Ok(None),
            Some(BlockNumberOrTag::Number(n)) => Ok(Some(n.to::<u64>())),
            Some(BlockNumberOrTag::Tag(tag)) => {
                use evolve_rpc_types::BlockTag;
                match tag {
                    BlockTag::Latest | BlockTag::Safe | BlockTag::Finalized => {
                        Ok(Some(self.state.block_number().await?))
                    }
                    BlockTag::Earliest => Ok(Some(0)),
                    BlockTag::Pending => Ok(Some(self.state.block_number().await?)),
                }
            }
        }
    }
}

#[async_trait]
impl<S: StateProvider> EthApiServer for EthRpcServer<S> {
    async fn chain_id(&self) -> Result<U64, ErrorObjectOwned> {
        Ok(U64::from(self.config.chain_id))
    }

    async fn block_number(&self) -> Result<U64, ErrorObjectOwned> {
        let number = self
            .state
            .block_number()
            .await
            .map_err(ErrorObjectOwned::from)?;
        Ok(U64::from(number))
    }

    async fn get_balance(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U256, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        self.state
            .get_balance(address, block_num)
            .await
            .map_err(|e| e.into())
    }

    async fn get_transaction_count(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U64, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        let count = self
            .state
            .get_transaction_count(address, block_num)
            .await
            .map_err(ErrorObjectOwned::from)?;
        Ok(U64::from(count))
    }

    async fn get_block_by_number(
        &self,
        block: BlockNumberOrTag,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(Some(block))
            .await
            .map_err(ErrorObjectOwned::from)?;
        match block_num {
            Some(n) => self
                .state
                .get_block_by_number(n, full_transactions)
                .await
                .map_err(|e| e.into()),
            None => Ok(None),
        }
    }

    async fn get_block_by_hash(
        &self,
        hash: B256,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, ErrorObjectOwned> {
        self.state
            .get_block_by_hash(hash, full_transactions)
            .await
            .map_err(|e| e.into())
    }

    async fn get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<RpcTransaction>, ErrorObjectOwned> {
        self.state
            .get_transaction_by_hash(hash)
            .await
            .map_err(|e| e.into())
    }

    async fn get_transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<RpcReceipt>, ErrorObjectOwned> {
        self.state
            .get_transaction_receipt(hash)
            .await
            .map_err(|e| e.into())
    }

    async fn call(
        &self,
        request: CallRequest,
        block: Option<BlockNumberOrTag>,
    ) -> Result<Bytes, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        self.state
            .call(&request, block_num)
            .await
            .map_err(|e| e.into())
    }

    async fn estimate_gas(
        &self,
        request: CallRequest,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U64, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        let gas = self
            .state
            .estimate_gas(&request, block_num)
            .await
            .map_err(ErrorObjectOwned::from)?;
        Ok(U64::from(gas))
    }

    async fn gas_price(&self) -> Result<U256, ErrorObjectOwned> {
        // Return zero gas price (gas is handled differently in evolve)
        Ok(U256::ZERO)
    }

    async fn send_raw_transaction(&self, data: Bytes) -> Result<B256, ErrorObjectOwned> {
        self.state
            .send_raw_transaction(data.as_ref())
            .await
            .map_err(|e| e.into())
    }

    async fn get_logs(&self, filter: LogFilter) -> Result<Vec<RpcLog>, ErrorObjectOwned> {
        self.state.get_logs(&filter).await.map_err(|e| e.into())
    }

    async fn syncing(&self) -> Result<SyncStatus, ErrorObjectOwned> {
        // For now, always return not syncing
        Ok(SyncStatus::NotSyncing(false))
    }

    async fn protocol_version(&self) -> Result<String, ErrorObjectOwned> {
        Ok("1".to_string())
    }

    async fn get_code(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<Bytes, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        self.state
            .get_code(address, block_num)
            .await
            .map_err(|e| e.into())
    }

    async fn get_storage_at(
        &self,
        address: Address,
        position: U256,
        block: Option<BlockNumberOrTag>,
    ) -> Result<B256, ErrorObjectOwned> {
        let block_num = self
            .resolve_block_number(block)
            .await
            .map_err(ErrorObjectOwned::from)?;
        self.state
            .get_storage_at(address, position, block_num)
            .await
            .map_err(|e| e.into())
    }

    async fn fee_history(
        &self,
        block_count: U64,
        _newest_block: BlockNumberOrTag,
        _reward_percentiles: Option<Vec<f64>>,
    ) -> Result<FeeHistory, ErrorObjectOwned> {
        // Return zero fees for the requested block count
        let count = block_count.to::<usize>().min(1024);
        Ok(FeeHistory {
            oldest_block: U64::ZERO,
            base_fee_per_gas: vec![U256::ZERO; count + 1],
            gas_used_ratio: vec![0.0; count],
            reward: None,
        })
    }

    async fn max_priority_fee_per_gas(&self) -> Result<U256, ErrorObjectOwned> {
        Ok(U256::ZERO)
    }

    async fn get_block_transaction_count_by_number(
        &self,
        block: BlockNumberOrTag,
    ) -> Result<Option<U64>, ErrorObjectOwned> {
        let block_data = self.get_block_by_number(block, false).await?;
        Ok(block_data.map(|b| match b.transactions {
            Some(evolve_rpc_types::block::BlockTransactions::Hashes(h)) => U64::from(h.len()),
            Some(evolve_rpc_types::block::BlockTransactions::Full(f)) => U64::from(f.len()),
            None => U64::ZERO,
        }))
    }

    async fn get_block_transaction_count_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<U64>, ErrorObjectOwned> {
        let block_data = self.get_block_by_hash(hash, false).await?;
        Ok(block_data.map(|b| match b.transactions {
            Some(evolve_rpc_types::block::BlockTransactions::Hashes(h)) => U64::from(h.len()),
            Some(evolve_rpc_types::block::BlockTransactions::Full(f)) => U64::from(f.len()),
            None => U64::ZERO,
        }))
    }
}

#[async_trait]
impl<S: StateProvider> Web3ApiServer for EthRpcServer<S> {
    async fn client_version(&self) -> Result<String, ErrorObjectOwned> {
        Ok(self.config.client_version.clone())
    }

    async fn sha3(&self, data: Bytes) -> Result<B256, ErrorObjectOwned> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data.as_ref());
        Ok(B256::from_slice(&hasher.finalize()))
    }
}

#[async_trait]
impl<S: StateProvider> NetApiServer for EthRpcServer<S> {
    async fn version(&self) -> Result<String, ErrorObjectOwned> {
        Ok(self.config.chain_id.to_string())
    }

    async fn listening(&self) -> Result<bool, ErrorObjectOwned> {
        Ok(true)
    }

    async fn peer_count(&self) -> Result<U64, ErrorObjectOwned> {
        Ok(U64::ZERO)
    }
}

#[async_trait]
impl<S: StateProvider> EthPubSubApiServer for EthRpcServer<S> {
    async fn subscribe(
        &self,
        pending: PendingSubscriptionSink,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> SubscriptionResult {
        // Parse the subscription kind
        let subscription_kind = match kind.as_str() {
            "newHeads" => SubscriptionKind::NewHeads,
            "logs" => SubscriptionKind::Logs,
            "newPendingTransactions" => SubscriptionKind::NewPendingTransactions,
            "syncing" => SubscriptionKind::Syncing,
            _ => {
                // Reject unknown subscription types
                pending
                    .reject(jsonrpsee::types::ErrorObject::owned(
                        -32602,
                        format!("Unknown subscription type: {}", kind),
                        None::<()>,
                    ))
                    .await;
                return Ok(());
            }
        };

        // Parse log filter params if this is a logs subscription
        let log_params = if subscription_kind == SubscriptionKind::Logs {
            params
                .map(serde_json::from_value::<LogSubscriptionParams>)
                .transpose()
                .map_err(|e| {
                    jsonrpsee::types::ErrorObject::owned(
                        -32602,
                        format!("Invalid log filter params: {}", e),
                        None::<()>,
                    )
                })?
        } else {
            None
        };

        // Register the subscription
        let sub_id = self
            .subscriptions
            .subscribe(subscription_kind.clone(), log_params.clone());

        // Accept the subscription
        let sink = pending.accept().await?;

        // Spawn a task to stream events to the subscriber
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            match subscription_kind {
                SubscriptionKind::NewHeads => {
                    Self::stream_new_heads(sink, subscriptions).await;
                }
                SubscriptionKind::Logs => {
                    Self::stream_logs(sink, subscriptions, log_params).await;
                }
                SubscriptionKind::NewPendingTransactions => {
                    Self::stream_pending_transactions(sink, subscriptions).await;
                }
                SubscriptionKind::Syncing => {
                    Self::stream_sync_status(sink, subscriptions).await;
                }
            }
        });

        log::debug!("Created subscription {} for {}", sub_id, kind);
        Ok(())
    }
}

impl<S: StateProvider> EthRpcServer<S> {
    /// Stream new block headers to a subscriber.
    async fn stream_new_heads(sink: SubscriptionSink, subscriptions: SharedSubscriptionManager) {
        let mut rx = subscriptions.subscribe_new_heads();

        loop {
            tokio::select! {
                _ = sink.closed() => {
                    log::debug!("Subscription closed by client");
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(block) => {
                            let msg = SubscriptionMessage::from_json(&*block).unwrap();
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Subscription lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Stream logs to a subscriber.
    async fn stream_logs(
        sink: SubscriptionSink,
        subscriptions: SharedSubscriptionManager,
        filter: Option<LogSubscriptionParams>,
    ) {
        let mut rx = subscriptions.subscribe_logs();

        loop {
            tokio::select! {
                _ = sink.closed() => {
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(log) => {
                            // Check if log matches filter
                            if !SubscriptionManager::log_matches_filter(&log, &filter) {
                                continue;
                            }
                            let msg = SubscriptionMessage::from_json(&*log).unwrap();
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Log subscription lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Stream pending transaction hashes to a subscriber.
    async fn stream_pending_transactions(
        sink: SubscriptionSink,
        subscriptions: SharedSubscriptionManager,
    ) {
        let mut rx = subscriptions.subscribe_pending_transactions();

        loop {
            tokio::select! {
                _ = sink.closed() => {
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(hash) => {
                            let msg = SubscriptionMessage::from_json(&hash).unwrap();
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Pending tx subscription lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Stream sync status to a subscriber.
    async fn stream_sync_status(sink: SubscriptionSink, subscriptions: SharedSubscriptionManager) {
        let mut rx = subscriptions.subscribe_sync();

        loop {
            tokio::select! {
                _ = sink.closed() => {
                    break;
                }
                result = rx.recv() => {
                    match result {
                        Ok(status) => {
                            let msg = SubscriptionMessage::from_json(&status).unwrap();
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Sync subscription lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Start the RPC server.
pub async fn start_server<S: StateProvider>(
    config: RpcServerConfig,
    state: S,
) -> Result<ServerHandle, Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::builder().build(config.http_addr).await?;

    let eth_rpc = EthRpcServer::new(config, state);
    let mut module = RpcModule::new(());

    module.merge(EthApiServer::into_rpc(eth_rpc.clone()))?;
    module.merge(Web3ApiServer::into_rpc(eth_rpc.clone()))?;
    module.merge(NetApiServer::into_rpc(eth_rpc.clone()))?;
    module.merge(EthPubSubApiServer::into_rpc(eth_rpc))?;

    let handle = server.start(module);
    Ok(handle)
}

// Need Clone for the RPC server to be used in multiple places
impl<S: StateProvider> Clone for EthRpcServer<S> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            state: Arc::clone(&self.state),
            subscriptions: Arc::clone(&self.subscriptions),
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_types)]
mod tests {
    use super::*;
    use evolve_rpc_types::{BlockTag, RpcBlock};
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// Minimal mock state provider for testing RPC server logic.
    /// Only implements what's needed for current tests.
    struct MockStateProvider {
        block_number: RwLock<u64>,
        blocks_by_number: RwLock<HashMap<u64, RpcBlock>>,
        blocks_by_hash: RwLock<HashMap<B256, RpcBlock>>,
        error: RwLock<Option<RpcError>>,
    }

    impl MockStateProvider {
        fn new() -> Self {
            Self {
                block_number: RwLock::new(0),
                blocks_by_number: RwLock::new(HashMap::new()),
                blocks_by_hash: RwLock::new(HashMap::new()),
                error: RwLock::new(None),
            }
        }

        fn with_block_number(self, number: u64) -> Self {
            *self.block_number.write().unwrap() = number;
            self
        }

        fn with_block(self, block: RpcBlock) -> Self {
            let number = block.number.to::<u64>();
            let hash = block.hash;
            self.blocks_by_number
                .write()
                .unwrap()
                .insert(number, block.clone());
            self.blocks_by_hash.write().unwrap().insert(hash, block);
            self
        }

        fn with_error(self, error: RpcError) -> Self {
            *self.error.write().unwrap() = Some(error);
            self
        }

        fn check_error(&self) -> Result<(), RpcError> {
            if let Some(ref err) = *self.error.read().unwrap() {
                Err(match err {
                    RpcError::InternalError(msg) => RpcError::InternalError(msg.clone()),
                    _ => RpcError::InternalError("Mock error".to_string()),
                })
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl StateProvider for MockStateProvider {
        async fn block_number(&self) -> Result<u64, RpcError> {
            self.check_error()?;
            Ok(*self.block_number.read().unwrap())
        }

        async fn get_block_by_number(
            &self,
            number: u64,
            _full_transactions: bool,
        ) -> Result<Option<RpcBlock>, RpcError> {
            self.check_error()?;
            Ok(self.blocks_by_number.read().unwrap().get(&number).cloned())
        }

        async fn get_block_by_hash(
            &self,
            hash: B256,
            _full_transactions: bool,
        ) -> Result<Option<RpcBlock>, RpcError> {
            self.check_error()?;
            Ok(self.blocks_by_hash.read().unwrap().get(&hash).cloned())
        }

        // Stub implementations for unused methods
        async fn get_transaction_by_hash(
            &self,
            _: B256,
        ) -> Result<Option<RpcTransaction>, RpcError> {
            Ok(None)
        }
        async fn get_transaction_receipt(&self, _: B256) -> Result<Option<RpcReceipt>, RpcError> {
            Ok(None)
        }
        async fn get_balance(&self, _: Address, _: Option<u64>) -> Result<U256, RpcError> {
            Ok(U256::ZERO)
        }
        async fn get_transaction_count(&self, _: Address, _: Option<u64>) -> Result<u64, RpcError> {
            Ok(0)
        }
        async fn call(&self, _: &CallRequest, _: Option<u64>) -> Result<Bytes, RpcError> {
            Ok(Bytes::new())
        }
        async fn estimate_gas(&self, _: &CallRequest, _: Option<u64>) -> Result<u64, RpcError> {
            Ok(21000)
        }
        async fn get_logs(&self, _: &LogFilter) -> Result<Vec<RpcLog>, RpcError> {
            Ok(vec![])
        }
        async fn send_raw_transaction(&self, _: &[u8]) -> Result<B256, RpcError> {
            Err(RpcError::NotImplemented("sendRawTransaction".to_string()))
        }
        async fn get_code(&self, _: Address, _: Option<u64>) -> Result<Bytes, RpcError> {
            Ok(Bytes::new())
        }
        async fn get_storage_at(
            &self,
            _: Address,
            _: U256,
            _: Option<u64>,
        ) -> Result<B256, RpcError> {
            Ok(B256::ZERO)
        }
    }

    fn make_test_block(number: u64, hash: B256) -> RpcBlock {
        RpcBlock {
            number: U64::from(number),
            hash,
            parent_hash: B256::ZERO,
            nonce: alloy_primitives::B64::ZERO,
            sha3_uncles: RpcBlock::empty_uncles_hash(),
            logs_bloom: Bytes::new(),
            transactions_root: B256::ZERO,
            state_root: B256::ZERO,
            receipts_root: B256::ZERO,
            miner: Address::ZERO,
            difficulty: U256::ZERO,
            total_difficulty: U256::ZERO,
            extra_data: Bytes::new(),
            size: U64::ZERO,
            gas_limit: U64::from(30_000_000u64),
            gas_used: U64::from(21000u64),
            timestamp: U64::from(1000u64),
            transactions: None,
            uncles: vec![],
            base_fee_per_gas: Some(U256::ZERO),
            withdrawals_root: None,
            withdrawals: None,
        }
    }

    // ==================== Block tag resolution tests ====================
    // These test the resolve_block_number method which has actual branching logic

    #[tokio::test]
    async fn test_resolve_block_number_tags() {
        let provider = MockStateProvider::new().with_block_number(100);
        let server = EthRpcServer::new(RpcServerConfig::default(), provider);

        // None returns None
        assert_eq!(server.resolve_block_number(None).await.unwrap(), None);

        // Explicit number
        let result = server
            .resolve_block_number(Some(BlockNumberOrTag::Number(U64::from(50))))
            .await
            .unwrap();
        assert_eq!(result, Some(50));

        // Latest/Safe/Finalized/Pending all resolve to current block
        for tag in [
            BlockTag::Latest,
            BlockTag::Safe,
            BlockTag::Finalized,
            BlockTag::Pending,
        ] {
            let result = server
                .resolve_block_number(Some(BlockNumberOrTag::Tag(tag)))
                .await
                .unwrap();
            assert_eq!(result, Some(100), "Failed for tag {:?}", tag);
        }

        // Earliest resolves to 0
        let result = server
            .resolve_block_number(Some(BlockNumberOrTag::Tag(BlockTag::Earliest)))
            .await
            .unwrap();
        assert_eq!(result, Some(0));
    }

    // ==================== Error handling tests ====================

    #[tokio::test]
    async fn test_error_propagation() {
        let provider =
            MockStateProvider::new().with_error(RpcError::InternalError("test error".to_string()));
        let server = EthRpcServer::new(RpcServerConfig::default(), provider);

        let result = EthApiServer::block_number(&server).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code(),
            crate::error::codes::INTERNAL_ERROR
        );
    }

    // ==================== Fee history tests ====================
    // Tests boundary condition (capping at 1024)

    #[tokio::test]
    async fn test_fee_history_capped_at_1024() {
        let provider = MockStateProvider::new().with_block_number(100);
        let server = EthRpcServer::new(RpcServerConfig::default(), provider);

        // Request more than 1024 blocks - should be capped
        let result = EthApiServer::fee_history(
            &server,
            U64::from(2000),
            BlockNumberOrTag::Tag(BlockTag::Latest),
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.base_fee_per_gas.len(), 1025); // capped at 1024 + 1
        assert_eq!(result.gas_used_ratio.len(), 1024);
    }

    // ==================== Transaction count extraction ====================
    // Tests the match logic for BlockTransactions enum

    #[tokio::test]
    async fn test_get_block_transaction_count_extraction() {
        use evolve_rpc_types::block::BlockTransactions;

        let hash = B256::repeat_byte(0x14);
        let mut block = make_test_block(100, hash);
        block.transactions = Some(BlockTransactions::Hashes(vec![
            B256::repeat_byte(0x15),
            B256::repeat_byte(0x16),
            B256::repeat_byte(0x17),
        ]));
        let provider = MockStateProvider::new()
            .with_block_number(100)
            .with_block(block);
        let server = EthRpcServer::new(RpcServerConfig::default(), provider);

        // Test by number
        let result = EthApiServer::get_block_transaction_count_by_number(
            &server,
            BlockNumberOrTag::Number(U64::from(100)),
        )
        .await
        .unwrap();
        assert_eq!(result, Some(U64::from(3)));

        // Test by hash
        let result = EthApiServer::get_block_transaction_count_by_hash(&server, hash)
            .await
            .unwrap();
        assert_eq!(result, Some(U64::from(3)));
    }
}
