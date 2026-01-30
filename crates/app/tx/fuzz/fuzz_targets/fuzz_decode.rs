//! Fuzz target for raw transaction decoding.
//!
//! This is the primary attack surface - arbitrary bytes fed to the decoder.
//! We're looking for:
//! - Panics (unwraps, out-of-bounds, etc.)
//! - Infinite loops / excessive CPU
//! - Excessive memory allocation

#![no_main]

use libfuzzer_sys::fuzz_target;

use evolve_tx_eth::{TxEnvelope, TypedTransaction};

fuzz_target!(|data: &[u8]| {
    // Try to decode - errors are fine, panics are not
    if let Ok(tx) = TxEnvelope::decode(data) {
        // If decode succeeds, all accessors must not panic
        let _ = tx.tx_type();
        let _ = tx.sender();
        let _ = tx.tx_hash();
        let _ = tx.nonce();
        let _ = tx.gas_limit();
        let _ = tx.chain_id();
        let _ = tx.to();
        let _ = tx.value();
        let _ = tx.input();
        let _ = tx.sender_account_id();
        let _ = tx.recipient_account_id();
        let _ = tx.to_invoke_requests();
    }
});
