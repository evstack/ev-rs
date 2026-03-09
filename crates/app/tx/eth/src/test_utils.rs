//! Shared test utilities for Ethereum transaction signing.

use alloy_primitives::{PrimitiveSignature, B256, U256};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};

/// Sign a hash using a k256 signing key and return an alloy PrimitiveSignature.
pub fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
    let (sig, recovery_id): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
        .sign_prehash(hash.as_ref())
        .expect("signing should succeed");
    let r = U256::from_be_slice(sig.r().to_bytes().as_slice());
    let s = U256::from_be_slice(sig.s().to_bytes().as_slice());
    PrimitiveSignature::new(r, s, recovery_id.is_y_odd())
}
