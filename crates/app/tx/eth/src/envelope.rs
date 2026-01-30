//! Transaction envelope supporting EIP-2718 typed transactions.

use alloy_primitives::{Address, B256, U256};
use evolve_core::{InvokeRequest, SdkResult};

use crate::error::{ERR_EMPTY_INPUT, ERR_TX_DECODE, ERR_UNSUPPORTED_TX_TYPE};
use crate::ethereum::{SignedEip1559Tx, SignedLegacyTx};
use crate::traits::TypedTransaction;

/// Transaction type constants per EIP-2718.
pub mod tx_type {
    /// Legacy transaction (pre-EIP-2718).
    pub const LEGACY: u8 = 0x00;
    /// EIP-2930 access list transaction.
    pub const EIP2930: u8 = 0x01;
    /// EIP-1559 fee market transaction.
    pub const EIP1559: u8 = 0x02;
    /// EIP-4844 blob transaction.
    pub const EIP4844: u8 = 0x03;
}

/// Unified transaction envelope that can hold any supported Ethereum transaction type.
///
/// This implements EIP-2718 typed transaction envelopes.
#[derive(Clone, Debug)]
pub enum TxEnvelope {
    /// Legacy transaction (type 0x00 or untyped).
    Legacy(SignedLegacyTx),
    /// EIP-1559 fee market transaction (type 0x02).
    Eip1559(SignedEip1559Tx),
}

impl TxEnvelope {
    /// Encode the transaction to RLP bytes.
    ///
    /// Returns the same format that `decode` accepts:
    /// - Legacy: RLP-encoded signed transaction
    /// - EIP-1559: Type prefix (0x02) followed by RLP-encoded signed transaction
    pub fn encode(&self) -> Vec<u8> {
        match self {
            TxEnvelope::Legacy(tx) => {
                let mut buf = Vec::new();
                tx.rlp_encode(&mut buf);
                buf
            }
            TxEnvelope::Eip1559(tx) => {
                let mut buf = vec![tx_type::EIP1559];
                tx.rlp_encode(&mut buf);
                buf
            }
        }
    }

    /// Decode a transaction from RLP-encoded bytes.
    ///
    /// Handles both legacy (untyped) and typed (EIP-2718) transactions.
    ///
    /// # Transaction Format
    ///
    /// - Legacy: RLP list starting with 0xc0-0xff
    /// - Typed: Type byte (0x00-0x7f) followed by RLP payload
    pub fn decode(bytes: &[u8]) -> SdkResult<Self> {
        let mut input = bytes;
        let tx = Self::decode_from(&mut input)?;
        if !input.is_empty() {
            return Err(ERR_TX_DECODE);
        }
        Ok(tx)
    }

    /// Decode a transaction from the front of a byte slice, advancing it.
    ///
    /// This is suitable for streaming decoders that process multiple
    /// transactions from a single buffer.
    pub fn decode_from(bytes: &mut &[u8]) -> SdkResult<Self> {
        if bytes.is_empty() {
            return Err(ERR_EMPTY_INPUT);
        }

        let input = *bytes;
        let first_byte = input[0];

        // Check if this is a legacy transaction (RLP list prefix)
        // RLP list prefixes start at 0xc0
        if first_byte >= 0xc0 {
            let mut cursor = input;
            let signed =
                alloy_consensus::Signed::<alloy_consensus::TxLegacy>::rlp_decode(&mut cursor)
                    .map_err(|_| ERR_TX_DECODE)?;

            let consumed = input.len().saturating_sub(cursor.len());
            let raw_bytes = &input[..consumed];
            let tx = SignedLegacyTx::from_alloy_with_bytes(signed, raw_bytes)?;
            *bytes = &input[consumed..];
            return Ok(TxEnvelope::Legacy(tx));
        }

        // Typed transaction: first byte is the type
        match first_byte {
            tx_type::LEGACY => Err(ERR_UNSUPPORTED_TX_TYPE.with_arg(tx_type::LEGACY as u16)),
            tx_type::EIP1559 => {
                let payload = &input[1..];
                let mut cursor = payload;
                let signed =
                    alloy_consensus::Signed::<alloy_consensus::TxEip1559>::rlp_decode(&mut cursor)
                        .map_err(|_| ERR_TX_DECODE)?;

                let consumed = payload.len().saturating_sub(cursor.len());
                let raw_rlp = &payload[..consumed];
                let tx = SignedEip1559Tx::from_alloy_with_bytes(signed, raw_rlp)?;
                *bytes = &input[1 + consumed..];
                Ok(TxEnvelope::Eip1559(tx))
            }
            // EIP-2930 and EIP-4844 can be added later
            ty => Err(ERR_UNSUPPORTED_TX_TYPE.with_arg(ty as u16)),
        }
    }

    /// Returns the transaction type byte.
    pub fn tx_type(&self) -> u8 {
        match self {
            TxEnvelope::Legacy(_) => tx_type::LEGACY,
            TxEnvelope::Eip1559(_) => tx_type::EIP1559,
        }
    }
}

impl TypedTransaction for TxEnvelope {
    fn tx_type(&self) -> u8 {
        TxEnvelope::tx_type(self)
    }

    fn sender(&self) -> Address {
        match self {
            TxEnvelope::Legacy(tx) => tx.sender(),
            TxEnvelope::Eip1559(tx) => tx.sender(),
        }
    }

    fn tx_hash(&self) -> B256 {
        match self {
            TxEnvelope::Legacy(tx) => tx.tx_hash(),
            TxEnvelope::Eip1559(tx) => tx.tx_hash(),
        }
    }

    fn gas_limit(&self) -> u64 {
        match self {
            TxEnvelope::Legacy(tx) => tx.gas_limit(),
            TxEnvelope::Eip1559(tx) => tx.gas_limit(),
        }
    }

    fn chain_id(&self) -> Option<u64> {
        match self {
            TxEnvelope::Legacy(tx) => tx.chain_id(),
            TxEnvelope::Eip1559(tx) => tx.chain_id(),
        }
    }

    fn nonce(&self) -> u64 {
        match self {
            TxEnvelope::Legacy(tx) => tx.nonce(),
            TxEnvelope::Eip1559(tx) => tx.nonce(),
        }
    }

    fn to(&self) -> Option<Address> {
        match self {
            TxEnvelope::Legacy(tx) => tx.to(),
            TxEnvelope::Eip1559(tx) => tx.to(),
        }
    }

    fn value(&self) -> U256 {
        match self {
            TxEnvelope::Legacy(tx) => tx.value(),
            TxEnvelope::Eip1559(tx) => tx.value(),
        }
    }

    fn input(&self) -> &[u8] {
        match self {
            TxEnvelope::Legacy(tx) => tx.input(),
            TxEnvelope::Eip1559(tx) => tx.input(),
        }
    }

    fn to_invoke_requests(&self) -> Vec<InvokeRequest> {
        match self {
            TxEnvelope::Legacy(tx) => tx.to_invoke_requests(),
            TxEnvelope::Eip1559(tx) => tx.to_invoke_requests(),
        }
    }
}

// Allow ErrorCode to have with_arg method
trait ErrorCodeExt {
    fn with_arg(self, arg: u16) -> evolve_core::ErrorCode;
}

impl ErrorCodeExt for evolve_core::ErrorCode {
    fn with_arg(self, arg: u16) -> evolve_core::ErrorCode {
        evolve_core::ErrorCode::new_with_arg(self.id, arg)
    }
}
