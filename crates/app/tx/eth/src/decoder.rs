//! Transaction decoder implementing the TxDecoder trait.

use std::collections::BTreeSet;

use evolve_core::SdkResult;
use evolve_stf_traits::TxDecoder;

use crate::envelope::{tx_type, TxEnvelope};
use crate::error::ERR_UNSUPPORTED_TX_TYPE;

/// Configurable decoder for typed transactions.
///
/// Allows restricting which transaction types are accepted,
/// useful for chains that only want to support specific types.
#[derive(Clone, Debug)]
pub struct TypedTxDecoder {
    /// Set of allowed transaction types.
    allowed_types: BTreeSet<u8>,
}

impl TypedTxDecoder {
    /// Create a decoder that accepts all Ethereum standard types.
    pub fn ethereum() -> Self {
        let mut allowed = BTreeSet::new();
        allowed.insert(tx_type::LEGACY);
        allowed.insert(tx_type::EIP1559);
        Self {
            allowed_types: allowed,
        }
    }

    /// Create a decoder that accepts specific transaction types.
    pub fn with_types(types: impl IntoIterator<Item = u8>) -> Self {
        Self {
            allowed_types: types.into_iter().collect(),
        }
    }

    /// Create a decoder that accepts all types (no filtering).
    pub fn permissive() -> Self {
        // Include all possible types
        Self {
            allowed_types: (0..=255).collect(),
        }
    }

    /// Add a transaction type to the allowed set.
    pub fn allow_type(&mut self, tx_type: u8) -> &mut Self {
        self.allowed_types.insert(tx_type);
        self
    }

    /// Check if a transaction type is allowed.
    pub fn is_allowed(&self, tx_type: u8) -> bool {
        self.allowed_types.contains(&tx_type)
    }
}

impl TxDecoder<TxEnvelope> for TypedTxDecoder {
    fn decode(&self, bytes: &mut &[u8]) -> SdkResult<TxEnvelope> {
        // Decode the transaction
        let tx = TxEnvelope::decode_from(bytes)?;

        // Check if the type is allowed
        let tx_type = tx.tx_type();
        if !self.allowed_types.contains(&tx_type) {
            return Err(evolve_core::ErrorCode::new_with_arg(
                ERR_UNSUPPORTED_TX_TYPE.id,
                tx_type as u16,
            ));
        }

        Ok(tx)
    }
}

impl Default for TypedTxDecoder {
    fn default() -> Self {
        Self::ethereum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ethereum_decoder_types() {
        let decoder = TypedTxDecoder::ethereum();

        assert!(decoder.is_allowed(tx_type::LEGACY));
        assert!(decoder.is_allowed(tx_type::EIP1559));
        assert!(!decoder.is_allowed(tx_type::EIP2930)); // Not enabled by default
    }

    #[test]
    fn test_custom_types() {
        let decoder = TypedTxDecoder::with_types([tx_type::EIP1559, tx_type::EIP2930]);

        assert!(!decoder.is_allowed(tx_type::LEGACY));
        assert!(decoder.is_allowed(tx_type::EIP1559));
        assert!(decoder.is_allowed(tx_type::EIP2930));
    }

    #[test]
    fn test_allow_type_builder() {
        let mut decoder = TypedTxDecoder::ethereum();
        decoder.allow_type(tx_type::EIP4844);

        assert!(decoder.is_allowed(tx_type::EIP4844));
    }
}
