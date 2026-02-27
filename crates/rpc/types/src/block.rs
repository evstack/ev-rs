//! Ethereum-compatible block types.
#![cfg_attr(test, allow(clippy::indexing_slicing))]

use alloy_primitives::{Address, Bytes, B256, B64, U256, U64};
use serde::{Deserialize, Serialize};

use crate::transaction::RpcTransaction;

/// Ethereum-compatible block representation.
///
/// This matches the format returned by eth_getBlockByNumber/eth_getBlockByHash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlock {
    /// Block number
    pub number: U64,
    /// Block hash
    pub hash: B256,
    /// Parent block hash
    pub parent_hash: B256,
    /// Nonce (PoW artifact, always zero)
    pub nonce: B64,
    /// SHA3 of uncles data (always empty uncles hash)
    pub sha3_uncles: B256,
    /// Bloom filter for logs
    pub logs_bloom: Bytes,
    /// Transactions root
    pub transactions_root: B256,
    /// State root
    pub state_root: B256,
    /// Receipts root
    pub receipts_root: B256,
    /// Block miner/validator address
    pub miner: Address,
    /// Difficulty (PoW artifact, always zero)
    pub difficulty: U256,
    /// Total difficulty (PoW artifact, always zero)
    pub total_difficulty: U256,
    /// Extra data
    pub extra_data: Bytes,
    /// Block size in bytes
    pub size: U64,
    /// Gas limit
    pub gas_limit: U64,
    /// Gas used
    pub gas_used: U64,
    /// Block timestamp (Unix seconds)
    pub timestamp: U64,
    /// Transactions - either hashes or full objects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<BlockTransactions>,
    /// Uncles (always empty)
    pub uncles: Vec<B256>,
    /// Base fee per gas (EIP-1559)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fee_per_gas: Option<U256>,
    /// Withdrawals root (post-Shanghai, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawals_root: Option<B256>,
    /// Withdrawals (post-Shanghai, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawals: Option<Vec<Withdrawal>>,
}

/// Block transactions - either just hashes or full transaction objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockTransactions {
    /// Only transaction hashes
    Hashes(Vec<B256>),
    /// Full transaction objects
    Full(Vec<RpcTransaction>),
}

/// Withdrawal (post-Shanghai).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Withdrawal {
    pub index: U64,
    pub validator_index: U64,
    pub address: Address,
    pub amount: U64,
}

impl RpcBlock {
    /// Create an empty block hash (32 zero bytes).
    pub fn empty_hash() -> B256 {
        B256::ZERO
    }

    /// Empty uncles hash (keccak256 of RLP empty list).
    pub fn empty_uncles_hash() -> B256 {
        // keccak256(rlp([])) = 0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347
        B256::from_slice(&[
            0x1d, 0xcc, 0x4d, 0xe8, 0xde, 0xc7, 0x5d, 0x7a, 0xab, 0x85, 0xb5, 0x67, 0xb6, 0xcc,
            0xd4, 0x1a, 0xd3, 0x12, 0x45, 0x1b, 0x94, 0x8a, 0x74, 0x13, 0xf0, 0xa1, 0x42, 0xfd,
            0x40, 0xd4, 0x93, 0x47,
        ])
    }

    /// Empty transactions root (keccak256 of RLP empty list).
    pub fn empty_transactions_root() -> B256 {
        // Same as uncles for empty trie
        B256::from_slice(&[
            0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0,
            0xf8, 0x6e, 0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5,
            0xe3, 0x63, 0xb4, 0x21,
        ])
    }

    /// Empty receipts root.
    pub fn empty_receipts_root() -> B256 {
        Self::empty_transactions_root()
    }

    /// Create a minimal block at a given height.
    pub fn minimal(number: u64, parent_hash: B256, state_root: B256, timestamp: u64) -> Self {
        Self {
            number: U64::from(number),
            hash: B256::ZERO, // Should be computed
            parent_hash,
            nonce: B64::ZERO,
            sha3_uncles: Self::empty_uncles_hash(),
            logs_bloom: Bytes::new(),
            transactions_root: Self::empty_transactions_root(),
            state_root,
            receipts_root: Self::empty_receipts_root(),
            miner: Address::ZERO,
            difficulty: U256::ZERO,
            total_difficulty: U256::ZERO,
            extra_data: Bytes::new(),
            size: U64::ZERO,
            gas_limit: U64::from(30_000_000u64), // Default gas limit
            gas_used: U64::ZERO,
            timestamp: U64::from(timestamp),
            transactions: Some(BlockTransactions::Hashes(vec![])),
            uncles: vec![],
            base_fee_per_gas: Some(U256::ZERO),
            withdrawals_root: None,
            withdrawals: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_block_serialization() {
        let block = RpcBlock::minimal(1, B256::ZERO, B256::ZERO, 1234567890);
        let json_value: Value = serde_json::to_value(&block).unwrap();
        assert_eq!(json_value["number"], "0x1");
        assert_eq!(json_value["timestamp"], "0x499602d2");
        assert_eq!(json_value["gasLimit"], "0x1c9c380");
        assert_eq!(json_value["gasUsed"], "0x0");
        assert_eq!(json_value["transactions"], serde_json::json!([]));
        assert_eq!(json_value["uncles"], serde_json::json!([]));
    }
}
