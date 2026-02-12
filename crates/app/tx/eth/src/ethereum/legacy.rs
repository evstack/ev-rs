//! Legacy (pre-EIP-2718) transaction type.

use alloy_consensus::{Signed, TxLegacy};
use alloy_primitives::{keccak256, Address, B256, U256};
use evolve_core::{InvokeRequest, SdkResult};

use crate::ethereum::invoke_request_from_input;
use crate::ethereum::recovery::recover_sender_from_signature_hash;
use crate::traits::TypedTransaction;

/// A signed legacy Ethereum transaction with cached sender.
///
/// Legacy transactions are pre-EIP-2718 and don't have a type prefix.
/// They support EIP-155 replay protection via chain_id in the signature.
#[derive(Clone, Debug)]
pub struct SignedLegacyTx {
    /// The underlying signed transaction from alloy.
    inner: Signed<TxLegacy>,
    /// Cached sender address (recovered from signature).
    sender: Address,
    /// Cached transaction hash (keccak256 of the RLP-encoded signed tx).
    hash: B256,
}

impl SignedLegacyTx {
    /// Create from an alloy Signed<TxLegacy>, recovering the sender.
    ///
    /// The hash is computed from the provided raw bytes to ensure consistency
    /// with the original transaction encoding.
    pub fn from_alloy_with_bytes(signed: Signed<TxLegacy>, raw_bytes: &[u8]) -> SdkResult<Self> {
        let sender =
            recover_sender_from_signature_hash(signed.signature_hash(), signed.signature())?;

        // Compute transaction hash from original bytes
        // This ensures the hash matches the original encoding
        let hash = keccak256(raw_bytes);

        Ok(Self {
            inner: signed,
            sender,
            hash,
        })
    }

    /// Create from an alloy Signed<TxLegacy>, recovering the sender.
    ///
    /// Uses alloy's internal hash computation (re-encodes the transaction).
    /// Prefer `from_alloy_with_bytes` when you have the original bytes.
    pub fn from_alloy(signed: Signed<TxLegacy>) -> SdkResult<Self> {
        let sender =
            recover_sender_from_signature_hash(signed.signature_hash(), signed.signature())?;

        // Use alloy's hash (computes from re-encoded tx)
        let hash = *signed.hash();

        Ok(Self {
            inner: signed,
            sender,
            hash,
        })
    }

    /// Get a reference to the underlying transaction.
    pub fn tx(&self) -> &TxLegacy {
        self.inner.tx()
    }

    /// Get the signature.
    pub fn signature(&self) -> &alloy_primitives::PrimitiveSignature {
        self.inner.signature()
    }

    /// Encode the signed transaction to RLP bytes.
    pub fn rlp_encode(&self, out: &mut Vec<u8>) {
        self.inner.rlp_encode(out);
    }
}

impl TypedTransaction for SignedLegacyTx {
    fn tx_type(&self) -> u8 {
        0x00
    }

    fn sender(&self) -> Address {
        self.sender
    }

    fn tx_hash(&self) -> B256 {
        self.hash
    }

    fn gas_limit(&self) -> u64 {
        self.tx().gas_limit
    }

    fn chain_id(&self) -> Option<u64> {
        self.tx().chain_id
    }

    fn nonce(&self) -> u64 {
        self.tx().nonce
    }

    fn to(&self) -> Option<Address> {
        self.tx().to.to().copied()
    }

    fn value(&self) -> U256 {
        self.tx().value
    }

    fn input(&self) -> &[u8] {
        &self.tx().input
    }

    fn to_invoke_requests(&self) -> Vec<InvokeRequest> {
        if self.to().is_none() {
            return vec![];
        }

        vec![invoke_request_from_input(self.input())]
    }
}
