//! Ethereum standard transaction types.
//!
//! This module implements wrappers around alloy's transaction types,
//! adding sender recovery caching and implementing our TypedTransaction trait.

mod eip1559;
mod legacy;
mod recovery;

pub use eip1559::SignedEip1559Tx;
pub use legacy::SignedLegacyTx;

use evolve_core::{InvokeRequest, Message};

/// Convert Ethereum calldata into an InvokeRequest.
///
/// The first 4 bytes are interpreted as a selector (u32 big-endian -> u64).
/// Remaining bytes are treated as a Borsh-encoded payload.
pub(crate) fn invoke_request_from_input(input: &[u8]) -> InvokeRequest {
    let (function_id, args) = if input.len() >= 4 {
        let selector = u32::from_be_bytes([input[0], input[1], input[2], input[3]]) as u64;
        (selector, &input[4..])
    } else {
        (0u64, input)
    };

    InvokeRequest::new_from_message(
        "eth_dispatch",
        function_id,
        Message::from_bytes(args.to_vec()),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_selector_extraction() {
        let expected_selector: u64 = 0xa9059cbb;
        let selector_bytes = [0xa9, 0x05, 0x9c, 0xbb];
        let computed = u32::from_be_bytes(selector_bytes) as u64;
        assert_eq!(computed, expected_selector);
    }
}
