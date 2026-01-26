//! Ethereum-compatible log/event types.

use alloy_primitives::{Address, Bytes, B256, U64};
use serde::{Deserialize, Serialize};

/// Ethereum-compatible log entry.
///
/// This matches the format returned by eth_getLogs and in transaction receipts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcLog {
    /// Contract address that emitted the log
    pub address: Address,
    /// Indexed topics (up to 4)
    pub topics: Vec<B256>,
    /// Log data
    pub data: Bytes,
    /// Block number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<U64>,
    /// Transaction hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<B256>,
    /// Transaction index in block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_index: Option<U64>,
    /// Block hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<B256>,
    /// Log index in block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_index: Option<U64>,
    /// Whether this log was removed due to chain reorg
    #[serde(default)]
    pub removed: bool,
}

impl RpcLog {
    /// Create a new log entry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: Address,
        topics: Vec<B256>,
        data: Bytes,
        block_number: u64,
        block_hash: B256,
        tx_hash: B256,
        tx_index: u64,
        log_index: u64,
    ) -> Self {
        Self {
            address,
            topics,
            data,
            block_number: Some(U64::from(block_number)),
            transaction_hash: Some(tx_hash),
            transaction_index: Some(U64::from(tx_index)),
            block_hash: Some(block_hash),
            log_index: Some(U64::from(log_index)),
            removed: false,
        }
    }

    /// Create a pending log (not yet in a block).
    pub fn pending(address: Address, topics: Vec<B256>, data: Bytes) -> Self {
        Self {
            address,
            topics,
            data,
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            block_hash: None,
            log_index: None,
            removed: false,
        }
    }
}

/// Convert an evolve Event to an RpcLog.
///
/// The evolve Event structure is:
/// - source: AccountId (becomes address)
/// - name: String (becomes first topic as keccak256 hash)
/// - contents: Message (becomes data)
///
/// Returns None if the event contents cannot be serialized.
pub fn event_to_log(
    event: &evolve_core::events_api::Event,
    block_number: u64,
    block_hash: B256,
    tx_hash: B256,
    tx_index: u64,
    log_index: u64,
) -> Option<RpcLog> {
    use sha2::{Digest, Sha256};

    let address = crate::account_id_to_address(event.source);

    // Hash the event name to create the first topic (similar to Solidity event signature)
    let mut hasher = Sha256::new();
    hasher.update(event.name.as_bytes());
    let name_hash = hasher.finalize();
    let topic0 = B256::from_slice(&name_hash);

    // Event contents become the data
    let data_bytes = event.contents.as_bytes().ok()?;
    let data = Bytes::copy_from_slice(data_bytes);

    Some(RpcLog::new(
        address,
        vec![topic0],
        data,
        block_number,
        block_hash,
        tx_hash,
        tx_index,
        log_index,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_serialization() {
        let log = RpcLog::new(
            Address::ZERO,
            vec![B256::ZERO],
            Bytes::from_static(&[1, 2, 3]),
            100,
            B256::ZERO,
            B256::ZERO,
            0,
            0,
        );
        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("\"blockNumber\":\"0x64\""));
        assert!(json.contains("\"removed\":false"));
    }

    #[test]
    fn test_pending_log() {
        let log = RpcLog::pending(Address::ZERO, vec![], Bytes::new());
        assert!(log.block_number.is_none());
        assert!(log.transaction_hash.is_none());
    }
}
