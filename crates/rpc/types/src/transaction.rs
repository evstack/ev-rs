//! Ethereum-compatible transaction types.
#![cfg_attr(test, allow(clippy::indexing_slicing))]

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use serde::{Deserialize, Serialize};

/// Ethereum-compatible transaction representation.
///
/// This matches the format returned by eth_getTransactionByHash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransaction {
    /// Transaction hash
    pub hash: B256,
    /// Nonce
    pub nonce: U64,
    /// Block hash (null if pending)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<B256>,
    /// Block number (null if pending)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<U64>,
    /// Transaction index in block (null if pending)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_index: Option<U64>,
    /// Sender address
    pub from: Address,
    /// Recipient address (null for contract creation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    /// Value transferred
    pub value: U256,
    /// Gas limit
    pub gas: U64,
    /// Gas price (legacy transactions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<U256>,
    /// Max fee per gas (EIP-1559)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<U256>,
    /// Max priority fee per gas (EIP-1559)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<U256>,
    /// Input data
    pub input: Bytes,
    /// Signature V
    pub v: U64,
    /// Signature R
    pub r: U256,
    /// Signature S
    pub s: U256,
    /// Transaction type (0 = legacy, 1 = EIP-2930, 2 = EIP-1559)
    #[serde(rename = "type")]
    pub tx_type: U64,
    /// Chain ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<U64>,
    /// Access list (EIP-2930)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_list: Option<Vec<AccessListItem>>,
}

/// Access list entry (EIP-2930).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<B256>,
}

/// Pending transaction (for mempool/subscription).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTransaction {
    /// Transaction hash
    pub hash: B256,
    /// Sender address
    pub from: Address,
    /// Recipient address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Address>,
    /// Value
    pub value: U256,
    /// Gas limit
    pub gas: U64,
    /// Gas price or max fee
    pub gas_price: U256,
    /// Nonce
    pub nonce: U64,
    /// Input data
    pub input: Bytes,
}

impl RpcTransaction {
    /// Create a minimal transaction for testing.
    pub fn minimal(
        hash: B256,
        from: Address,
        to: Option<Address>,
        nonce: u64,
        value: U256,
    ) -> Self {
        Self {
            hash,
            nonce: U64::from(nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to,
            value,
            gas: U64::from(21000u64),
            gas_price: Some(U256::ZERO),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            input: Bytes::new(),
            v: U64::ZERO,
            r: U256::ZERO,
            s: U256::ZERO,
            tx_type: U64::ZERO,
            chain_id: None,
            access_list: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_transaction_serialization() {
        let tx = RpcTransaction::minimal(
            B256::ZERO,
            Address::ZERO,
            Some(Address::ZERO),
            1,
            U256::from(1000),
        );
        let json_value: Value = serde_json::to_value(&tx).unwrap();
        assert_eq!(json_value["nonce"], "0x1");
        assert_eq!(json_value["type"], "0x0");
        assert_eq!(json_value["value"], "0x3e8");
        assert_eq!(json_value["gas"], "0x5208");
        assert_eq!(
            json_value["to"],
            "0x0000000000000000000000000000000000000000"
        );
    }
}
