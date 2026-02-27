use alloy_primitives::{keccak256, Address, B256};
use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{All, Message, Secp256k1};
use std::sync::OnceLock;

use crate::error::ERR_SIGNATURE_RECOVERY;

pub fn recover_sender_from_signature_hash(
    signature_hash: B256,
    signature: &alloy_primitives::PrimitiveSignature,
) -> evolve_core::SdkResult<Address> {
    let mut compact = [0u8; 64];
    let r = signature.r().to_be_bytes::<32>();
    let s = signature.s().to_be_bytes::<32>();
    compact[..32].copy_from_slice(&r);
    compact[32..].copy_from_slice(&s);

    let recid = RecoveryId::from_i32(if signature.v() { 1 } else { 0 })
        .map_err(|_| ERR_SIGNATURE_RECOVERY)?;
    let recoverable =
        RecoverableSignature::from_compact(&compact, recid).map_err(|_| ERR_SIGNATURE_RECOVERY)?;

    let msg = Message::from_digest_slice(signature_hash.as_slice())
        .map_err(|_| ERR_SIGNATURE_RECOVERY)?;
    let pubkey = secp()
        .recover_ecdsa(&msg, &recoverable)
        .map_err(|_| ERR_SIGNATURE_RECOVERY)?;
    let uncompressed = pubkey.serialize_uncompressed();
    let pubkey_payload = uncompressed.get(1..).ok_or(ERR_SIGNATURE_RECOVERY)?;
    let hash = keccak256(pubkey_payload);
    let address_bytes = hash.as_slice().get(12..).ok_or(ERR_SIGNATURE_RECOVERY)?;
    Ok(Address::from_slice(address_bytes))
}

fn secp() -> &'static Secp256k1<All> {
    static SECP: OnceLock<Secp256k1<All>> = OnceLock::new();
    SECP.get_or_init(Secp256k1::new)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
    use rand::rngs::OsRng;

    fn sign_hash(signing_key: &SigningKey, hash: B256) -> alloy_primitives::PrimitiveSignature {
        let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).expect("sign");
        let r = U256::from_be_slice(&sig.r().to_bytes());
        let s = U256::from_be_slice(&sig.s().to_bytes());
        alloy_primitives::PrimitiveSignature::new(r, s, recovery_id.is_y_odd())
    }

    fn get_address(signing_key: &SigningKey) -> Address {
        let verifying_key = VerifyingKey::from(signing_key);
        let public_key = verifying_key.to_encoded_point(false);
        let public_key_bytes = &public_key.as_bytes()[1..];
        let hash = keccak256(public_key_bytes);
        Address::from_slice(&hash[12..])
    }

    #[test]
    fn test_recover_sender_from_valid_signature_hash() {
        let signing_key = SigningKey::random(&mut OsRng);
        let signature_hash = B256::from(keccak256(b"eth-sender-recovery-test"));
        let signature = sign_hash(&signing_key, signature_hash);

        let recovered =
            recover_sender_from_signature_hash(signature_hash, &signature).expect("recover");

        assert_eq!(recovered, get_address(&signing_key));
    }

    #[test]
    fn test_recover_sender_rejects_invalid_signature_components() {
        let signature_hash = B256::from(keccak256(b"invalid-signature-components"));
        let invalid = alloy_primitives::PrimitiveSignature::new(U256::MAX, U256::MAX, false);

        let err = recover_sender_from_signature_hash(signature_hash, &invalid).unwrap_err();
        assert_eq!(err, ERR_SIGNATURE_RECOVERY);
    }

    #[test]
    fn test_recover_sender_detects_forged_recovery_id() {
        let signing_key = SigningKey::random(&mut OsRng);
        let signature_hash = B256::from(keccak256(b"forged-recovery-id"));
        let valid = sign_hash(&signing_key, signature_hash);
        let forged = alloy_primitives::PrimitiveSignature::new(valid.r(), valid.s(), !valid.v());

        let recovered_valid =
            recover_sender_from_signature_hash(signature_hash, &valid).expect("recover valid");
        let recovered_forged =
            recover_sender_from_signature_hash(signature_hash, &forged).expect("recover forged");

        assert_ne!(recovered_valid, recovered_forged);
    }
}
