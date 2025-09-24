//! Conversion between internal RPC types and protobuf types.

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use evolve_rpc_types::{
    block::BlockTransactions, BlockNumberOrTag, BlockTag, CallRequest as RpcCallRequest,
    FilterAddress, FilterTopic, LogFilter as RpcLogFilter, RpcBlock, RpcLog, RpcReceipt,
    RpcTransaction, SyncStatus as RpcSyncStatus,
};

use crate::proto::evolve::v1::{self as proto};

// ============================================================================
// Primitive type conversions
// ============================================================================

/// Convert a B256 to proto H256.
pub fn b256_to_proto(value: B256) -> proto::H256 {
    proto::H256 {
        data: value.as_slice().to_vec(),
    }
}

/// Convert proto H256 to B256.
pub fn proto_to_b256(value: &proto::H256) -> Option<B256> {
    if value.data.len() == 32 {
        Some(B256::from_slice(&value.data))
    } else {
        None
    }
}

/// Convert an Address to proto Address.
pub fn address_to_proto(value: Address) -> proto::Address {
    proto::Address {
        data: value.as_slice().to_vec(),
    }
}

/// Convert proto Address to Address.
pub fn proto_to_address(value: &proto::Address) -> Option<Address> {
    if value.data.len() == 20 {
        Some(Address::from_slice(&value.data))
    } else {
        None
    }
}

/// Convert a U256 to proto U256.
pub fn u256_to_proto(value: U256) -> proto::U256 {
    proto::U256 {
        data: value.to_be_bytes_vec(),
    }
}

/// Convert proto U256 to U256.
pub fn proto_to_u256(value: &proto::U256) -> U256 {
    if value.data.is_empty() {
        U256::ZERO
    } else {
        U256::from_be_slice(&value.data)
    }
}

// ============================================================================
// Block ID conversions
// ============================================================================

/// Convert proto BlockId to BlockNumberOrTag.
pub fn proto_to_block_id(value: &proto::BlockId) -> Option<BlockNumberOrTag> {
    value.id.as_ref().map(|id| match id {
        proto::block_id::Id::Number(n) => BlockNumberOrTag::Number(U64::from(*n)),
        proto::block_id::Id::Tag(t) => {
            let tag = match proto::BlockTag::try_from(*t) {
                Ok(proto::BlockTag::Latest) => BlockTag::Latest,
                Ok(proto::BlockTag::Earliest) => BlockTag::Earliest,
                Ok(proto::BlockTag::Pending) => BlockTag::Pending,
                Ok(proto::BlockTag::Safe) => BlockTag::Safe,
                Ok(proto::BlockTag::Finalized) => BlockTag::Finalized,
                _ => BlockTag::Latest, // Default to latest for unspecified
            };
            BlockNumberOrTag::Tag(tag)
        }
    })
}

/// Convert BlockNumberOrTag to proto BlockId.
pub fn block_id_to_proto(value: &BlockNumberOrTag) -> proto::BlockId {
    match value {
        BlockNumberOrTag::Number(n) => proto::BlockId {
            id: Some(proto::block_id::Id::Number(n.to::<u64>())),
        },
        BlockNumberOrTag::Tag(tag) => {
            let proto_tag = match tag {
                BlockTag::Latest => proto::BlockTag::Latest,
                BlockTag::Earliest => proto::BlockTag::Earliest,
                BlockTag::Pending => proto::BlockTag::Pending,
                BlockTag::Safe => proto::BlockTag::Safe,
                BlockTag::Finalized => proto::BlockTag::Finalized,
            };
            proto::BlockId {
                id: Some(proto::block_id::Id::Tag(proto_tag as i32)),
            }
        }
    }
}

// ============================================================================
// Block conversions
// ============================================================================

/// Convert RpcBlock to proto BlockHeader.
pub fn rpc_block_to_header(block: &RpcBlock) -> proto::BlockHeader {
    proto::BlockHeader {
        number: block.number.to::<u64>(),
        hash: Some(b256_to_proto(block.hash)),
        parent_hash: Some(b256_to_proto(block.parent_hash)),
        state_root: Some(b256_to_proto(block.state_root)),
        transactions_root: Some(b256_to_proto(block.transactions_root)),
        receipts_root: Some(b256_to_proto(block.receipts_root)),
        miner: Some(address_to_proto(block.miner)),
        gas_limit: block.gas_limit.to::<u64>(),
        gas_used: block.gas_used.to::<u64>(),
        timestamp: block.timestamp.to::<u64>(),
        extra_data: block.extra_data.to_vec(),
        base_fee_per_gas: block.base_fee_per_gas.map(u256_to_proto),
        logs_bloom: block.logs_bloom.to_vec(),
    }
}

/// Convert RpcBlock to proto Block.
pub fn rpc_block_to_proto(block: &RpcBlock, full_transactions: bool) -> proto::Block {
    let header = rpc_block_to_header(block);

    let transactions = match &block.transactions {
        Some(BlockTransactions::Hashes(hashes)) if !full_transactions => Some(
            proto::block::Transactions::TxHashes(proto::TransactionHashes {
                hashes: hashes.iter().map(|h| b256_to_proto(*h)).collect(),
            }),
        ),
        Some(BlockTransactions::Full(txs)) if full_transactions => Some(
            proto::block::Transactions::FullTransactions(proto::Transactions {
                transactions: txs.iter().map(rpc_transaction_to_proto).collect(),
            }),
        ),
        Some(BlockTransactions::Hashes(hashes)) => Some(proto::block::Transactions::TxHashes(
            proto::TransactionHashes {
                hashes: hashes.iter().map(|h| b256_to_proto(*h)).collect(),
            },
        )),
        Some(BlockTransactions::Full(txs)) => Some(proto::block::Transactions::TxHashes(
            proto::TransactionHashes {
                hashes: txs.iter().map(|tx| b256_to_proto(tx.hash)).collect(),
            },
        )),
        None => None,
    };

    proto::Block {
        header: Some(header),
        transactions,
        size: block.size.to::<u64>(),
    }
}

// ============================================================================
// Transaction conversions
// ============================================================================

/// Convert RpcTransaction to proto Transaction.
pub fn rpc_transaction_to_proto(tx: &RpcTransaction) -> proto::Transaction {
    proto::Transaction {
        hash: Some(b256_to_proto(tx.hash)),
        nonce: tx.nonce.to::<u64>(),
        block_hash: tx.block_hash.map(b256_to_proto),
        block_number: tx.block_number.map(|n| n.to::<u64>()),
        transaction_index: tx.transaction_index.map(|n| n.to::<u64>()),
        from: Some(address_to_proto(tx.from)),
        to: tx.to.map(address_to_proto),
        value: Some(u256_to_proto(tx.value)),
        gas: tx.gas.to::<u64>(),
        gas_price: tx.gas_price.map(u256_to_proto),
        max_fee_per_gas: tx.max_fee_per_gas.map(u256_to_proto),
        max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(u256_to_proto),
        input: tx.input.to_vec(),
        tx_type: tx.tx_type.to::<u32>(),
        chain_id: tx.chain_id.map(|c| c.to::<u64>()),
        v: tx.v.to::<u64>(),
        r: Some(u256_to_proto(tx.r)),
        s: Some(u256_to_proto(tx.s)),
        access_list: tx
            .access_list
            .as_ref()
            .map(|al| {
                al.iter()
                    .map(|item| proto::AccessListItem {
                        address: Some(address_to_proto(item.address)),
                        storage_keys: item
                            .storage_keys
                            .iter()
                            .map(|k| b256_to_proto(*k))
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

// ============================================================================
// Receipt conversions
// ============================================================================

/// Convert RpcReceipt to proto Receipt.
pub fn rpc_receipt_to_proto(receipt: &RpcReceipt) -> proto::Receipt {
    proto::Receipt {
        transaction_hash: Some(b256_to_proto(receipt.transaction_hash)),
        transaction_index: receipt.transaction_index.to::<u64>(),
        block_hash: Some(b256_to_proto(receipt.block_hash)),
        block_number: receipt.block_number.to::<u64>(),
        from: Some(address_to_proto(receipt.from)),
        to: receipt.to.map(address_to_proto),
        cumulative_gas_used: receipt.cumulative_gas_used.to::<u64>(),
        gas_used: receipt.gas_used.to::<u64>(),
        effective_gas_price: Some(u256_to_proto(receipt.effective_gas_price)),
        contract_address: receipt.contract_address.map(address_to_proto),
        logs: receipt.logs.iter().map(rpc_log_to_proto).collect(),
        logs_bloom: receipt.logs_bloom.to_vec(),
        tx_type: receipt.tx_type.to::<u32>(),
        status: receipt.status.to::<u64>(),
    }
}

// ============================================================================
// Log conversions
// ============================================================================

/// Convert RpcLog to proto Log.
pub fn rpc_log_to_proto(log: &RpcLog) -> proto::Log {
    proto::Log {
        address: Some(address_to_proto(log.address)),
        topics: log.topics.iter().map(|t| b256_to_proto(*t)).collect(),
        data: log.data.to_vec(),
        block_number: log.block_number.map(|n| n.to::<u64>()),
        transaction_hash: log.transaction_hash.map(b256_to_proto),
        transaction_index: log.transaction_index.map(|n| n.to::<u64>()),
        block_hash: log.block_hash.map(b256_to_proto),
        log_index: log.log_index.map(|n| n.to::<u64>()),
        removed: log.removed,
    }
}

// ============================================================================
// Log filter conversions
// ============================================================================

/// Convert proto LogFilter to RpcLogFilter.
pub fn proto_to_log_filter(filter: &proto::LogFilter) -> RpcLogFilter {
    let address = if filter.addresses.is_empty() {
        None
    } else if filter.addresses.len() == 1 {
        filter
            .addresses
            .first()
            .and_then(proto_to_address)
            .map(FilterAddress::Single)
    } else {
        let addrs: Vec<Address> = filter
            .addresses
            .iter()
            .filter_map(proto_to_address)
            .collect();
        if addrs.is_empty() {
            None
        } else {
            Some(FilterAddress::Multiple(addrs))
        }
    };

    let topics = if filter.topics.is_empty() {
        None
    } else {
        let topic_filters: Vec<Option<FilterTopic>> = filter
            .topics
            .iter()
            .map(|tf| {
                if tf.topics.is_empty() {
                    None
                } else if tf.topics.len() == 1 {
                    tf.topics
                        .first()
                        .and_then(proto_to_b256)
                        .map(FilterTopic::Single)
                } else {
                    let topics: Vec<B256> = tf.topics.iter().filter_map(proto_to_b256).collect();
                    if topics.is_empty() {
                        None
                    } else {
                        Some(FilterTopic::Multiple(topics))
                    }
                }
            })
            .collect();
        Some(topic_filters)
    };

    RpcLogFilter {
        from_block: filter.from_block.as_ref().and_then(proto_to_block_id),
        to_block: filter.to_block.as_ref().and_then(proto_to_block_id),
        address,
        topics,
        block_hash: filter.block_hash.as_ref().and_then(proto_to_b256),
    }
}

// ============================================================================
// Call request conversions
// ============================================================================

/// Convert proto CallRequest to RpcCallRequest.
pub fn proto_to_call_request(req: &proto::CallRequest) -> RpcCallRequest {
    RpcCallRequest {
        from: req.from.as_ref().and_then(proto_to_address),
        to: req
            .to
            .as_ref()
            .and_then(proto_to_address)
            .unwrap_or(Address::ZERO),
        gas: req.gas.map(U64::from),
        gas_price: req.gas_price.as_ref().map(proto_to_u256),
        max_fee_per_gas: req.max_fee_per_gas.as_ref().map(proto_to_u256),
        max_priority_fee_per_gas: req.max_priority_fee_per_gas.as_ref().map(proto_to_u256),
        value: req.value.as_ref().map(proto_to_u256),
        data: req.data.as_ref().map(|d| Bytes::copy_from_slice(d)),
        input: None,
    }
}

// ============================================================================
// Sync status conversions
// ============================================================================

/// Convert RpcSyncStatus to proto SyncStatus.
pub fn rpc_sync_status_to_proto(status: &RpcSyncStatus) -> proto::SyncStatus {
    match status {
        RpcSyncStatus::NotSyncing(_) => proto::SyncStatus {
            status: Some(proto::sync_status::Status::NotSyncing(false)),
        },
        RpcSyncStatus::Syncing(progress) => proto::SyncStatus {
            status: Some(proto::sync_status::Status::Progress(proto::SyncProgress {
                starting_block: progress.starting_block.to::<u64>(),
                current_block: progress.current_block.to::<u64>(),
                highest_block: progress.highest_block.to::<u64>(),
            })),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_b256_roundtrip() {
        let original = B256::repeat_byte(0xab);
        let proto = b256_to_proto(original);
        let recovered = proto_to_b256(&proto).unwrap();
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_address_roundtrip() {
        let original = Address::repeat_byte(0xcd);
        let proto = address_to_proto(original);
        let recovered = proto_to_address(&proto).unwrap();
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_u256_roundtrip() {
        let original = U256::from(0x123456789abcdef0u64);
        let proto = u256_to_proto(original);
        let recovered = proto_to_u256(&proto);
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_u256_zero() {
        let proto = proto::U256 { data: vec![] };
        let recovered = proto_to_u256(&proto);
        assert_eq!(recovered, U256::ZERO);
    }

    #[test]
    fn test_block_tag_conversion() {
        for tag in [
            BlockTag::Latest,
            BlockTag::Earliest,
            BlockTag::Pending,
            BlockTag::Safe,
            BlockTag::Finalized,
        ] {
            let original = BlockNumberOrTag::Tag(tag);
            let proto = block_id_to_proto(&original);
            let recovered = proto_to_block_id(&proto).unwrap();
            match (&original, &recovered) {
                (BlockNumberOrTag::Tag(t1), BlockNumberOrTag::Tag(t2)) => assert_eq!(t1, t2),
                _ => panic!("Tag mismatch"),
            }
        }
    }

    #[test]
    fn test_block_number_conversion() {
        let original = BlockNumberOrTag::Number(U64::from(12345));
        let proto = block_id_to_proto(&original);
        let recovered = proto_to_block_id(&proto).unwrap();
        match (&original, &recovered) {
            (BlockNumberOrTag::Number(n1), BlockNumberOrTag::Number(n2)) => assert_eq!(n1, n2),
            _ => panic!("Number mismatch"),
        }
    }
}
