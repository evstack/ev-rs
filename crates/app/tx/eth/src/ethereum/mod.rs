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
        if let (Some(head), Some(args)) = (input.get(..4), input.get(4..)) {
            if let Ok(head) = <[u8; 4]>::try_from(head) {
                (u32::from_be_bytes(head) as u64, args)
            } else {
                (0u64, input)
            }
        } else {
            (0u64, input)
        }
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
#[allow(clippy::indexing_slicing)]
mod tests {
    #[test]
    fn test_selector_extraction() {
        let expected_selector: u64 = 0xa9059cbb;
        let selector_bytes = [0xa9, 0x05, 0x9c, 0xbb];
        let computed = u32::from_be_bytes(selector_bytes) as u64;
        assert_eq!(computed, expected_selector);
    }
}
