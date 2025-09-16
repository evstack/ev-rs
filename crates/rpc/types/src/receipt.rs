//! Ethereum-compatible transaction receipt types.

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use serde::{Deserialize, Serialize};

use crate::log::RpcLog;

/// Ethereum-compatible transaction receipt.
///
/// This matches the format returned by eth_getTransactionReceipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceipt {
    /// Transaction hash
    pub transaction_hash: B256,
    /// Transaction index in block
    pub transaction_index: U64,
    /// Block hash
    pub block_hash: B256,
    /// Block number
    pub block_number: U64,
    /// Sender address
    pub from: Address,
    /// Recipient address (null for contract creation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    /// Cumulative gas used in block up to this transaction
    pub cumulative_gas_used: U64,
    /// Gas used by this transaction
    pub gas_used: U64,
    /// Effective gas price
    pub effective_gas_price: U256,
    /// Contract address if this was a contract creation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<Address>,
    /// Logs emitted by this transaction
    pub logs: Vec<RpcLog>,
    /// Logs bloom filter
    pub logs_bloom: Bytes,
    /// Transaction type
    #[serde(rename = "type")]
    pub tx_type: U64,
    /// Status (1 = success, 0 = failure)
    pub status: U64,
}

impl RpcReceipt {
    /// Status code for successful transaction.
    pub const STATUS_SUCCESS: u64 = 1;
    /// Status code for failed transaction.
    pub const STATUS_FAILURE: u64 = 0;

    /// Create a receipt for a successful transaction.
    pub fn success(
        tx_hash: B256,
        tx_index: u64,
        block_hash: B256,
        block_number: u64,
        from: Address,
        to: Option<Address>,
        gas_used: u64,
        cumulative_gas_used: u64,
        logs: Vec<RpcLog>,
    ) -> Self {
        Self {
            transaction_hash: tx_hash,
            transaction_index: U64::from(tx_index),
            block_hash,
            block_number: U64::from(block_number),
            from,
            to,
            cumulative_gas_used: U64::from(cumulative_gas_used),
            gas_used: U64::from(gas_used),
            effective_gas_price: U256::ZERO,
            contract_address: None,
            logs,
            logs_bloom: Bytes::new(),
            tx_type: U64::ZERO,
            status: U64::from(Self::STATUS_SUCCESS),
        }
    }

    /// Create a receipt for a failed transaction.
    pub fn failure(
        tx_hash: B256,
        tx_index: u64,
        block_hash: B256,
        block_number: u64,
        from: Address,
        to: Option<Address>,
        gas_used: u64,
        cumulative_gas_used: u64,
    ) -> Self {
        Self {
            transaction_hash: tx_hash,
            transaction_index: U64::from(tx_index),
            block_hash,
            block_number: U64::from(block_number),
            from,
            to,
            cumulative_gas_used: U64::from(cumulative_gas_used),
            gas_used: U64::from(gas_used),
            effective_gas_price: U256::ZERO,
            contract_address: None,
            logs: vec![],
            logs_bloom: Bytes::new(),
            tx_type: U64::ZERO,
            status: U64::from(Self::STATUS_FAILURE),
        }
    }

    /// Check if the transaction was successful.
    pub fn is_success(&self) -> bool {
        self.status == U64::from(Self::STATUS_SUCCESS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receipt_serialization() {
        let receipt = RpcReceipt::success(
            B256::ZERO,
            0,
            B256::ZERO,
            1,
            Address::ZERO,
            Some(Address::ZERO),
            21000,
            21000,
            vec![],
        );
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(json.contains("\"status\":\"0x1\""));
        assert!(json.contains("\"gasUsed\":\"0x5208\""));
    }

    #[test]
    fn test_failure_receipt() {
        let receipt = RpcReceipt::failure(
            B256::ZERO,
            0,
            B256::ZERO,
            1,
            Address::ZERO,
            None,
            50000,
            50000,
        );
        assert!(!receipt.is_success());
        assert_eq!(receipt.status, U64::from(0u64));
    }
}
