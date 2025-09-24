//! Fuzz target for encode/decode roundtrip consistency.
//!
//! If we can decode a transaction, re-encoding and decoding again
//! should produce identical results.

#![no_main]

use libfuzzer_sys::fuzz_target;

use alloy_consensus::transaction::RlpEcdsaTx;
use evolve_tx::{tx_type, TxEnvelope, TypedTransaction};

fuzz_target!(|data: &[u8]| {
    // Try to decode
    let Ok(tx1) = TxEnvelope::decode(data) else {
        return;
    };

    // Re-encode the transaction
    let encoded = match &tx1 {
        TxEnvelope::Legacy(legacy) => {
            let mut buf = Vec::new();
            legacy.tx().rlp_encode_signed(legacy.signature(), &mut buf);
            buf
        }
        TxEnvelope::Eip1559(eip1559) => {
            let mut buf = vec![tx_type::EIP1559];
            eip1559.tx().rlp_encode_signed(eip1559.signature(), &mut buf);
            buf
        }
    };

    // Decode again
    let Ok(tx2) = TxEnvelope::decode(&encoded) else {
        // If we successfully decoded once, re-encoding should be decodable
        panic!("Failed to decode re-encoded transaction");
    };

    // Verify consistency
    assert_eq!(tx1.tx_type(), tx2.tx_type(), "tx_type mismatch");
    assert_eq!(tx1.sender(), tx2.sender(), "sender mismatch");
    assert_eq!(tx1.nonce(), tx2.nonce(), "nonce mismatch");
    assert_eq!(tx1.gas_limit(), tx2.gas_limit(), "gas_limit mismatch");
    assert_eq!(tx1.chain_id(), tx2.chain_id(), "chain_id mismatch");
    assert_eq!(tx1.to(), tx2.to(), "to mismatch");
    assert_eq!(tx1.value(), tx2.value(), "value mismatch");
    assert_eq!(tx1.input(), tx2.input(), "input mismatch");
});
