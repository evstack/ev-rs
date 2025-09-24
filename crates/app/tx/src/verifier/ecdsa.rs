//! ECDSA signature verification for Ethereum transactions.

use evolve_core::SdkResult;
use evolve_stf_traits::SignatureVerifier;

use crate::envelope::TxEnvelope;
use crate::error::ERR_INVALID_CHAIN_ID;
use crate::traits::TypedTransaction;

/// ECDSA signature verifier for Ethereum transaction types.
///
/// Verifies that:
/// 1. The signature is valid (sender was already recovered during decoding)
/// 2. The chain ID matches the expected value (EIP-155 replay protection)
///
/// Note: Signature validity is already checked during transaction decoding
/// when we recover the sender. This verifier primarily enforces chain ID.
#[derive(Clone, Debug)]
pub struct EcdsaVerifier {
    /// Expected chain ID for replay protection.
    chain_id: u64,
    /// Whether to require chain ID (reject legacy txs without EIP-155).
    require_chain_id: bool,
}

impl EcdsaVerifier {
    /// Create a new ECDSA verifier with the expected chain ID.
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            require_chain_id: true,
        }
    }

    /// Create a verifier that allows legacy transactions without chain ID.
    pub fn new_permissive(chain_id: u64) -> Self {
        Self {
            chain_id,
            require_chain_id: false,
        }
    }

    /// Verify chain ID matches for a transaction.
    pub fn verify_chain_id_for_tx(&self, tx: &TxEnvelope) -> SdkResult<()> {
        self.verify_chain_id(tx.chain_id())
    }

    /// Verify chain ID matches.
    fn verify_chain_id(&self, tx_chain_id: Option<u64>) -> SdkResult<()> {
        match tx_chain_id {
            Some(id) if id == self.chain_id => Ok(()),
            Some(_) => Err(ERR_INVALID_CHAIN_ID),
            None if self.require_chain_id => Err(ERR_INVALID_CHAIN_ID),
            None => Ok(()), // Legacy tx without EIP-155, allowed in permissive mode
        }
    }
}

impl SignatureVerifier<TxEnvelope> for EcdsaVerifier {
    fn verify_signature(&self, tx: &TxEnvelope) -> SdkResult<()> {
        // Signature validity is already verified during decoding (sender recovery).
        // Here we just verify chain ID for replay protection.
        self.verify_chain_id(tx.chain_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_id_verification() {
        let verifier = EcdsaVerifier::new(1); // Mainnet

        // Matching chain ID
        assert!(verifier.verify_chain_id(Some(1)).is_ok());

        // Mismatched chain ID
        assert!(verifier.verify_chain_id(Some(5)).is_err());

        // Missing chain ID (strict mode)
        assert!(verifier.verify_chain_id(None).is_err());
    }

    #[test]
    fn test_permissive_mode() {
        let verifier = EcdsaVerifier::new_permissive(1);

        // Missing chain ID allowed in permissive mode
        assert!(verifier.verify_chain_id(None).is_ok());

        // Mismatched still rejected
        assert!(verifier.verify_chain_id(Some(5)).is_err());
    }
}
