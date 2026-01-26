//! Mempool transaction wrapper.
//!
//! Wraps an Ethereum `TxEnvelope` and implements the STF `Transaction` trait.
//!
//! # Function Routing
//!
//! Transactions are routed to recipient accounts based on the Ethereum function selector
//! (first 4 bytes of calldata). This allows accounts to expose multiple entry points
//! that can be called directly from Ethereum transactions.
//!
//! Example: A CLOB account can expose `place_order`, `cancel_order`, etc., each with
//! their own Ethereum selector computed as `keccak256(signature)[0..4]`.

use alloy_primitives::B256;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{AccountId, FungibleAsset, InvokeRequest, Message, SdkResult};
use evolve_stf_traits::Transaction;
use evolve_tx::{address_to_account_id, TxEnvelope, TypedTransaction};

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
    ///
    /// # Function Routing
    ///
    /// The first 4 bytes of calldata are interpreted as an Ethereum function selector.
    /// This selector is converted to a u64 function ID for dispatch. The remaining
    /// calldata bytes are passed as the message payload (Borsh-encoded as `Vec<u8>`).
    ///
    /// For empty calldata (plain value transfers), function ID is 0.
    pub fn new(envelope: TxEnvelope, base_fee: u128) -> Option<Self> {
        let sender_id = address_to_account_id(envelope.sender());
        let recipient = envelope.to()?;
        let recipient_id = address_to_account_id(recipient);

        let input = envelope.input();

        // Extract function selector (first 4 bytes) and args (remaining bytes)
        let (function_id, args) = if input.len() >= 4 {
            // Function selector is first 4 bytes as big-endian u32, extended to u64
            let selector = u32::from_be_bytes([input[0], input[1], input[2], input[3]]) as u64;
            (selector, &input[4..])
        } else {
            // No selector: plain value transfer or empty call
            // Use function ID 0 for receive/fallback
            (0u64, input)
        };

        // Pass args directly - they're already Borsh-encoded by the caller
        let invoke_request = InvokeRequest::new_from_message(
            "eth_dispatch", // Generic name, actual dispatch by function_id
            function_id,
            Message::from_bytes(args.to_vec()),
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

impl Encodable for MempoolTransaction {
    fn encode(&self) -> SdkResult<Vec<u8>> {
        Ok(self.envelope.encode())
    }
}

impl Decodable for MempoolTransaction {
    fn decode(bytes: &[u8]) -> SdkResult<Self> {
        let envelope = TxEnvelope::decode(bytes)?;
        // Use base_fee of 0 for decoding - the effective gas price will be
        // recalculated if needed when the transaction is added to a mempool
        MempoolTransaction::new(envelope, 0).ok_or_else(|| {
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
mod tests {

    #[test]
    fn test_selector_extraction() {
        // Verify that selector extraction works correctly
        // keccak256("transfer(address,uint256)")[0..4] = 0xa9059cbb
        let expected_selector: u64 = 0xa9059cbb;
        let selector_bytes = [0xa9, 0x05, 0x9c, 0xbb];
        let computed = u32::from_be_bytes(selector_bytes) as u64;
        assert_eq!(computed, expected_selector);
    }
}
