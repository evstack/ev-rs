//! Integration helpers for converting STF execution results to chain-index storage types.
//!
//! This module provides utilities to bridge the STF execution output with the
//! chain indexer, converting blocks, transactions, and events into storable formats.

use alloy_primitives::{Address, Bytes, B256, U256};
use evolve_core::events_api::Event;
use evolve_core::AccountId;
use evolve_rpc_types::account_id_to_address;
use evolve_server_core::{Block, Transaction};
use evolve_stf::results::{BlockResult, TxResult};
use sha2::{Digest, Sha256};

use crate::types::{StoredBlock, StoredLog, StoredReceipt, StoredTransaction};

/// Metadata required for block indexing that isn't available from STF alone.
#[derive(Debug, Clone)]
pub struct BlockMetadata {
    /// Block hash (computed externally or from consensus).
    pub hash: B256,
    /// Parent block hash.
    pub parent_hash: B256,
    /// State root after block execution.
    pub state_root: B256,
    /// Block timestamp in Unix seconds.
    pub timestamp: u64,
    /// Gas limit for the block.
    pub gas_limit: u64,
    /// Validator/proposer address.
    pub miner: Address,
    /// Extra data (chain-specific).
    pub extra_data: Bytes,
    /// Chain ID for transactions.
    pub chain_id: u64,
}

impl Default for BlockMetadata {
    fn default() -> Self {
        Self {
            hash: B256::ZERO,
            parent_hash: B256::ZERO,
            state_root: B256::ZERO,
            timestamp: 0,
            gas_limit: 30_000_000, // Default gas limit
            miner: Address::ZERO,
            extra_data: Bytes::new(),
            chain_id: 1,
        }
    }
}

impl BlockMetadata {
    /// Create new block metadata.
    pub fn new(
        hash: B256,
        parent_hash: B256,
        state_root: B256,
        timestamp: u64,
        gas_limit: u64,
        miner: Address,
        chain_id: u64,
    ) -> Self {
        Self {
            hash,
            parent_hash,
            state_root,
            timestamp,
            gas_limit,
            miner,
            extra_data: Bytes::new(),
            chain_id,
        }
    }

    /// Set extra data.
    pub fn with_extra_data(mut self, extra_data: Bytes) -> Self {
        self.extra_data = extra_data;
        self
    }
}

/// Convert an STF Event to a StoredLog.
///
/// Events in the Evolve system have:
/// - `source`: AccountId of the emitting contract
/// - `name`: Event name (becomes topic[0] as keccak hash)
/// - `contents`: Serialized event data
pub fn event_to_stored_log(event: &Event) -> StoredLog {
    // Convert source AccountId to Address
    let address = account_id_to_address(event.source);

    // Create topic[0] from event name hash
    let name_hash = {
        let mut hasher = Sha256::new();
        hasher.update(event.name.as_bytes());
        let result = hasher.finalize();
        B256::from_slice(&result)
    };

    // Event contents become the data
    let data = Bytes::from(event.contents.as_vec().unwrap_or_default());

    StoredLog {
        address,
        topics: vec![name_hash],
        data,
    }
}

/// Build indexable data from STF execution results.
///
/// This function takes a Block, its execution results, and metadata,
/// and produces the types needed for chain indexing.
pub fn build_index_data<B, Tx>(
    block: &B,
    result: &BlockResult,
    metadata: &BlockMetadata,
) -> (StoredBlock, Vec<StoredTransaction>, Vec<StoredReceipt>)
where
    B: Block<Tx>,
    Tx: Transaction,
{
    let block_number = block.height();
    let block_hash = metadata.hash;
    let txs = block.txs();

    // Calculate total gas used
    let total_gas_used: u64 = result.tx_results.iter().map(|r| r.gas_used).sum();

    // Build stored transactions and receipts
    let mut stored_txs = Vec::with_capacity(txs.len());
    let mut stored_receipts = Vec::with_capacity(txs.len());
    let mut cumulative_gas = 0u64;

    for (idx, (tx, tx_result)) in txs.iter().zip(result.tx_results.iter()).enumerate() {
        let tx_hash = B256::from(tx.compute_identifier());
        cumulative_gas += tx_result.gas_used;

        // Build stored transaction
        let stored_tx = build_stored_transaction(
            tx,
            tx_hash,
            block_number,
            block_hash,
            idx as u32,
            metadata.chain_id,
        );
        stored_txs.push(stored_tx);

        // Build stored receipt
        let stored_receipt = build_stored_receipt(
            tx,
            tx_result,
            tx_hash,
            block_number,
            block_hash,
            idx as u32,
            cumulative_gas,
        );
        stored_receipts.push(stored_receipt);
    }

    // Compute merkle roots (simplified - just hash of concatenated hashes)
    let transactions_root = compute_tx_root(&stored_txs);
    let receipts_root = compute_receipts_root(&stored_receipts);

    // Build stored block
    let stored_block = StoredBlock {
        number: block_number,
        hash: block_hash,
        parent_hash: metadata.parent_hash,
        state_root: metadata.state_root,
        transactions_root,
        receipts_root,
        timestamp: metadata.timestamp,
        gas_used: total_gas_used,
        gas_limit: metadata.gas_limit,
        transaction_count: txs.len() as u32,
        miner: metadata.miner,
        extra_data: metadata.extra_data.clone(),
    };

    (stored_block, stored_txs, stored_receipts)
}

/// Build a StoredTransaction from an STF Transaction.
fn build_stored_transaction<Tx: Transaction>(
    tx: &Tx,
    tx_hash: B256,
    block_number: u64,
    block_hash: B256,
    transaction_index: u32,
    chain_id: u64,
) -> StoredTransaction {
    let from = account_id_to_address(tx.sender());
    let to = {
        let recipient = tx.recipient();
        // Check if recipient is the invalid/zero account
        if recipient == AccountId::invalid() {
            None
        } else {
            Some(account_id_to_address(recipient))
        }
    };

    // Extract value from funds (sum of all fungible assets as a simple approach)
    let value = tx
        .funds()
        .iter()
        .fold(U256::ZERO, |acc, fa| acc + U256::from(fa.amount));

    // Serialize the request as input data
    let input = Bytes::from(borsh::to_vec(tx.request()).unwrap_or_default());

    StoredTransaction {
        hash: tx_hash,
        block_number,
        block_hash,
        transaction_index,
        from,
        to,
        value,
        gas: tx.gas_limit(),
        gas_price: U256::ZERO, // TODO: Support gas pricing
        input,
        nonce: 0, // TODO: Track nonces if needed
        v: 0,
        r: U256::ZERO,
        s: U256::ZERO,
        tx_type: 0, // Legacy type
        chain_id: Some(chain_id),
    }
}

/// Build a StoredReceipt from an STF TxResult.
fn build_stored_receipt<Tx: Transaction>(
    tx: &Tx,
    tx_result: &TxResult,
    tx_hash: B256,
    block_number: u64,
    block_hash: B256,
    transaction_index: u32,
    cumulative_gas_used: u64,
) -> StoredReceipt {
    let from = account_id_to_address(tx.sender());
    let to = {
        let recipient = tx.recipient();
        if recipient == AccountId::invalid() {
            None
        } else {
            Some(account_id_to_address(recipient))
        }
    };

    // Convert events to logs
    let logs: Vec<StoredLog> = tx_result.events.iter().map(event_to_stored_log).collect();

    // Determine status (1 = success, 0 = failure)
    let status = if tx_result.response.is_ok() { 1 } else { 0 };

    StoredReceipt {
        transaction_hash: tx_hash,
        transaction_index,
        block_hash,
        block_number,
        from,
        to,
        cumulative_gas_used,
        gas_used: tx_result.gas_used,
        contract_address: None, // TODO: Detect contract creation
        logs,
        status,
        tx_type: 0,
    }
}

/// Compute a simple transactions root (hash of all tx hashes).
fn compute_tx_root(txs: &[StoredTransaction]) -> B256 {
    if txs.is_empty() {
        return B256::ZERO;
    }

    let mut hasher = Sha256::new();
    for tx in txs {
        hasher.update(tx.hash.as_slice());
    }
    B256::from_slice(&hasher.finalize())
}

/// Compute a simple receipts root (hash of all receipt hashes).
fn compute_receipts_root(receipts: &[StoredReceipt]) -> B256 {
    if receipts.is_empty() {
        return B256::ZERO;
    }

    let mut hasher = Sha256::new();
    for receipt in receipts {
        hasher.update(receipt.transaction_hash.as_slice());
        hasher.update([receipt.status]);
        hasher.update(receipt.gas_used.to_be_bytes());
    }
    B256::from_slice(&hasher.finalize())
}

/// Helper to index a block after STF execution.
///
/// This is a convenience function that builds the index data and stores it.
pub fn index_block<B, Tx, I>(
    index: &I,
    block: &B,
    result: &BlockResult,
    metadata: &BlockMetadata,
) -> crate::error::ChainIndexResult<()>
where
    B: Block<Tx>,
    Tx: Transaction,
    I: crate::index::ChainIndex,
{
    let (stored_block, stored_txs, stored_receipts) = build_index_data(block, result, metadata);
    index.store_block(stored_block, stored_txs, stored_receipts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::Message;

    #[test]
    fn test_event_to_stored_log() {
        let event = Event {
            source: AccountId::new(42),
            name: "Transfer".to_string(),
            contents: Message::from_bytes(vec![1, 2, 3, 4]),
        };

        let log = event_to_stored_log(&event);

        // Address should be derived from AccountId
        assert_eq!(log.address, account_id_to_address(AccountId::new(42)));
        // Should have one topic (the name hash)
        assert_eq!(log.topics.len(), 1);
        // Data should match contents
        assert_eq!(log.data.as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_block_metadata_default() {
        let meta = BlockMetadata::default();
        assert_eq!(meta.hash, B256::ZERO);
        assert_eq!(meta.gas_limit, 30_000_000);
    }
}
