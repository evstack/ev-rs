//! Integration tests for transaction decoding and verification.
//!
//! These tests use real Ethereum transaction data to verify correct behavior.

use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
use alloy_primitives::{Address, Bytes, PrimitiveSignature, B256, U256};
use evolve_stf_traits::TxDecoder;
use evolve_tx_eth::{
    tx_type, EcdsaVerifier, SignatureVerifierRegistry, TxEnvelope, TypedTransaction, TypedTxDecoder,
};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use rand::rngs::OsRng;

/// Helper to sign a transaction hash and create an alloy signature
fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
    let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).unwrap();
    let r = U256::from_be_slice(&sig.r().to_bytes());
    let s = U256::from_be_slice(&sig.s().to_bytes());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

/// Get address from signing key (Ethereum address derivation)
fn get_address(signing_key: &SigningKey) -> Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let public_key_bytes = &public_key.as_bytes()[1..]; // Skip the 0x04 prefix
    let hash = alloy_primitives::keccak256(public_key_bytes);
    Address::from_slice(&hash[12..])
}

/// Test vectors from real Ethereum transactions
mod test_vectors {
    /// A real legacy transaction from Ethereum mainnet
    pub const LEGACY_TX_RLP: &str = concat!(
        "f86c098504a817c800825208943535353535353535353535353535353535353535880de0",
        "b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590",
        "620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
    );

    /// Expected sender for the legacy transaction above
    pub const LEGACY_TX_SENDER: &str = "0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F";

    /// Expected hash for the legacy transaction (keccak256 of RLP-encoded signed tx)
    pub const LEGACY_TX_HASH: &str =
        "0x33469b22e9f636356c4160a87eb19df52b7412e8eac32a4a55ffe88ea8350788";
}

// ============================================================================
// Legacy Transaction Tests
// ============================================================================

#[test]
fn test_decode_legacy_transaction() {
    let tx_bytes = hex::decode(test_vectors::LEGACY_TX_RLP).expect("valid hex");

    let tx = TxEnvelope::decode(&tx_bytes).expect("should decode");

    // Verify it's a legacy transaction
    assert_eq!(tx.tx_type(), tx_type::LEGACY);

    // Verify basic fields
    assert_eq!(tx.nonce(), 9);
    assert_eq!(tx.gas_limit(), 21000);

    // Verify sender recovery
    let expected_sender: Address = test_vectors::LEGACY_TX_SENDER.parse().unwrap();
    assert_eq!(tx.sender(), expected_sender);

    // Verify hash
    let expected_hash: B256 = test_vectors::LEGACY_TX_HASH.parse().unwrap();
    assert_eq!(tx.tx_hash(), expected_hash);
}

#[test]
fn test_decode_empty_input_fails() {
    let result = TxEnvelope::decode(&[]);
    assert!(result.is_err());
}

#[test]
fn test_decode_invalid_rlp_fails() {
    let invalid_bytes = vec![0x01, 0x02, 0x03]; // Not valid RLP
    let result = TxEnvelope::decode(&invalid_bytes);
    assert!(result.is_err());
}

#[test]
fn test_decode_rejects_trailing_bytes() {
    let mut tx_bytes = hex::decode(test_vectors::LEGACY_TX_RLP).expect("valid hex");
    tx_bytes.push(0x00);
    let result = TxEnvelope::decode(&tx_bytes);
    assert!(result.is_err());
}

#[test]
fn test_decode_rejects_typed_legacy_prefix() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 1,
        gas_limit: 21000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x00];
    signed.rlp_encode(&mut encoded);

    let result = TxEnvelope::decode(&encoded);
    assert!(result.is_err());
}

// ============================================================================
// Signature and Signing Tests
// ============================================================================

#[test]
fn test_sign_and_decode_legacy_transaction() {
    // Create a signer with a random private key
    let signing_key = SigningKey::random(&mut OsRng);
    let sender = get_address(&signing_key);

    // Create a legacy transaction
    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 20_000_000_000, // 20 gwei
        gas_limit: 21000,
        to: alloy_primitives::TxKind::Call(Address::repeat_byte(0x42)),
        value: U256::from(1_000_000_000_000_000_000u64), // 1 ETH
        input: Bytes::new(),
    };

    // Sign the transaction
    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    // Encode to RLP
    let mut encoded = Vec::new();
    signed.rlp_encode(&mut encoded);

    // Decode and verify
    let decoded = TxEnvelope::decode(&encoded).expect("should decode");

    assert_eq!(decoded.tx_type(), tx_type::LEGACY);
    assert_eq!(decoded.sender(), sender);
    assert_eq!(decoded.nonce(), 0);
    assert_eq!(decoded.gas_limit(), 21000);
    assert_eq!(decoded.chain_id(), Some(1));
}

#[test]
fn test_sign_and_decode_eip1559_transaction() {
    // Create a signer
    let signing_key = SigningKey::random(&mut OsRng);
    let sender = get_address(&signing_key);

    // Create an EIP-1559 transaction
    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 42,
        gas_limit: 100_000,
        max_fee_per_gas: 30_000_000_000,         // 30 gwei
        max_priority_fee_per_gas: 1_000_000_000, // 1 gwei
        to: alloy_primitives::TxKind::Call(Address::repeat_byte(0xAB)),
        value: U256::from(500_000_000_000_000_000u64), // 0.5 ETH
        input: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
        access_list: Default::default(),
    };

    // Sign the transaction
    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    // Encode with type prefix for EIP-2718
    let mut encoded = vec![0x02]; // EIP-1559 type prefix
    signed.rlp_encode(&mut encoded);

    // Decode and verify
    let decoded = TxEnvelope::decode(&encoded).expect("should decode");

    assert_eq!(decoded.tx_type(), tx_type::EIP1559);
    assert_eq!(decoded.sender(), sender);
    assert_eq!(decoded.nonce(), 42);
    assert_eq!(decoded.gas_limit(), 100_000);
    assert_eq!(decoded.chain_id(), Some(1));
    assert_eq!(decoded.input(), &[0xde, 0xad, 0xbe, 0xef]);
}

// ============================================================================
// Chain ID Verification Tests
// ============================================================================

#[test]
fn test_chain_id_verification_passes() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxEip1559 {
        chain_id: 1, // Mainnet
        nonce: 0,
        gas_limit: 21000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    // Verify with matching chain ID
    let verifier = EcdsaVerifier::new(1);
    assert!(verifier.verify_chain_id_for_tx(&decoded).is_ok());
}

#[test]
fn test_chain_id_verification_fails_mismatch() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxEip1559 {
        chain_id: 1, // Mainnet
        nonce: 0,
        gas_limit: 21000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    // Verify with different chain ID should fail
    let verifier = EcdsaVerifier::new(5); // Goerli, not mainnet
    assert!(verifier.verify_chain_id_for_tx(&decoded).is_err());
}

// ============================================================================
// Decoder Filter Tests
// ============================================================================

#[test]
fn test_decoder_rejects_unsupported_type() {
    let signing_key = SigningKey::random(&mut OsRng);

    // Create an EIP-1559 transaction
    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 21000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    // Create a decoder that only accepts legacy transactions
    let decoder = TypedTxDecoder::with_types([tx_type::LEGACY]);

    // Should reject EIP-1559
    let result = decoder.decode(&mut encoded.as_slice());
    assert!(result.is_err());
}

#[test]
fn test_decoder_accepts_configured_type() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 21000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    // Create a decoder that accepts EIP-1559
    let decoder = TypedTxDecoder::ethereum();

    // Should accept EIP-1559
    let result = decoder.decode(&mut encoded.as_slice());
    assert!(result.is_ok());
}

// ============================================================================
// Registry Tests
// ============================================================================

#[test]
fn test_registry_verifies_correct_chain() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxEip1559 {
        chain_id: 1,
        nonce: 0,
        gas_limit: 21000,
        max_fee_per_gas: 20_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
        access_list: Default::default(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    // Create registry for mainnet
    let registry = SignatureVerifierRegistry::ethereum(1);
    assert!(registry.verify(&decoded).is_ok());

    // Create registry for different chain
    let registry_wrong = SignatureVerifierRegistry::ethereum(5);
    assert!(registry_wrong.verify(&decoded).is_err());
}

// ============================================================================
// Address/AccountId Conversion Tests
// ============================================================================

#[test]
fn test_address_to_account_id_preserves_uniqueness() {
    use evolve_tx_eth::address_to_account_id;

    let addr1 = Address::repeat_byte(0x11);
    let addr2 = Address::repeat_byte(0x22);
    let addr3 = Address::repeat_byte(0x33);

    let id1 = address_to_account_id(addr1);
    let id2 = address_to_account_id(addr2);
    let id3 = address_to_account_id(addr3);

    // All should be unique
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

#[test]
fn test_sender_account_id_matches_address_conversion() {
    use evolve_tx_eth::address_to_account_id;

    let signing_key = SigningKey::random(&mut OsRng);
    let sender = get_address(&signing_key);
    let expected_id = address_to_account_id(sender);

    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 20_000_000_000,
        gas_limit: 21000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = Vec::new();
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    assert_eq!(decoded.sender_account_id(), expected_id);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_unsupported_transaction_type() {
    // Type 0x03 (EIP-4844) is not supported yet
    let bytes = vec![0x03, 0xc0]; // Type prefix + empty RLP list
    let result = TxEnvelope::decode(&bytes);
    assert!(result.is_err());
}

#[test]
fn test_contract_creation_transaction() {
    let signing_key = SigningKey::random(&mut OsRng);

    // Contract creation has no 'to' address
    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 20_000_000_000,
        gas_limit: 1_000_000,
        to: alloy_primitives::TxKind::Create, // Contract creation
        value: U256::ZERO,
        input: Bytes::from(vec![0x60, 0x80, 0x60, 0x40]), // Simple bytecode
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = Vec::new();
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    // to() should be None for contract creation
    assert!(decoded.to().is_none());
    assert!(decoded.recipient_account_id().is_none());
}

#[test]
fn test_max_value_transaction() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: u64::MAX,
        gas_price: u128::MAX,
        gas_limit: u64::MAX,
        to: alloy_primitives::TxKind::Call(Address::repeat_byte(0xFF)),
        value: U256::MAX,
        input: Bytes::new(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = Vec::new();
    signed.rlp_encode(&mut encoded);

    let decoded = TxEnvelope::decode(&encoded).unwrap();

    assert_eq!(decoded.nonce(), u64::MAX);
    assert_eq!(decoded.gas_limit(), u64::MAX);
    assert_eq!(decoded.value(), U256::MAX);
}

// ============================================================================
// Determinism Tests
// ============================================================================

#[test]
fn test_same_tx_same_hash() {
    let signing_key = SigningKey::random(&mut OsRng);

    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 123,
        gas_price: 20_000_000_000,
        gas_limit: 21000,
        to: alloy_primitives::TxKind::Call(Address::repeat_byte(0x42)),
        value: U256::from(1000u64),
        input: Bytes::new(),
    };

    let signature = sign_hash(&signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = Vec::new();
    signed.rlp_encode(&mut encoded);

    // Decode twice
    let decoded1 = TxEnvelope::decode(&encoded).unwrap();
    let decoded2 = TxEnvelope::decode(&encoded).unwrap();

    // Hash should be identical
    assert_eq!(decoded1.tx_hash(), decoded2.tx_hash());
    assert_eq!(decoded1.sender(), decoded2.sender());
}

#[test]
fn test_different_signers_different_senders() {
    let signing_key1 = SigningKey::random(&mut OsRng);
    let signing_key2 = SigningKey::random(&mut OsRng);

    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 20_000_000_000,
        gas_limit: 21000,
        to: alloy_primitives::TxKind::Call(Address::ZERO),
        value: U256::ZERO,
        input: Bytes::new(),
    };

    // Sign with different keys
    let sig1 = sign_hash(&signing_key1, tx.signature_hash());
    let sig2 = sign_hash(&signing_key2, tx.signature_hash());

    let signed1 = tx.clone().into_signed(sig1);
    let signed2 = tx.into_signed(sig2);

    let mut encoded1 = Vec::new();
    let mut encoded2 = Vec::new();
    signed1.rlp_encode(&mut encoded1);
    signed2.rlp_encode(&mut encoded2);

    let decoded1 = TxEnvelope::decode(&encoded1).unwrap();
    let decoded2 = TxEnvelope::decode(&encoded2).unwrap();

    // Senders should be different
    assert_ne!(decoded1.sender(), decoded2.sender());
}
