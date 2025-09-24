//! Structured fuzzing with arbitrary transaction generation.
//!
//! This generates valid-looking transactions and tests the full pipeline:
//! signing, encoding, decoding, and verification.

#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;

use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
use alloy_primitives::{Address, Bytes, PrimitiveSignature, U256};
use evolve_tx::{tx_type, EcdsaVerifier, TxEnvelope, TypedTransaction};

/// Fuzzable legacy transaction parameters
#[derive(Debug, Arbitrary)]
struct FuzzLegacyTx {
    nonce: u64,
    gas_price: u64,      // Constrained to u64 to avoid huge values
    gas_limit: u64,
    to: [u8; 20],
    value: [u8; 32],
    input: Vec<u8>,
    use_chain_id: bool,
    chain_id: u64,
}

/// Fuzzable EIP-1559 transaction parameters
#[derive(Debug, Arbitrary)]
struct FuzzEip1559Tx {
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    max_fee_per_gas: u64,
    max_priority_fee_per_gas: u64,
    to: [u8; 20],
    value: [u8; 32],
    input: Vec<u8>,
}

/// Fuzzable signature (note: these won't be cryptographically valid)
#[derive(Debug, Arbitrary)]
struct FuzzSignature {
    r: [u8; 32],
    s: [u8; 32],
    v: bool,
}

/// Combined fuzz input
#[derive(Debug, Arbitrary)]
enum FuzzInput {
    Legacy(FuzzLegacyTx, FuzzSignature),
    Eip1559(FuzzEip1559Tx, FuzzSignature),
    Raw(Vec<u8>),
}

impl FuzzSignature {
    fn to_primitive(&self) -> PrimitiveSignature {
        let r = U256::from_be_bytes(self.r);
        let s = U256::from_be_bytes(self.s);
        PrimitiveSignature::new(r, s, self.v)
    }
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::Legacy(tx_data, sig_data) => {
            let tx = TxLegacy {
                chain_id: if tx_data.use_chain_id { Some(tx_data.chain_id) } else { None },
                nonce: tx_data.nonce,
                gas_price: tx_data.gas_price as u128,
                gas_limit: tx_data.gas_limit,
                to: alloy_primitives::TxKind::Call(Address::from(tx_data.to)),
                value: U256::from_be_bytes(tx_data.value),
                input: Bytes::from(tx_data.input),
            };

            let sig = sig_data.to_primitive();
            let signed = tx.into_signed(sig);

            let mut encoded = Vec::new();
            signed.rlp_encode(&mut encoded);

            // Try to decode - signature recovery will likely fail with random sig
            // but the decoder itself should not panic
            match TxEnvelope::decode(&encoded) {
                Ok(decoded) => {
                    // If it decoded, accessors should work
                    let _ = decoded.tx_type();
                    let _ = decoded.nonce();
                    let _ = decoded.gas_limit();
                    let _ = decoded.chain_id();
                    let _ = decoded.to();
                    let _ = decoded.value();
                    let _ = decoded.input();

                    // Test verifier (will fail on chain_id mismatch, but shouldn't panic)
                    let verifier = EcdsaVerifier::new(1);
                    let _ = verifier.verify_chain_id_for_tx(&decoded);
                }
                Err(_) => {
                    // Expected - random signatures won't recover valid senders
                }
            }
        }

        FuzzInput::Eip1559(tx_data, sig_data) => {
            let tx = TxEip1559 {
                chain_id: tx_data.chain_id,
                nonce: tx_data.nonce,
                gas_limit: tx_data.gas_limit,
                max_fee_per_gas: tx_data.max_fee_per_gas as u128,
                max_priority_fee_per_gas: tx_data.max_priority_fee_per_gas as u128,
                to: alloy_primitives::TxKind::Call(Address::from(tx_data.to)),
                value: U256::from_be_bytes(tx_data.value),
                input: Bytes::from(tx_data.input),
                access_list: Default::default(),
            };

            let sig = sig_data.to_primitive();
            let signed = tx.into_signed(sig);

            let mut encoded = vec![tx_type::EIP1559];
            signed.rlp_encode(&mut encoded);

            match TxEnvelope::decode(&encoded) {
                Ok(decoded) => {
                    let _ = decoded.tx_type();
                    let _ = decoded.nonce();
                    let _ = decoded.gas_limit();
                    let _ = decoded.chain_id();
                    let _ = decoded.to();
                    let _ = decoded.value();
                    let _ = decoded.input();

                    let verifier = EcdsaVerifier::new(1);
                    let _ = verifier.verify_chain_id_for_tx(&decoded);
                }
                Err(_) => {
                    // Expected
                }
            }
        }

        FuzzInput::Raw(data) => {
            // Pure random bytes - same as fuzz_decode
            if let Ok(tx) = TxEnvelope::decode(&data) {
                let _ = tx.tx_type();
                let _ = tx.sender();
                let _ = tx.tx_hash();
                let _ = tx.nonce();
                let _ = tx.gas_limit();
                let _ = tx.chain_id();
                let _ = tx.to();
                let _ = tx.value();
                let _ = tx.input();
            }
        }
    }
});
