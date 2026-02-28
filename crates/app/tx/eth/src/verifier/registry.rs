//! Registry for sender-type-based signature verification.

use std::collections::BTreeMap;

use evolve_core::SdkResult;
use evolve_stf_traits::SignatureVerifier;

use crate::envelope::TxEnvelope;
use crate::error::{ERR_INVALID_SIGNATURE, ERR_UNSUPPORTED_SENDER_TYPE};
use crate::payload::TxPayload;
use crate::sender_type;
use crate::verifier::EcdsaVerifier;

/// Trait object for dynamic dispatch of signature verification by payload.
pub trait SignatureVerifierDyn: Send + Sync {
    /// Verify the signature/auth proof for a payload.
    fn verify(&self, payload: &TxPayload) -> SdkResult<()>;
}

/// Adapter for verifiers that operate on Ethereum envelopes.
struct EoaPayloadVerifier<V> {
    inner: V,
}

impl<V> EoaPayloadVerifier<V> {
    fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V> SignatureVerifierDyn for EoaPayloadVerifier<V>
where
    V: SignatureVerifier<TxEnvelope> + Send + Sync,
{
    fn verify(&self, payload: &TxPayload) -> SdkResult<()> {
        match payload {
            TxPayload::Eoa(tx) => self.inner.verify_signature(tx.as_ref()),
            TxPayload::Custom(_) => Err(ERR_INVALID_SIGNATURE),
        }
    }
}

/// Registry that maps sender types to signature verifiers.
///
/// This allows decoupling sender authentication from tx envelope types.
pub struct SignatureVerifierRegistry {
    verifiers: BTreeMap<u16, Box<dyn SignatureVerifierDyn>>,
}

impl SignatureVerifierRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            verifiers: BTreeMap::new(),
        }
    }

    /// Create a registry pre-configured for Ethereum EOA sender authentication.
    pub fn ethereum(chain_id: u64) -> Self {
        let mut registry = Self::new();
        registry.register_eoa(sender_type::EOA_SECP256K1, EcdsaVerifier::new(chain_id));
        registry
    }

    /// Register a payload verifier for a sender type.
    pub fn register_dyn(
        &mut self,
        sender_type: u16,
        verifier: impl SignatureVerifierDyn + 'static,
    ) {
        self.verifiers.insert(sender_type, Box::new(verifier));
    }

    /// Register an envelope-based verifier for an EOA sender type.
    pub fn register_eoa<V>(&mut self, sender_type: u16, verifier: V)
    where
        V: SignatureVerifier<TxEnvelope> + Send + Sync + 'static,
    {
        self.register_dyn(sender_type, EoaPayloadVerifier::new(verifier));
    }

    /// Verify a payload using the verifier configured for `sender_type`.
    pub fn verify_payload(&self, sender_type: u16, payload: &TxPayload) -> SdkResult<()> {
        let verifier = self.verifiers.get(&sender_type).ok_or_else(|| {
            evolve_core::ErrorCode::new_with_arg(ERR_UNSUPPORTED_SENDER_TYPE.id, sender_type)
        })?;
        verifier.verify(payload)
    }

    /// Backward-compatible verification entrypoint for Ethereum EOA envelopes.
    pub fn verify(&self, tx: &TxEnvelope) -> SdkResult<()> {
        self.verify_payload(
            sender_type::EOA_SECP256K1,
            &TxPayload::Eoa(Box::new(tx.clone())),
        )
    }

    /// Check if a sender type is supported.
    pub fn supports(&self, sender_type: u16) -> bool {
        self.verifiers.contains_key(&sender_type)
    }
}

impl Default for SignatureVerifierRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SignatureVerifier<TxEnvelope> for SignatureVerifierRegistry {
    fn verify_signature(&self, tx: &TxEnvelope) -> SdkResult<()> {
        self.verify(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_supports() {
        let registry = SignatureVerifierRegistry::ethereum(1);
        assert!(registry.supports(sender_type::EOA_SECP256K1));
        assert!(!registry.supports(sender_type::CUSTOM));
    }

    #[test]
    fn test_empty_registry() {
        let registry = SignatureVerifierRegistry::new();
        assert!(!registry.supports(sender_type::EOA_SECP256K1));
    }
}
