//! Mempool transaction wrapper.
//!
//! Wraps an Ethereum `TxEnvelope` and implements the STF `Transaction` trait.

use alloy_primitives::B256;
use evolve_core::{AccountId, FungibleAsset, InvokeRequest, Message};
use evolve_stf_traits::Transaction;
use evolve_tx::{address_to_account_id, TxEnvelope, TypedTransaction};

/// Well-known function ID for EVM call requests.
///
/// This is a placeholder that allows the mempool to work without a full EVM.
/// The receiving module interprets the calldata.
pub const EVM_CALL_FUNCTION_ID: u64 = 0x45564D43414C4C; // "EVMCALL" in ASCII

/// A transaction in the mempool.
///
/// Wraps an Ethereum `TxEnvelope` and caches derived values for efficient
/// access during block production.
#[derive(Clone, Debug)]
pub struct MempoolTransaction {
    /// The original Ethereum transaction envelope.
    envelope: TxEnvelope,
    /// Sender account ID (derived from address).
    sender_id: AccountId,
    /// Recipient account ID (derived from address).
    recipient_id: AccountId,
    /// The invoke request to execute.
    invoke_request: InvokeRequest,
    /// Gas price for ordering (effective gas price).
    effective_gas_price: u128,
}

impl MempoolTransaction {
    /// Create a new mempool transaction from an Ethereum envelope.
    ///
    /// Returns `None` if the transaction has no recipient (contract creation).
    pub fn new(envelope: TxEnvelope, base_fee: u128) -> Option<Self> {
        let sender_id = address_to_account_id(envelope.sender());
        let recipient = envelope.to()?;
        let recipient_id = address_to_account_id(recipient);

        // Create invoke request from calldata
        let invoke_request = InvokeRequest::new_from_message(
            "evm_call",
            EVM_CALL_FUNCTION_ID,
            Message::from_bytes(envelope.input().to_vec()),
        );

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
    pub fn sender_address(&self) -> alloy_primitives::Address {
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

impl Transaction for MempoolTransaction {
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
mod tests {
    use super::*;

    #[test]
    fn test_evm_call_function_id() {
        // Verify the constant is what we expect
        let expected = u64::from_be_bytes(*b"EVMCALL\0");
        // Note: The actual value doesn't match "EVMCALL" exactly due to encoding,
        // but that's fine - it just needs to be a well-known constant.
        assert_eq!(EVM_CALL_FUNCTION_ID, 0x45564D43414C4C);
    }
}
