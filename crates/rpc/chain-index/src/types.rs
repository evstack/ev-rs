//! Types for chain data storage.
//!
//! These types are optimized for storage and indexing, separate from the RPC
//! response types in `evolve_rpc_types`.

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use serde::{Deserialize, Serialize};

/// Stored block header with essential fields for indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBlock {
    /// Block number/height.
    pub number: u64,
    /// Block hash.
    pub hash: B256,
    /// Parent block hash.
    pub parent_hash: B256,
    /// State root after this block.
    pub state_root: B256,
    /// Transactions root (merkle root of tx hashes).
    pub transactions_root: B256,
    /// Receipts root.
    pub receipts_root: B256,
    /// Block timestamp (Unix seconds).
    pub timestamp: u64,
    /// Total gas used in this block.
    pub gas_used: u64,
    /// Gas limit for this block.
    pub gas_limit: u64,
    /// Number of transactions in this block.
    pub transaction_count: u32,
    /// Validator/miner address (if applicable).
    pub miner: Address,
    /// Extra data (chain-specific).
    pub extra_data: Bytes,
}

impl StoredBlock {
    /// Convert to RPC block format.
    pub fn to_rpc_block(
        &self,
        transactions: Option<evolve_rpc_types::block::BlockTransactions>,
    ) -> evolve_rpc_types::RpcBlock {
        use alloy_primitives::B64;

        evolve_rpc_types::RpcBlock {
            number: U64::from(self.number),
            hash: self.hash,
            parent_hash: self.parent_hash,
            nonce: B64::ZERO,
            sha3_uncles: evolve_rpc_types::RpcBlock::empty_uncles_hash(),
            logs_bloom: Bytes::new(),
            transactions_root: self.transactions_root,
            state_root: self.state_root,
            receipts_root: self.receipts_root,
            miner: self.miner,
            difficulty: U256::ZERO,
            total_difficulty: U256::ZERO,
            extra_data: self.extra_data.clone(),
            size: U64::ZERO, // TODO: compute actual size
            gas_limit: U64::from(self.gas_limit),
            gas_used: U64::from(self.gas_used),
            timestamp: U64::from(self.timestamp),
            transactions,
            uncles: vec![],
            base_fee_per_gas: Some(U256::ZERO),
            withdrawals_root: None,
            withdrawals: None,
        }
    }
}

/// Stored transaction with full data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTransaction {
    /// Transaction hash.
    pub hash: B256,
    /// Block number this transaction is in.
    pub block_number: u64,
    /// Block hash.
    pub block_hash: B256,
    /// Index within the block.
    pub transaction_index: u32,
    /// Sender address.
    pub from: Address,
    /// Recipient address (None for contract creation).
    pub to: Option<Address>,
    /// Value transferred.
    pub value: U256,
    /// Gas limit.
    pub gas: u64,
    /// Gas price.
    pub gas_price: U256,
    /// Input data.
    pub input: Bytes,
    /// Nonce.
    pub nonce: u64,
    /// Signature V.
    pub v: u64,
    /// Signature R.
    pub r: U256,
    /// Signature S.
    pub s: U256,
    /// Transaction type (0 = legacy, 2 = EIP-1559).
    pub tx_type: u8,
    /// Chain ID.
    pub chain_id: Option<u64>,
}

impl StoredTransaction {
    /// Convert to RPC transaction format.
    pub fn to_rpc_transaction(&self) -> evolve_rpc_types::RpcTransaction {
        evolve_rpc_types::RpcTransaction {
            hash: self.hash,
            nonce: U64::from(self.nonce),
            block_hash: Some(self.block_hash),
            block_number: Some(U64::from(self.block_number)),
            transaction_index: Some(U64::from(self.transaction_index)),
            from: self.from,
            to: self.to,
            value: self.value,
            gas: U64::from(self.gas),
            gas_price: Some(self.gas_price),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            input: self.input.clone(),
            v: U64::from(self.v),
            r: self.r,
            s: self.s,
            tx_type: U64::from(self.tx_type as u64),
            chain_id: self.chain_id.map(U64::from),
            access_list: None,
        }
    }
}

/// Stored transaction receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredReceipt {
    /// Transaction hash.
    pub transaction_hash: B256,
    /// Transaction index in block.
    pub transaction_index: u32,
    /// Block hash.
    pub block_hash: B256,
    /// Block number.
    pub block_number: u64,
    /// Sender address.
    pub from: Address,
    /// Recipient address.
    pub to: Option<Address>,
    /// Cumulative gas used up to this transaction.
    pub cumulative_gas_used: u64,
    /// Gas used by this transaction.
    pub gas_used: u64,
    /// Contract address if this was a contract creation.
    pub contract_address: Option<Address>,
    /// Logs emitted by this transaction.
    pub logs: Vec<StoredLog>,
    /// Status (1 = success, 0 = failure).
    pub status: u8,
    /// Transaction type.
    pub tx_type: u8,
}

impl StoredReceipt {
    /// Convert to RPC receipt format.
    pub fn to_rpc_receipt(&self) -> evolve_rpc_types::RpcReceipt {
        let logs: Vec<evolve_rpc_types::RpcLog> = self
            .logs
            .iter()
            .enumerate()
            .map(|(i, log)| {
                log.to_rpc_log(
                    self.block_number,
                    self.block_hash,
                    self.transaction_hash,
                    self.transaction_index,
                    i as u64,
                )
            })
            .collect();

        evolve_rpc_types::RpcReceipt {
            transaction_hash: self.transaction_hash,
            transaction_index: U64::from(self.transaction_index),
            block_hash: self.block_hash,
            block_number: U64::from(self.block_number),
            from: self.from,
            to: self.to,
            cumulative_gas_used: U64::from(self.cumulative_gas_used),
            gas_used: U64::from(self.gas_used),
            effective_gas_price: U256::ZERO,
            contract_address: self.contract_address,
            logs,
            logs_bloom: Bytes::new(),
            tx_type: U64::from(self.tx_type as u64),
            status: U64::from(self.status as u64),
        }
    }
}

/// Stored log/event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredLog {
    /// Contract address that emitted the log.
    pub address: Address,
    /// Indexed topics.
    pub topics: Vec<B256>,
    /// Log data.
    pub data: Bytes,
}

impl StoredLog {
    /// Convert to RPC log format with block/tx context.
    pub fn to_rpc_log(
        &self,
        block_number: u64,
        block_hash: B256,
        tx_hash: B256,
        tx_index: u32,
        log_index: u64,
    ) -> evolve_rpc_types::RpcLog {
        evolve_rpc_types::RpcLog {
            address: self.address,
            topics: self.topics.clone(),
            data: self.data.clone(),
            block_number: Some(U64::from(block_number)),
            transaction_hash: Some(tx_hash),
            transaction_index: Some(U64::from(tx_index)),
            block_hash: Some(block_hash),
            log_index: Some(U64::from(log_index)),
            removed: false,
        }
    }
}

/// Transaction location in the chain.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TxLocation {
    /// Block number.
    pub block_number: u64,
    /// Index within the block.
    pub transaction_index: u32,
}
