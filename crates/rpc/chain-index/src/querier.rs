//! State querier for reading account balances and nonces from storage.
//!
//! This module provides direct storage reads for RPC state queries
//! (eth_getBalance, eth_getTransactionCount) without going through
//! the full STF execution pipeline.

use alloy_primitives::{Address, Bytes, U256};
use async_trait::async_trait;

use evolve_core::encoding::Encodable;
use evolve_core::{AccountId, Message, ReadonlyKV};
use evolve_eth_jsonrpc::error::RpcError;
use evolve_rpc_types::CallRequest;
use evolve_tx_eth::{lookup_account_id_in_storage, lookup_contract_account_id_in_storage};

/// Trait for querying on-chain state (balance, nonce).
///
/// Implementors hold a reference to storage and know how to
/// map Ethereum addresses to Evolve account state.
#[async_trait]
pub trait StateQuerier: Send + Sync {
    /// Get the token balance for an Ethereum address.
    async fn get_balance(&self, address: Address) -> Result<U256, RpcError>;

    /// Get the transaction count (nonce) for an Ethereum address.
    async fn get_transaction_count(&self, address: Address) -> Result<u64, RpcError>;

    /// Execute a read-only call.
    async fn call(&self, request: &CallRequest) -> Result<Bytes, RpcError>;

    /// Estimate gas for a transaction.
    async fn estimate_gas(&self, request: &CallRequest) -> Result<u64, RpcError>;
}

/// State querier that reads directly from storage.
///
/// Uses the known storage key layout to read token balances and nonces
/// without invoking the STF. This is the same key format used by the
/// `#[account_impl]` macro:
/// - Nonce: `account_id_bytes ++ [0]` (EthEoaAccount storage prefix 0)
/// - Balance: `token_id_bytes ++ [1] ++ encode(account_id)` (Token storage prefix 1)
pub struct StorageStateQuerier<S> {
    storage: S,
    token_account_id: AccountId,
}

impl<S: ReadonlyKV + Send + Sync> StorageStateQuerier<S> {
    pub fn new(storage: S, token_account_id: AccountId) -> Self {
        Self {
            storage,
            token_account_id,
        }
    }

    fn read_nonce(&self, account_id: AccountId) -> Result<u64, RpcError> {
        let mut key = account_id.as_bytes().to_vec();
        key.push(0u8); // EthEoaAccount::nonce storage prefix
        match self
            .storage
            .get(&key)
            .map_err(|e| RpcError::InternalError(format!("storage read: {:?}", e)))?
        {
            Some(value) => Message::from_bytes(value)
                .get::<u64>()
                .map_err(|e| RpcError::InternalError(format!("decode nonce: {:?}", e))),
            None => Ok(0),
        }
    }

    fn read_balance(&self, account_id: AccountId) -> Result<u128, RpcError> {
        let mut key = self.token_account_id.as_bytes().to_vec();
        key.push(1u8); // Token::balances storage prefix
        key.extend(
            account_id
                .encode()
                .map_err(|e| RpcError::InternalError(format!("encode account id: {:?}", e)))?,
        );
        match self
            .storage
            .get(&key)
            .map_err(|e| RpcError::InternalError(format!("storage read: {:?}", e)))?
        {
            Some(value) => Message::from_bytes(value)
                .get::<u128>()
                .map_err(|e| RpcError::InternalError(format!("decode balance: {:?}", e))),
            None => Ok(0),
        }
    }

    fn resolve_account_id(&self, address: Address) -> Result<Option<AccountId>, RpcError> {
        if let Some(account_id) = lookup_account_id_in_storage(&self.storage, address)
            .map_err(|e| RpcError::InternalError(format!("lookup account id: {:?}", e)))?
        {
            return Ok(Some(account_id));
        }

        lookup_contract_account_id_in_storage(&self.storage, address)
            .map_err(|e| RpcError::InternalError(format!("lookup contract account id: {:?}", e)))
    }
}

#[async_trait]
impl<S: ReadonlyKV + Send + Sync> StateQuerier for StorageStateQuerier<S> {
    async fn get_balance(&self, address: Address) -> Result<U256, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(U256::ZERO);
        };
        let balance = self.read_balance(account_id)?;
        Ok(U256::from(balance))
    }

    async fn get_transaction_count(&self, address: Address) -> Result<u64, RpcError> {
        let Some(account_id) = self.resolve_account_id(address)? else {
            return Ok(0);
        };
        self.read_nonce(account_id)
    }

    async fn call(&self, _request: &CallRequest) -> Result<Bytes, RpcError> {
        // TODO: Implement via STF::query()
        Ok(Bytes::new())
    }

    async fn estimate_gas(&self, _request: &CallRequest) -> Result<u64, RpcError> {
        // TODO: Implement via STF with gas tracking
        Ok(21000)
    }
}
