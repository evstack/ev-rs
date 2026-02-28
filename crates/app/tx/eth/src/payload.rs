//! Transaction payload variants.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::envelope::TxEnvelope;

/// Payload for a mempool transaction context.
///
/// - `Eoa`: Ethereum EOA transaction envelope.
/// - `Custom`: chain-specific payload bytes interpreted by a custom sender type.
#[derive(Clone, Debug)]
pub enum TxPayload {
    Eoa(Box<TxEnvelope>),
    Custom(Vec<u8>),
}

/// ETH transaction intent payload for custom sender schemes.
///
/// This preserves Ethereum transaction semantics (`to`, `data`, `value`, `nonce`,
/// gas fields, and chain ID) while decoupling sender authentication from secp256k1.
///
/// `auth_proof` is sender-type-specific material (e.g. Ed25519 signature package).
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct EthIntentPayload {
    /// Raw encoded Ethereum transaction envelope bytes.
    pub envelope: Vec<u8>,
    /// Signature/auth proof bytes interpreted by the sender-type verifier.
    pub auth_proof: Vec<u8>,
}

impl EthIntentPayload {
    pub fn decode_envelope(&self) -> evolve_core::SdkResult<TxEnvelope> {
        TxEnvelope::decode(&self.envelope)
    }
}
