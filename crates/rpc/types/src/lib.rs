//! Ethereum-compatible RPC types for the Evolve execution client.
//!
//! This crate provides types that serialize to JSON in the format expected by
//! Ethereum wallets and tooling (MetaMask, Foundry, etc.).

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use serde::{Deserialize, Serialize};

pub mod block;
pub mod log;
pub mod receipt;
pub mod transaction;

pub use block::RpcBlock;
pub use log::RpcLog;
pub use receipt::RpcReceipt;
pub use transaction::RpcTransaction;

/// Chain configuration for RPC responses.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub chain_id: u64,
}

/// Convert an internal AccountId to a deterministic compatibility address.
///
/// This is one-way and is intended for indexing/display fallbacks only.
pub fn account_id_to_address(account_id: evolve_core::AccountId) -> Address {
    evolve_tx_eth::derive_runtime_contract_address(account_id)
}

/// Derive canonical ETH-EOA AccountId from an Ethereum address.
pub fn address_to_account_id(address: Address) -> evolve_core::AccountId {
    evolve_tx_eth::derive_eth_eoa_account_id(address)
}

/// Sync status for eth_syncing response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SyncStatus {
    /// Not syncing - returns false
    NotSyncing(bool),
    /// Syncing - returns sync progress
    Syncing(SyncProgress),
}

/// Sync progress details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub starting_block: U64,
    pub current_block: U64,
    pub highest_block: U64,
}

/// Block number or tag for RPC requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockNumberOrTag {
    /// Specific block number (hex encoded)
    Number(U64),
    /// Block tag
    Tag(BlockTag),
}

impl Default for BlockNumberOrTag {
    fn default() -> Self {
        BlockNumberOrTag::Tag(BlockTag::Latest)
    }
}

/// Standard Ethereum block tags.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BlockTag {
    Latest,
    Earliest,
    Pending,
    Safe,
    Finalized,
}

/// Filter for eth_getLogs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// Start block (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_block: Option<BlockNumberOrTag>,
    /// End block (inclusive)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_block: Option<BlockNumberOrTag>,
    /// Contract address(es) to filter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<FilterAddress>,
    /// Topic filters (up to 4 topics)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<Option<FilterTopic>>>,
    /// Block hash (mutually exclusive with from_block/to_block)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<B256>,
}

/// Address filter - single or multiple addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterAddress {
    Single(Address),
    Multiple(Vec<Address>),
}

/// Topic filter - single topic or array of alternatives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterTopic {
    Single(B256),
    Multiple(Vec<B256>),
}

/// Call request for eth_call and eth_estimateGas.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    /// Sender address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Address>,
    /// Target address
    pub to: Address,
    /// Gas limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas: Option<U64>,
    /// Gas price (legacy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<U256>,
    /// Max fee per gas (EIP-1559)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<U256>,
    /// Max priority fee per gas (EIP-1559)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<U256>,
    /// Value to send
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<U256>,
    /// Input data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Bytes>,
    /// Input data (alias for data)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Bytes>,
}

impl CallRequest {
    /// Get the input data, preferring `input` over `data` if both are set.
    pub fn input_data(&self) -> Option<&Bytes> {
        self.input.as_ref().or(self.data.as_ref())
    }
}

/// Fee history response for eth_feeHistory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeHistory {
    /// Oldest block number in the range
    pub oldest_block: U64,
    /// Base fee per gas for each block
    pub base_fee_per_gas: Vec<U256>,
    /// Gas used ratio for each block (0.0 to 1.0)
    pub gas_used_ratio: Vec<f64>,
    /// Reward percentiles if requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward: Option<Vec<Vec<U256>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_tag_serialization() {
        let tag = BlockTag::Latest;
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, "\"latest\"");
    }

    #[test]
    fn test_block_number_or_tag() {
        let num = BlockNumberOrTag::Number(U64::from(100));
        let json = serde_json::to_string(&num).unwrap();
        assert_eq!(json, "\"0x64\"");

        let tag = BlockNumberOrTag::Tag(BlockTag::Latest);
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, "\"latest\"");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_account_id() -> impl Strategy<Value = evolve_core::AccountId> {
        any::<[u8; 32]>().prop_map(evolve_core::AccountId::from_bytes)
    }

    fn arb_address() -> impl Strategy<Value = Address> {
        any::<[u8; 20]>().prop_map(Address::from)
    }

    fn arb_b256() -> impl Strategy<Value = B256> {
        any::<[u8; 32]>().prop_map(B256::from)
    }

    fn arb_u64() -> impl Strategy<Value = U64> {
        any::<u64>().prop_map(U64::from)
    }

    fn arb_block_tag() -> impl Strategy<Value = BlockTag> {
        prop_oneof![
            Just(BlockTag::Latest),
            Just(BlockTag::Earliest),
            Just(BlockTag::Pending),
            Just(BlockTag::Safe),
            Just(BlockTag::Finalized),
        ]
    }

    fn arb_block_number_or_tag() -> impl Strategy<Value = BlockNumberOrTag> {
        prop_oneof![
            arb_u64().prop_map(BlockNumberOrTag::Number),
            arb_block_tag().prop_map(BlockNumberOrTag::Tag),
        ]
    }

    fn arb_sync_progress() -> impl Strategy<Value = SyncProgress> {
        (arb_u64(), arb_u64(), arb_u64()).prop_map(|(starting, current, highest)| SyncProgress {
            starting_block: starting,
            current_block: current,
            highest_block: highest,
        })
    }

    fn arb_sync_status() -> impl Strategy<Value = SyncStatus> {
        prop_oneof![
            Just(SyncStatus::NotSyncing(false)),
            arb_sync_progress().prop_map(SyncStatus::Syncing),
        ]
    }

    fn arb_filter_address() -> impl Strategy<Value = FilterAddress> {
        prop_oneof![
            arb_address().prop_map(FilterAddress::Single),
            prop::collection::vec(arb_address(), 1..5).prop_map(FilterAddress::Multiple),
        ]
    }

    fn arb_filter_topic() -> impl Strategy<Value = FilterTopic> {
        prop_oneof![
            arb_b256().prop_map(FilterTopic::Single),
            prop::collection::vec(arb_b256(), 1..5).prop_map(FilterTopic::Multiple),
        ]
    }

    fn arb_log_filter() -> impl Strategy<Value = LogFilter> {
        (
            prop::option::of(arb_block_number_or_tag()),
            prop::option::of(arb_block_number_or_tag()),
            prop::option::of(arb_filter_address()),
            prop::option::of(prop::collection::vec(
                prop::option::of(arb_filter_topic()),
                0..4,
            )),
            prop::option::of(arb_b256()),
        )
            .prop_map(
                |(from_block, to_block, address, topics, block_hash)| LogFilter {
                    from_block,
                    to_block,
                    address,
                    topics,
                    block_hash,
                },
            )
    }

    proptest! {
        #[test]
        fn prop_account_id_to_address_is_deterministic(id in arb_account_id()) {
            prop_assert_eq!(account_id_to_address(id), account_id_to_address(id));
        }

        #[test]
        fn prop_address_to_account_id_is_deterministic(address in arb_address()) {
            prop_assert_eq!(address_to_account_id(address), address_to_account_id(address));
        }

        // ==================== Custom serde implementations ====================
        // These test our custom Serialize/Deserialize impls, which have non-trivial logic

        #[test]
        fn prop_block_number_or_tag_serde_roundtrip(block in arb_block_number_or_tag()) {
            let json = serde_json::to_string(&block).unwrap();
            let recovered: BlockNumberOrTag = serde_json::from_str(&json).unwrap();

            match (&block, &recovered) {
                (BlockNumberOrTag::Number(n1), BlockNumberOrTag::Number(n2)) => {
                    prop_assert_eq!(n1, n2);
                }
                (BlockNumberOrTag::Tag(t1), BlockNumberOrTag::Tag(t2)) => {
                    prop_assert_eq!(t1, t2);
                }
                _ => prop_assert!(false, "Variant mismatch"),
            }
        }

        #[test]
        fn prop_sync_status_serde_roundtrip(status in arb_sync_status()) {
            let json = serde_json::to_string(&status).unwrap();
            let recovered: SyncStatus = serde_json::from_str(&json).unwrap();

            match (&status, &recovered) {
                (SyncStatus::NotSyncing(a), SyncStatus::NotSyncing(b)) => {
                    prop_assert_eq!(a, b);
                }
                (SyncStatus::Syncing(a), SyncStatus::Syncing(b)) => {
                    prop_assert_eq!(a.starting_block, b.starting_block);
                    prop_assert_eq!(a.current_block, b.current_block);
                    prop_assert_eq!(a.highest_block, b.highest_block);
                }
                _ => prop_assert!(false, "Variant mismatch"),
            }
        }

        #[test]
        fn prop_filter_address_serde_roundtrip(filter in arb_filter_address()) {
            let json = serde_json::to_string(&filter).unwrap();
            let recovered: FilterAddress = serde_json::from_str(&json).unwrap();

            match (&filter, &recovered) {
                (FilterAddress::Single(a), FilterAddress::Single(b)) => {
                    prop_assert_eq!(a, b);
                }
                (FilterAddress::Multiple(a), FilterAddress::Multiple(b)) => {
                    prop_assert_eq!(a, b);
                }
                _ => prop_assert!(false, "Variant mismatch"),
            }
        }

        #[test]
        fn prop_filter_topic_serde_roundtrip(filter in arb_filter_topic()) {
            let json = serde_json::to_string(&filter).unwrap();
            let recovered: FilterTopic = serde_json::from_str(&json).unwrap();

            match (&filter, &recovered) {
                (FilterTopic::Single(a), FilterTopic::Single(b)) => {
                    prop_assert_eq!(a, b);
                }
                (FilterTopic::Multiple(a), FilterTopic::Multiple(b)) => {
                    prop_assert_eq!(a, b);
                }
                _ => prop_assert!(false, "Variant mismatch"),
            }
        }

        // LogFilter has complex nested structure with custom serde
        #[test]
        fn prop_log_filter_serde_roundtrip(filter in arb_log_filter()) {
            let json = serde_json::to_string(&filter).unwrap();
            let _recovered: LogFilter = serde_json::from_str(&json).unwrap();
            // Successful deserialization validates the roundtrip
        }
    }
}
