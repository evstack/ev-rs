//! Ethereum transaction gateway for decode + verify.
//!
//! The gateway is responsible for:
//! 1. Decoding raw transaction bytes
//! 2. Verifying signatures and chain ID
//! 3. Producing verified `TxContext` ready for mempool insertion

use evolve_core::encoding::Decodable;
use evolve_stf_traits::TxDecoder;

use crate::decoder::TypedTxDecoder;
use crate::envelope::TxEnvelope;
use crate::mempool::TxContext;
use crate::payload::TxPayload;
use crate::sender_type;
use crate::traits::TypedTransaction;
use crate::verifier::{SignatureVerifierDyn, SignatureVerifierRegistry};

/// Error type for gateway operations.
#[derive(Debug, Clone)]
pub enum GatewayError {
    /// Failed to decode transaction.
    DecodeFailed(String),
    /// Chain ID mismatch or missing.
    InvalidChainId { expected: u64, actual: Option<u64> },
    /// Signature verification failed.
    InvalidSignature,
    /// Contract creation not supported.
    ContractCreationNotSupported,
    /// Trailing bytes after transaction.
    TrailingBytes,
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::DecodeFailed(msg) => write!(f, "decode failed: {}", msg),
            GatewayError::InvalidChainId { expected, actual } => {
                write!(
                    f,
                    "invalid chain ID: expected {}, got {:?}",
                    expected, actual
                )
            }
            GatewayError::InvalidSignature => write!(f, "invalid signature"),
            GatewayError::ContractCreationNotSupported => {
                write!(f, "contract creation not supported")
            }
            GatewayError::TrailingBytes => write!(f, "trailing bytes after transaction"),
        }
    }
}

impl std::error::Error for GatewayError {}

/// Gateway for Ethereum transactions.
///
/// Decodes raw transaction bytes and verifies signatures/chain ID,
/// producing verified `TxContext` objects ready for mempool insertion.
pub struct EthGateway {
    /// Chain ID for validation.
    chain_id: u64,
    /// Transaction decoder (type filtering).
    decoder: TypedTxDecoder,
    /// Signature/chain ID verifier registry.
    verifier: SignatureVerifierRegistry,
    /// Current base fee for effective gas price calculation.
    base_fee: u128,
}

impl EthGateway {
    /// Create a new gateway for the given chain ID.
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            decoder: TypedTxDecoder::ethereum(),
            verifier: SignatureVerifierRegistry::ethereum(chain_id),
            base_fee: 0,
        }
    }

    /// Create a new gateway with a specific base fee.
    pub fn with_base_fee(chain_id: u64, base_fee: u128) -> Self {
        Self {
            chain_id,
            decoder: TypedTxDecoder::ethereum(),
            verifier: SignatureVerifierRegistry::ethereum(chain_id),
            base_fee,
        }
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Get the current base fee.
    pub fn base_fee(&self) -> u128 {
        self.base_fee
    }

    /// Update the base fee (e.g., after a new block).
    pub fn set_base_fee(&mut self, base_fee: u128) {
        self.base_fee = base_fee;
    }

    /// Register a payload verifier for a sender type.
    pub fn register_payload_verifier(
        &mut self,
        sender_type: u16,
        verifier: impl SignatureVerifierDyn + 'static,
    ) {
        self.verifier.register_dyn(sender_type, verifier);
    }

    /// Check whether a sender type is supported by ingress verification.
    pub fn supports_sender_type(&self, sender_type: u16) -> bool {
        self.verifier.supports(sender_type)
    }

    /// Decode and verify a raw transaction.
    ///
    /// Returns a verified `TxContext` ready for mempool insertion.
    pub fn decode_and_verify(&self, raw: &[u8]) -> Result<TxContext, GatewayError> {
        if TxContext::is_wire_encoded(raw) {
            let context = TxContext::decode(raw)
                .map_err(|e| GatewayError::DecodeFailed(format!("{:?}", e)))?;
            return self.verify_context(context);
        }

        // Decode the transaction with type filtering
        let mut input = raw;
        let envelope = self
            .decoder
            .decode(&mut input)
            .map_err(|e| GatewayError::DecodeFailed(format!("{:?}", e)))?;

        if !input.is_empty() {
            return Err(GatewayError::TrailingBytes);
        }

        self.verify_envelope(envelope)
    }

    /// Verify sender-type payload for a decoded context.
    pub fn verify_context(&self, context: TxContext) -> Result<TxContext, GatewayError> {
        if let Some(id) = context.chain_id() {
            if id != self.chain_id {
                return Err(GatewayError::InvalidChainId {
                    expected: self.chain_id,
                    actual: Some(id),
                });
            }
        }

        self.verifier
            .verify_payload(context.sender_type(), context.payload())
            .map_err(|_| GatewayError::InvalidSignature)?;

        Ok(context)
    }

    /// Verify an already-decoded envelope and create a TxContext.
    pub fn verify_envelope(&self, envelope: TxEnvelope) -> Result<TxContext, GatewayError> {
        let payload = TxPayload::Eoa(Box::new(envelope.clone()));

        // Verify chain ID and signature
        self.verifier
            .verify_payload(sender_type::EOA_SECP256K1, &payload)
            .map_err(|_| match envelope.chain_id() {
                Some(id) if id != self.chain_id => GatewayError::InvalidChainId {
                    expected: self.chain_id,
                    actual: Some(id),
                },
                None => GatewayError::InvalidChainId {
                    expected: self.chain_id,
                    actual: None,
                },
                _ => GatewayError::InvalidSignature,
            })?;

        // Create verified transaction context
        TxContext::new(envelope, self.base_fee).ok_or(GatewayError::ContractCreationNotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_with_base_fee() {
        let gateway = EthGateway::with_base_fee(1337, 1000);
        assert_eq!(gateway.chain_id(), 1337);
        assert_eq!(gateway.base_fee(), 1000);
    }

    #[test]
    fn test_set_base_fee() {
        let mut gateway = EthGateway::new(1337);
        gateway.set_base_fee(500);
        assert_eq!(gateway.base_fee(), 500);
    }

    #[test]
    fn test_empty_input_fails() {
        let gateway = EthGateway::new(1337);
        let result = gateway.decode_and_verify(&[]);
        assert!(matches!(result, Err(GatewayError::DecodeFailed(_))));
    }

    #[test]
    fn test_invalid_input_fails() {
        let gateway = EthGateway::new(1337);
        let result = gateway.decode_and_verify(&[0xFF, 0xFF, 0xFF]);
        assert!(matches!(result, Err(GatewayError::DecodeFailed(_))));
    }
}
