//! Property-based tests for transaction encoding/decoding roundtrips.

use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
use alloy_primitives::{Address, Bytes, PrimitiveSignature, B256, U256};
use evolve_tx_eth::{tx_type, TxEnvelope, TypedTransaction};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use proptest::prelude::*;
use rand::rngs::OsRng;

/// Helper to sign a transaction hash and create an alloy signature
fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
    let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).unwrap();
    let r = U256::from_be_slice(&sig.r().to_bytes());
    let s = U256::from_be_slice(&sig.s().to_bytes());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

/// Get address from signing key
fn get_address(signing_key: &SigningKey) -> Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let public_key_bytes = &public_key.as_bytes()[1..];
    let hash = alloy_primitives::keccak256(public_key_bytes);
    Address::from_slice(&hash[12..])
}

// ============================================================================
// Strategies for generating random transaction data
// ============================================================================

fn arb_address() -> impl Strategy<Value = Address> {
    prop::array::uniform20(any::<u8>()).prop_map(Address::from)
}

fn arb_bytes(max_len: usize) -> impl Strategy<Value = Bytes> {
    prop::collection::vec(any::<u8>(), 0..max_len).prop_map(Bytes::from)
}

fn arb_u256() -> impl Strategy<Value = U256> {
    prop::array::uniform32(any::<u8>()).prop_map(|bytes| U256::from_be_bytes(bytes))
}

fn arb_legacy_tx() -> impl Strategy<Value = TxLegacy> {
    (
        any::<u64>(),                 // nonce
        1u128..1_000_000_000_000u128, // gas_price (reasonable range)
        21000u64..1_000_000u64,       // gas_limit
        arb_address(),                // to
        arb_u256(),                   // value
        arb_bytes(256),               // input (smaller for faster tests)
    )
        .prop_map(|(nonce, gas_price, gas_limit, to, value, input)| TxLegacy {
            chain_id: Some(1),
            nonce,
            gas_price,
            gas_limit,
            to: alloy_primitives::TxKind::Call(to),
            value,
            input,
        })
}

fn arb_eip1559_tx() -> impl Strategy<Value = TxEip1559> {
    (
        any::<u64>(),               // nonce
        21000u64..1_000_000u64,     // gas_limit
        1u128..100_000_000_000u128, // max_fee_per_gas
        1u128..10_000_000_000u128,  // max_priority_fee
        arb_address(),              // to
        arb_u256(),                 // value
        arb_bytes(256),             // input
    )
        .prop_map(
            |(nonce, gas_limit, max_fee, max_priority, to, value, input)| TxEip1559 {
                chain_id: 1,
                nonce,
                gas_limit,
                max_fee_per_gas: max_fee,
                max_priority_fee_per_gas: max_priority.min(max_fee), // priority <= max
                to: alloy_primitives::TxKind::Call(to),
                value,
                input,
                access_list: Default::default(),
            },
        )
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// Property: Encoding then decoding a legacy transaction preserves all fields
    #[test]
    fn prop_legacy_tx_roundtrip(tx in arb_legacy_tx()) {
        let signing_key = SigningKey::random(&mut OsRng);
        let expected_sender = get_address(&signing_key);

        // Sign the transaction
        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.clone().into_signed(signature);

        // Encode
        let mut encoded = Vec::new();
        signed.rlp_encode(&mut encoded);

        // Decode
        let decoded = TxEnvelope::decode(&encoded).unwrap();

        // Verify all fields
        prop_assert_eq!(decoded.tx_type(), tx_type::LEGACY);
        prop_assert_eq!(decoded.sender(), expected_sender);
        prop_assert_eq!(decoded.nonce(), tx.nonce);
        prop_assert_eq!(decoded.gas_limit(), tx.gas_limit);
        prop_assert_eq!(decoded.chain_id(), tx.chain_id);
        prop_assert_eq!(decoded.value(), tx.value);
        prop_assert_eq!(decoded.input(), tx.input.as_ref());

        if let alloy_primitives::TxKind::Call(to) = tx.to {
            prop_assert_eq!(decoded.to(), Some(to));
        }
    }

    /// Property: Encoding then decoding an EIP-1559 transaction preserves all fields
    #[test]
    fn prop_eip1559_tx_roundtrip(tx in arb_eip1559_tx()) {
        let signing_key = SigningKey::random(&mut OsRng);
        let expected_sender = get_address(&signing_key);

        // Sign the transaction
        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.clone().into_signed(signature);

        // Encode with type prefix
        let mut encoded = vec![0x02];
        signed.rlp_encode(&mut encoded);

        // Decode
        let decoded = TxEnvelope::decode(&encoded).unwrap();

        // Verify all fields
        prop_assert_eq!(decoded.tx_type(), tx_type::EIP1559);
        prop_assert_eq!(decoded.sender(), expected_sender);
        prop_assert_eq!(decoded.nonce(), tx.nonce);
        prop_assert_eq!(decoded.gas_limit(), tx.gas_limit);
        prop_assert_eq!(decoded.chain_id(), Some(tx.chain_id));
        prop_assert_eq!(decoded.value(), tx.value);
        prop_assert_eq!(decoded.input(), tx.input.as_ref());

        if let alloy_primitives::TxKind::Call(to) = tx.to {
            prop_assert_eq!(decoded.to(), Some(to));
        }
    }

    /// Property: Transaction hash is deterministic (same tx -> same hash)
    #[test]
    fn prop_tx_hash_deterministic(tx in arb_legacy_tx()) {
        let signing_key = SigningKey::random(&mut OsRng);

        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.into_signed(signature);

        let mut encoded = Vec::new();
        signed.rlp_encode(&mut encoded);

        // Decode twice
        let decoded1 = TxEnvelope::decode(&encoded).unwrap();
        let decoded2 = TxEnvelope::decode(&encoded).unwrap();

        // Hashes should be identical
        prop_assert_eq!(decoded1.tx_hash(), decoded2.tx_hash());
    }

    /// Property: Different private keys produce different senders
    #[test]
    fn prop_different_signers_different_senders(tx in arb_legacy_tx()) {
        let signing_key1 = SigningKey::random(&mut OsRng);
        let signing_key2 = SigningKey::random(&mut OsRng);

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

        // Senders should be different (with overwhelming probability)
        prop_assert_ne!(decoded1.sender(), decoded2.sender());
    }

    /// Property: Sender recovery is consistent with original signer
    #[test]
    fn prop_sender_matches_signer(tx in arb_eip1559_tx()) {
        let signing_key = SigningKey::random(&mut OsRng);
        let original_address = get_address(&signing_key);

        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.into_signed(signature);

        let mut encoded = vec![0x02];
        signed.rlp_encode(&mut encoded);

        let decoded = TxEnvelope::decode(&encoded).unwrap();

        prop_assert_eq!(decoded.sender(), original_address);
    }

}

// ============================================================================
// Model-Based Tests
// ============================================================================

/// A simple model of what a transaction should contain
#[derive(Debug, Clone)]
struct TxModel {
    sender: Address,
    nonce: u64,
    gas_limit: u64,
    chain_id: Option<u64>,
    value: U256,
    input: Vec<u8>,
    to: Option<Address>,
}

impl TxModel {
    fn from_envelope(tx: &TxEnvelope) -> Self {
        Self {
            sender: tx.sender(),
            nonce: tx.nonce(),
            gas_limit: tx.gas_limit(),
            chain_id: tx.chain_id(),
            value: tx.value(),
            input: tx.input().to_vec(),
            to: tx.to(),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(15))]

    /// Model test: TxEnvelope preserves the model
    #[test]
    fn prop_model_preservation(
        nonce in any::<u64>(),
        gas_limit in 21000u64..1_000_000u64,
        value_bytes in prop::array::uniform32(any::<u8>()),
        input in arb_bytes(128),
        to in arb_address(),
    ) {
        let signing_key = SigningKey::random(&mut OsRng);
        let value = U256::from_be_bytes(value_bytes);

        let tx = TxLegacy {
            chain_id: Some(1),
            nonce,
            gas_price: 20_000_000_000,
            gas_limit,
            to: alloy_primitives::TxKind::Call(to),
            value,
            input: input.clone(),
        };

        // Expected model
        let expected_model = TxModel {
            sender: get_address(&signing_key),
            nonce,
            gas_limit,
            chain_id: Some(1),
            value,
            input: input.to_vec(),
            to: Some(to),
        };

        // Sign and encode
        let signature = sign_hash(&signing_key, tx.signature_hash());
        let signed = tx.into_signed(signature);

        let mut encoded = Vec::new();
        signed.rlp_encode(&mut encoded);

        // Decode and extract model
        let decoded = TxEnvelope::decode(&encoded).unwrap();
        let actual_model = TxModel::from_envelope(&decoded);

        // Compare models
        prop_assert_eq!(actual_model.sender, expected_model.sender);
        prop_assert_eq!(actual_model.nonce, expected_model.nonce);
        prop_assert_eq!(actual_model.gas_limit, expected_model.gas_limit);
        prop_assert_eq!(actual_model.chain_id, expected_model.chain_id);
        prop_assert_eq!(actual_model.value, expected_model.value);
        prop_assert_eq!(actual_model.input, expected_model.input);
        prop_assert_eq!(actual_model.to, expected_model.to);
    }
}
