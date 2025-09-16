//! JSON-RPC API trait definitions using jsonrpsee.
//!
//! This module defines the Ethereum-compatible JSON-RPC API that the server implements.

use alloy_primitives::{Address, Bytes, B256, U256, U64};
use evolve_rpc_types::{
    BlockNumberOrTag, CallRequest, FeeHistory, LogFilter, RpcBlock, RpcLog, RpcReceipt,
    RpcTransaction, SyncStatus,
};
use jsonrpsee::proc_macros::rpc;

/// Ethereum namespace RPC API.
///
/// This trait defines the standard Ethereum JSON-RPC methods that wallets and tooling expect.
#[rpc(server, namespace = "eth")]
pub trait EthApi {
    /// Returns the current chain ID.
    #[method(name = "chainId")]
    async fn chain_id(&self) -> Result<U64, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the number of the most recent block.
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> Result<U64, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the balance of the account at the given address.
    #[method(name = "getBalance")]
    async fn get_balance(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U256, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the number of transactions sent from an address (nonce).
    #[method(name = "getTransactionCount")]
    async fn get_transaction_count(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U64, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns block information by block number.
    #[method(name = "getBlockByNumber")]
    async fn get_block_by_number(
        &self,
        block: BlockNumberOrTag,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns block information by block hash.
    #[method(name = "getBlockByHash")]
    async fn get_block_by_hash(
        &self,
        hash: B256,
        full_transactions: bool,
    ) -> Result<Option<RpcBlock>, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns transaction information by transaction hash.
    #[method(name = "getTransactionByHash")]
    async fn get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<RpcTransaction>, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the receipt of a transaction by transaction hash.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<RpcReceipt>, jsonrpsee::types::ErrorObjectOwned>;

    /// Executes a call without creating a transaction (dry-run).
    #[method(name = "call")]
    async fn call(
        &self,
        request: CallRequest,
        block: Option<BlockNumberOrTag>,
    ) -> Result<Bytes, jsonrpsee::types::ErrorObjectOwned>;

    /// Estimates the gas needed to execute a transaction.
    #[method(name = "estimateGas")]
    async fn estimate_gas(
        &self,
        request: CallRequest,
        block: Option<BlockNumberOrTag>,
    ) -> Result<U64, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the current gas price in wei.
    #[method(name = "gasPrice")]
    async fn gas_price(&self) -> Result<U256, jsonrpsee::types::ErrorObjectOwned>;

    /// Submits a raw transaction to the network.
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(
        &self,
        data: Bytes,
    ) -> Result<B256, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns logs matching the given filter.
    #[method(name = "getLogs")]
    async fn get_logs(
        &self,
        filter: LogFilter,
    ) -> Result<Vec<RpcLog>, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the current sync status.
    #[method(name = "syncing")]
    async fn syncing(&self) -> Result<SyncStatus, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the current client version.
    #[method(name = "protocolVersion")]
    async fn protocol_version(&self) -> Result<String, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns code at a given address.
    #[method(name = "getCode")]
    async fn get_code(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> Result<Bytes, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the value from a storage position at a given address.
    #[method(name = "getStorageAt")]
    async fn get_storage_at(
        &self,
        address: Address,
        position: U256,
        block: Option<BlockNumberOrTag>,
    ) -> Result<B256, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns fee history for the requested block range.
    #[method(name = "feeHistory")]
    async fn fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> Result<FeeHistory, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the current max priority fee per gas.
    #[method(name = "maxPriorityFeePerGas")]
    async fn max_priority_fee_per_gas(&self) -> Result<U256, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the number of transactions in a block by block number.
    #[method(name = "getBlockTransactionCountByNumber")]
    async fn get_block_transaction_count_by_number(
        &self,
        block: BlockNumberOrTag,
    ) -> Result<Option<U64>, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the number of transactions in a block by block hash.
    #[method(name = "getBlockTransactionCountByHash")]
    async fn get_block_transaction_count_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<U64>, jsonrpsee::types::ErrorObjectOwned>;
}

/// Web3 namespace RPC API.
#[rpc(server, namespace = "web3")]
pub trait Web3Api {
    /// Returns the current client version.
    #[method(name = "clientVersion")]
    async fn client_version(&self) -> Result<String, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns Keccak-256 hash of the given data.
    #[method(name = "sha3")]
    async fn sha3(&self, data: Bytes) -> Result<B256, jsonrpsee::types::ErrorObjectOwned>;
}

/// Net namespace RPC API.
#[rpc(server, namespace = "net")]
pub trait NetApi {
    /// Returns the current network ID.
    #[method(name = "version")]
    async fn version(&self) -> Result<String, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns true if client is actively listening for network connections.
    #[method(name = "listening")]
    async fn listening(&self) -> Result<bool, jsonrpsee::types::ErrorObjectOwned>;

    /// Returns the number of peers currently connected.
    #[method(name = "peerCount")]
    async fn peer_count(&self) -> Result<U64, jsonrpsee::types::ErrorObjectOwned>;
}
