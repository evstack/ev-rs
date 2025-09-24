//! Registry for per-transaction-type signature verification.

use std::collections::BTreeMap;

use evolve_core::SdkResult;
use evolve_stf_traits::SignatureVerifier;

use crate::envelope::{tx_type, TxEnvelope};
use crate::error::ERR_UNSUPPORTED_TX_TYPE;
use crate::verifier::EcdsaVerifier;

/// Trait object for dynamic dispatch of signature verification.
pub trait SignatureVerifierDyn: Send + Sync {
    /// Verify the signature of a transaction envelope.
    fn verify(&self, tx: &TxEnvelope) -> SdkResult<()>;
}

/// Blanket implementation for any SignatureVerifier<TxEnvelope>.
impl<T: SignatureVerifier<TxEnvelope> + Send + Sync> SignatureVerifierDyn for T {
    fn verify(&self, tx: &TxEnvelope) -> SdkResult<()> {
        self.verify_signature(tx)
    }
}

/// Registry that maps transaction types to their signature verifiers.
///
/// This allows different transaction types to have different verification
/// logic while providing a unified interface.
pub struct SignatureVerifierRegistry {
    verifiers: BTreeMap<u8, Box<dyn SignatureVerifierDyn>>,
}

impl SignatureVerifierRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            verifiers: BTreeMap::new(),
        }
    }

    /// Create a registry pre-configured for Ethereum transaction types.
    ///
    /// Registers ECDSA verification for legacy (0x00) and EIP-1559 (0x02) types.
    pub fn ethereum(chain_id: u64) -> Self {
        let mut registry = Self::new();
        let verifier = EcdsaVerifier::new(chain_id);

        // All Ethereum types use ECDSA
        registry.register(tx_type::LEGACY, verifier.clone());
        registry.register(tx_type::EIP1559, verifier);

        registry
    }

    /// Register a verifier for a specific transaction type.
    pub fn register<V: SignatureVerifier<TxEnvelope> + Send + Sync + 'static>(
        &mut self,
        tx_type: u8,
        verifier: V,
    ) {
        self.verifiers.insert(tx_type, Box::new(verifier));
    }

    /// Verify a transaction's signature using the appropriate verifier.
    pub fn verify(&self, tx: &TxEnvelope) -> SdkResult<()> {
        let tx_type = tx.tx_type();
        let verifier = self.verifiers.get(&tx_type).ok_or_else(|| {
            evolve_core::ErrorCode::new_with_arg(ERR_UNSUPPORTED_TX_TYPE.id, tx_type as u16)
        })?;

        verifier.verify(tx)
    }

    /// Check if a transaction type is supported.
    pub fn supports(&self, tx_type: u8) -> bool {
        self.verifiers.contains_key(&tx_type)
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

        assert!(registry.supports(tx_type::LEGACY));
        assert!(registry.supports(tx_type::EIP1559));
        assert!(!registry.supports(tx_type::EVOLVE_BATCH));
    }

    #[test]
    fn test_empty_registry() {
        let registry = SignatureVerifierRegistry::new();
        assert!(!registry.supports(tx_type::LEGACY));
    }
}
