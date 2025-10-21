//! StateProvider implementation using ChainIndex.
//!
//! This module provides `ChainStateProvider` which implements the `StateProvider`
//! trait from `evolve_eth_jsonrpc` using:
//! - `ChainIndex` for block/transaction/receipt queries
//! - `Storage` for state queries (balance, nonce, code)
//! - `Stf` for call/estimateGas execution

use std::sync::Arc;

use alloy_primitives::{Address, Bytes, B256, U256};
use async_trait::async_trait;

use crate::error::ChainIndexError;
use crate::index::ChainIndex;
use evolve_eth_jsonrpc::error::RpcError;
use evolve_eth_jsonrpc::StateProvider;
use evolve_rpc_types::block::BlockTransactions;
use evolve_rpc_types::SyncStatus;
use evolve_rpc_types::{CallRequest, LogFilter, RpcBlock, RpcLog, RpcReceipt, RpcTransaction};

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
pub struct ChainStateProvider<I: ChainIndex> {
    /// Chain index for blocks/transactions/receipts.
    index: Arc<I>,
    /// Configuration.
    config: ChainStateProviderConfig,
}

impl<I: ChainIndex> ChainStateProvider<I> {
    /// Create a new chain state provider.
    pub fn new(index: Arc<I>, config: ChainStateProviderConfig) -> Self {
        Self { index, config }
    }

    /// Get the chain index.
    pub fn index(&self) -> &Arc<I> {
        &self.index
    }

    /// Get the configuration.
    pub fn config(&self) -> &ChainStateProviderConfig {
        &self.config
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
impl<I: ChainIndex + 'static> StateProvider for ChainStateProvider<I> {
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

    async fn send_raw_transaction(&self, _data: &[u8]) -> Result<B256, RpcError> {
        // No mempool implemented yet
        Err(RpcError::NotImplemented("sendRawTransaction".to_string()))
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

impl<I: ChainIndex> ChainStateProvider<I> {
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
