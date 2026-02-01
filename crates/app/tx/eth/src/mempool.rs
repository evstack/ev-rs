//! Mempool transaction wrapper for Ethereum transactions.
//!
//! Wraps an Ethereum `TxEnvelope` and implements the `MempoolTx` trait
//! for use with the generic mempool.
//!
//! # Function Routing
//!
//! Routing is defined by `evolve_tx_eth` and derived from Ethereum calldata.
//! See `TxEnvelope::to_invoke_requests()` for the canonical mapping.
//!
//! Example: A CLOB account can expose `place_order`, `cancel_order`, etc., each with
//! their own Ethereum selector computed as `keccak256(signature)[0..4]`.

use alloy_primitives::{Address, B256};
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{AccountId, FungibleAsset, InvokeRequest, SdkResult};
use evolve_mempool::{GasPriceOrdering, MempoolTx};
use evolve_stf_traits::Transaction;

use crate::envelope::TxEnvelope;
use crate::traits::{address_to_account_id, TypedTransaction};

/// A verified transaction ready for mempool storage.
///
/// Wraps an Ethereum `TxEnvelope` and caches derived values for efficient
/// access during block production.
#[derive(Clone, Debug)]
pub struct TxContext {
    /// The original Ethereum transaction envelope.
    envelope: TxEnvelope,
    /// Sender account ID (derived from address).
    sender_id: AccountId,
    /// Recipient account ID (derived from address).
    recipient_id: AccountId,
    /// The invoke request to execute (derived by evolve_tx).
    invoke_request: InvokeRequest,
    /// Gas price for ordering (effective gas price).
    effective_gas_price: u128,
}

impl TxContext {
    /// Create a new mempool transaction from an Ethereum envelope.
    ///
    /// Returns `None` if the transaction has no recipient (contract creation).
    pub fn new(envelope: TxEnvelope, base_fee: u128) -> Option<Self> {
        let sender_id = address_to_account_id(envelope.sender());
        let recipient = envelope.to()?;
        let recipient_id = address_to_account_id(recipient);

        let invoke_request = envelope.to_invoke_requests().into_iter().next()?;

        // Calculate effective gas price for ordering
        let effective_gas_price = calculate_effective_gas_price(&envelope, base_fee);

        Some(Self {
            envelope,
            sender_id,
            recipient_id,
            invoke_request,
            effective_gas_price,
        })
    }

    /// Get the transaction hash.
    pub fn hash(&self) -> B256 {
        self.envelope.tx_hash()
    }

    /// Get the sender address.
    pub fn sender_address(&self) -> Address {
        self.envelope.sender()
    }

    /// Get the nonce.
    pub fn nonce(&self) -> u64 {
        self.envelope.nonce()
    }

    /// Get the effective gas price for ordering.
    pub fn effective_gas_price(&self) -> u128 {
        self.effective_gas_price
    }

    /// Get the underlying envelope.
    pub fn envelope(&self) -> &TxEnvelope {
        &self.envelope
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> Option<u64> {
        self.envelope.chain_id()
    }
}

impl MempoolTx for TxContext {
    type OrderingKey = GasPriceOrdering;

    fn tx_id(&self) -> [u8; 32] {
        self.envelope.tx_hash().0
    }

    fn ordering_key(&self) -> Self::OrderingKey {
        GasPriceOrdering::new(self.effective_gas_price, self.nonce())
    }

    fn sender_key(&self) -> Option<[u8; 20]> {
        Some(self.envelope.sender().0 .0)
    }

    fn gas_limit(&self) -> u64 {
        self.envelope.gas_limit()
    }
}

impl Transaction for TxContext {
    fn sender(&self) -> AccountId {
        self.sender_id
    }

    fn recipient(&self) -> AccountId {
        self.recipient_id
    }

    fn request(&self) -> &InvokeRequest {
        &self.invoke_request
    }

    fn gas_limit(&self) -> u64 {
        self.envelope.gas_limit()
    }

    fn funds(&self) -> &[FungibleAsset] {
        // TODO: Convert value transfer to FungibleAsset when native token is defined
        &[]
    }

    fn compute_identifier(&self) -> [u8; 32] {
        self.envelope.tx_hash().0
    }
}

impl Encodable for TxContext {
    fn encode(&self) -> SdkResult<Vec<u8>> {
        Ok(self.envelope.encode())
    }
}

impl Decodable for TxContext {
    fn decode(bytes: &[u8]) -> SdkResult<Self> {
        let envelope = TxEnvelope::decode(bytes)?;
        // Use base_fee of 0 for decoding - the effective gas price will be
        // recalculated if needed when the transaction is added to a mempool
        TxContext::new(envelope, 0).ok_or_else(|| {
            evolve_core::ErrorCode::new(0x50) // Contract creation not supported
        })
    }
}

/// Calculate effective gas price for transaction ordering.
///
/// - Legacy: gas_price
/// - EIP-1559: min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)
fn calculate_effective_gas_price(envelope: &TxEnvelope, base_fee: u128) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => {
            // Legacy transactions have a fixed gas price
            tx.tx().gas_price
        }
        TxEnvelope::Eip1559(tx) => {
            let max_fee = tx.max_fee_per_gas();
            let priority_fee = tx.max_priority_fee_per_gas();
            // Effective = min(max_fee, base_fee + priority_fee)
            max_fee.min(base_fee.saturating_add(priority_fee))
        }
    }
}

#[cfg(test)]
mod tests {}
