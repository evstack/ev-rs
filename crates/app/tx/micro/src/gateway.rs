//! Micro transaction gateway for decode + verify.
//!
//! The gateway is responsible for:
//! 1. Decoding raw transaction bytes (signature is verified during decode)
//! 2. Verifying chain ID
//! 3. Producing verified `MicroTxContext` ready for mempool insertion

use crate::mempool::MicroTxContext;
use crate::{MicroTx, MicroTxError, SignedMicroTx};

/// Error type for micro gateway operations.
#[derive(Debug, Clone)]
pub enum MicroGatewayError {
    /// Failed to decode/verify transaction.
    DecodeFailed(MicroTxError),
    /// Chain ID mismatch.
    InvalidChainId { expected: u64, actual: u64 },
}

impl std::fmt::Display for MicroGatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MicroGatewayError::DecodeFailed(e) => write!(f, "decode failed: {:?}", e),
            MicroGatewayError::InvalidChainId { expected, actual } => {
                write!(f, "invalid chain ID: expected {}, got {}", expected, actual)
            }
        }
    }
}

impl std::error::Error for MicroGatewayError {}

impl From<MicroTxError> for MicroGatewayError {
    fn from(e: MicroTxError) -> Self {
        MicroGatewayError::DecodeFailed(e)
    }
}

/// Gateway for micro transactions.
///
/// Decodes raw transaction bytes and verifies chain ID.
/// Note: Signature verification happens during decode for micro transactions.
pub struct MicroGateway {
    /// Chain ID for validation.
    chain_id: u64,
}

impl MicroGateway {
    /// Create a new gateway for the given chain ID.
    pub fn new(chain_id: u64) -> Self {
        Self { chain_id }
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Decode and verify a raw micro transaction.
    ///
    /// Returns a verified `MicroTxContext` ready for mempool insertion.
    pub fn decode_and_verify(&self, raw: &[u8]) -> Result<MicroTxContext, MicroGatewayError> {
        // Decode the transaction (signature is verified during decode)
        let tx = SignedMicroTx::decode(raw)?;

        // Verify chain ID
        if tx.chain_id() != self.chain_id {
            return Err(MicroGatewayError::InvalidChainId {
                expected: self.chain_id,
                actual: tx.chain_id(),
            });
        }

        Ok(MicroTxContext::new(tx))
    }

    /// Accept an already-decoded transaction with chain ID verification.
    pub fn accept(&self, tx: SignedMicroTx) -> Result<MicroTxContext, MicroGatewayError> {
        // Verify chain ID
        if tx.chain_id() != self.chain_id {
            return Err(MicroGatewayError::InvalidChainId {
                expected: self.chain_id,
                actual: tx.chain_id(),
            });
        }

        Ok(MicroTxContext::new(tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_unsigned_payload, hash_for_signing};
    use alloy_primitives::Address;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn create_test_tx(chain_id: u64) -> Vec<u8> {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let from_pubkey: [u8; 32] = verifying_key.to_bytes();

        let to = Address::repeat_byte(0x22);
        let amount = 1000u128;
        let timestamp = 1234567890u64;

        let payload = create_unsigned_payload(chain_id, &from_pubkey, to, amount, timestamp, &[]);
        let signing_hash = hash_for_signing(&payload);
        let signature = signing_key.sign(signing_hash.as_slice());

        let mut tx_bytes = payload;
        tx_bytes.extend_from_slice(&signature.to_bytes());
        tx_bytes
    }

    #[test]
    fn test_gateway_new() {
        let gateway = MicroGateway::new(1337);
        assert_eq!(gateway.chain_id(), 1337);
    }

    #[test]
    fn test_decode_and_verify_success() {
        let chain_id = 1337u64;
        let gateway = MicroGateway::new(chain_id);
        let tx_bytes = create_test_tx(chain_id);

        let result = gateway.decode_and_verify(&tx_bytes);
        assert!(result.is_ok());

        let ctx = result.unwrap();
        assert_eq!(ctx.inner().chain_id(), chain_id);
    }

    #[test]
    fn test_decode_and_verify_wrong_chain_id() {
        let gateway = MicroGateway::new(1337);
        let tx_bytes = create_test_tx(9999); // Different chain ID

        let result = gateway.decode_and_verify(&tx_bytes);
        assert!(matches!(
            result,
            Err(MicroGatewayError::InvalidChainId {
                expected: 1337,
                actual: 9999
            })
        ));
    }

    #[test]
    fn test_decode_and_verify_invalid_input() {
        let gateway = MicroGateway::new(1337);
        let result = gateway.decode_and_verify(&[0xFF; 10]);
        assert!(matches!(result, Err(MicroGatewayError::DecodeFailed(_))));
    }
}
