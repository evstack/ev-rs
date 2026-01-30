//! EIP-1559 fee market transaction type.

use alloy_consensus::{Signed, TxEip1559};
use alloy_primitives::{keccak256, Address, B256, U256};
use evolve_core::{InvokeRequest, SdkResult};

use crate::envelope::tx_type;
use crate::error::ERR_SIGNATURE_RECOVERY;
use crate::ethereum::invoke_request_from_input;
use crate::traits::TypedTransaction;

/// A signed EIP-1559 transaction with cached sender.
///
/// EIP-1559 transactions include:
/// - Base fee and priority fee (tip) for gas pricing
/// - Access lists for gas optimization
/// - Chain ID is mandatory (always has replay protection)
#[derive(Clone, Debug)]
pub struct SignedEip1559Tx {
    /// The underlying signed transaction from alloy.
    inner: Signed<TxEip1559>,
    /// Cached sender address (recovered from signature).
    sender: Address,
    /// Cached transaction hash (keccak256 of type prefix + RLP).
    hash: B256,
}

impl SignedEip1559Tx {
    /// Create from an alloy Signed<TxEip1559>, recovering the sender.
    ///
    /// The hash is computed from the provided raw RLP bytes (without type prefix).
    /// The type prefix is added internally for correct hash computation.
    pub fn from_alloy_with_bytes(signed: Signed<TxEip1559>, rlp_bytes: &[u8]) -> SdkResult<Self> {
        // Recover sender from signature
        let sender = signed
            .recover_signer()
            .map_err(|_| ERR_SIGNATURE_RECOVERY)?;

        // Compute transaction hash: keccak256(type_prefix || rlp_bytes)
        // EIP-2718 typed transaction hash includes the type byte
        let mut hash_input = Vec::with_capacity(1 + rlp_bytes.len());
        hash_input.push(tx_type::EIP1559);
        hash_input.extend_from_slice(rlp_bytes);
        let hash = keccak256(&hash_input);

        Ok(Self {
            inner: signed,
            sender,
            hash,
        })
    }

    /// Create from an alloy Signed<TxEip1559>, recovering the sender.
    ///
    /// Uses alloy's internal hash computation (re-encodes the transaction).
    /// Prefer `from_alloy_with_bytes` when you have the original bytes.
    pub fn from_alloy(signed: Signed<TxEip1559>) -> SdkResult<Self> {
        // Recover sender from signature
        let sender = signed
            .recover_signer()
            .map_err(|_| ERR_SIGNATURE_RECOVERY)?;

        // Use alloy's hash (computes from re-encoded tx)
        let hash = *signed.hash();

        Ok(Self {
            inner: signed,
            sender,
            hash,
        })
    }

    /// Get a reference to the underlying transaction.
    pub fn tx(&self) -> &TxEip1559 {
        self.inner.tx()
    }

    /// Get the signature.
    pub fn signature(&self) -> &alloy_primitives::PrimitiveSignature {
        self.inner.signature()
    }

    /// Get the max fee per gas.
    pub fn max_fee_per_gas(&self) -> u128 {
        self.tx().max_fee_per_gas
    }

    /// Get the max priority fee per gas (tip).
    pub fn max_priority_fee_per_gas(&self) -> u128 {
        self.tx().max_priority_fee_per_gas
    }

    /// Get the access list.
    pub fn access_list(&self) -> &alloy_eips::eip2930::AccessList {
        &self.tx().access_list
    }

    /// Encode the signed transaction to RLP bytes.
    pub fn rlp_encode(&self, out: &mut Vec<u8>) {
        self.inner.rlp_encode(out);
    }
}

impl TypedTransaction for SignedEip1559Tx {
    fn tx_type(&self) -> u8 {
        0x02
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
        // EIP-1559 always has chain_id
        Some(self.tx().chain_id)
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
