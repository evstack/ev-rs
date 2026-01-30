//! Evolve Micro TX - Minimal transaction format for high-throughput use cases.
//!
//! This crate provides a lean, optimized transaction format designed for
//! scenarios requiring maximum throughput and minimal overhead.
//!
//! ## Design Principles
//!
//! - **Minimal size**: Fixed-layout binary encoding, no RLP overhead
//! - **Fast decode**: Direct byte offset reads, no parsing
//! - **Timestamp-based**: No nonce coordination needed for parallel submission
//! - **Ed25519 signatures**: Hardware wallet friendly, faster verification
//!
//! ## Binary Format
//!
//! ```text
//! | Field      | Offset | Size | Description                    |
//! |------------|--------|------|--------------------------------|
//! | chain_id   | 0      | 8    | u64 big-endian                 |
//! | from       | 8      | 32   | sender Ed25519 public key      |
//! | to         | 40     | 20   | recipient address              |
//! | amount     | 60     | 16   | u128 big-endian                |
//! | timestamp  | 76     | 8    | u64 big-endian (ms)            |
//! | data_len   | 84     | 2    | u16 big-endian (max 65535)     |
//! | data       | 86     | var  | optional calldata              |
//! | signature  | 86+N   | 64   | Ed25519 signature              |
//! ```
//!
//! Minimum size (no data): **150 bytes**
//!
//! ## Differences from Ethereum Transactions
//!
//! - No nonce (uses timestamp for ordering)
//! - No gas fields (handled separately by execution layer)
//! - Amount is u128 not U256 (sufficient for most use cases, saves 16 bytes)
//! - Ed25519 instead of secp256k1 ECDSA (faster, hardware wallet support)
//! - No access lists
//! - Simpler fixed-layout encoding (not RLP)
//!
//! ## Usage
//!
//! ```ignore
//! use evolve_tx_micro::{MicroGateway, MicroTxContext};
//!
//! // Create a gateway for transaction validation
//! let gateway = MicroGateway::new(1337);
//!
//! // Decode and verify a raw transaction
//! let tx_context = gateway.decode_and_verify(raw_tx)?;
//!
//! // Add to mempool
//! mempool.add(tx_context)?;
//! ```

pub mod gateway;
pub mod mempool;

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use ed25519_dalek::{Signature, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};

pub use gateway::{MicroGateway, MicroGatewayError};
pub use mempool::MicroTxContext;

/// Micro transaction type identifier.
pub const MICRO_TX_TYPE: u8 = 0x83;

/// Minimum transaction size (fixed fields + signature, no data).
pub const MIN_TX_SIZE: usize = 150;

/// Maximum calldata size.
const MAX_DATA_SIZE: usize = 65535;

// Field offsets for zero-copy reads
const OFF_CHAIN_ID: usize = 0;
const OFF_FROM: usize = 8;
const OFF_TO: usize = 40;
const OFF_AMOUNT: usize = 60;
const OFF_TIMESTAMP: usize = 76;
const OFF_DATA_LEN: usize = 84;
const OFF_DATA: usize = 86;

/// Error type for micro transaction operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroTxError {
    /// Input too short.
    TooShort,
    /// Invalid data length.
    InvalidDataLen,
    /// Size mismatch.
    SizeMismatch,
    /// Invalid public key.
    InvalidPublicKey,
    /// Invalid signature.
    InvalidSignature,
    /// Signature verification failed.
    SignatureVerificationFailed,
}

/// Result type for micro transaction operations.
pub type MicroTxResult<T> = Result<T, MicroTxError>;

/// Core trait for micro transactions.
pub trait MicroTx {
    /// Transaction type identifier.
    fn tx_type(&self) -> u8;
    /// Sender's Ed25519 public key.
    fn sender_pubkey(&self) -> &[u8; 32];
    /// Sender address (derived from pubkey).
    fn sender(&self) -> Address;
    /// Recipient address.
    fn recipient(&self) -> Address;
    /// Transfer amount.
    fn amount(&self) -> u128;
    /// Timestamp in milliseconds.
    fn timestamp(&self) -> u64;
    /// Chain ID.
    fn chain_id(&self) -> u64;
    /// Transaction hash.
    fn tx_hash(&self) -> B256;
    /// Optional calldata.
    fn data(&self) -> &[u8];
}

/// A signed micro transaction with Ed25519 signature and timestamp-based replay protection.
#[derive(Clone, Debug)]
pub struct SignedMicroTx {
    chain_id: u64,
    from_pubkey: [u8; 32],
    from_address: Address,
    to: Address,
    amount: u128,
    timestamp: u64,
    data: Bytes,
    signature: [u8; 64],
    hash: B256,
}

impl SignedMicroTx {
    /// Decode from fixed-layout binary format.
    ///
    /// Optimized for speed - fixed fields are read directly from byte offsets.
    pub fn decode(bytes: &[u8]) -> MicroTxResult<Self> {
        if bytes.len() < MIN_TX_SIZE {
            return Err(MicroTxError::TooShort);
        }

        // Read fixed fields directly
        let chain_id =
            u64::from_be_bytes(bytes[OFF_CHAIN_ID..OFF_CHAIN_ID + 8].try_into().unwrap());

        let mut from_pubkey = [0u8; 32];
        from_pubkey.copy_from_slice(&bytes[OFF_FROM..OFF_FROM + 32]);

        let to = Address::from_slice(&bytes[OFF_TO..OFF_TO + 20]);

        let amount = u128::from_be_bytes(bytes[OFF_AMOUNT..OFF_AMOUNT + 16].try_into().unwrap());

        let timestamp =
            u64::from_be_bytes(bytes[OFF_TIMESTAMP..OFF_TIMESTAMP + 8].try_into().unwrap());

        // Read variable-length data
        let data_len =
            u16::from_be_bytes(bytes[OFF_DATA_LEN..OFF_DATA_LEN + 2].try_into().unwrap()) as usize;

        if data_len > MAX_DATA_SIZE {
            return Err(MicroTxError::InvalidDataLen);
        }

        let expected_size = MIN_TX_SIZE + data_len;
        if bytes.len() != expected_size {
            return Err(MicroTxError::SizeMismatch);
        }

        let data = if data_len > 0 {
            Bytes::copy_from_slice(&bytes[OFF_DATA..OFF_DATA + data_len])
        } else {
            Bytes::new()
        };

        // Read signature
        let sig_offset = OFF_DATA + data_len;
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&bytes[sig_offset..sig_offset + 64]);

        // Verify signature
        let payload_bytes = &bytes[..sig_offset];
        let signing_hash = compute_signing_hash(payload_bytes);

        verify_ed25519_signature(&from_pubkey, &signing_hash, &signature)?;

        // Derive address from pubkey (keccak256 hash, take last 20 bytes)
        let from_address = pubkey_to_address(&from_pubkey);

        // Compute transaction hash
        let mut hash_input = Vec::with_capacity(1 + bytes.len());
        hash_input.push(MICRO_TX_TYPE);
        hash_input.extend_from_slice(bytes);
        let hash = keccak256(&hash_input);

        Ok(Self {
            chain_id,
            from_pubkey,
            from_address,
            to,
            amount,
            timestamp,
            data,
            signature,
            hash,
        })
    }

    /// Encode to fixed-layout binary format.
    pub fn encode(&self) -> Vec<u8> {
        let data_len = self.data.len();
        let mut buf = Vec::with_capacity(MIN_TX_SIZE + data_len);

        buf.extend_from_slice(&self.chain_id.to_be_bytes());
        buf.extend_from_slice(&self.from_pubkey);
        buf.extend_from_slice(self.to.as_slice());
        buf.extend_from_slice(&self.amount.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());
        buf.extend_from_slice(&(data_len as u16).to_be_bytes());
        buf.extend_from_slice(&self.data);
        buf.extend_from_slice(&self.signature);

        buf
    }

    /// Get amount as U256 for compatibility.
    #[inline]
    pub fn amount_u256(&self) -> U256 {
        U256::from(self.amount)
    }

    /// Get the raw signature bytes.
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
}

impl MicroTx for SignedMicroTx {
    #[inline]
    fn tx_type(&self) -> u8 {
        MICRO_TX_TYPE
    }

    #[inline]
    fn sender_pubkey(&self) -> &[u8; 32] {
        &self.from_pubkey
    }

    #[inline]
    fn sender(&self) -> Address {
        self.from_address
    }

    #[inline]
    fn recipient(&self) -> Address {
        self.to
    }

    #[inline]
    fn amount(&self) -> u128 {
        self.amount
    }

    #[inline]
    fn timestamp(&self) -> u64 {
        self.timestamp
    }

    #[inline]
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    #[inline]
    fn tx_hash(&self) -> B256 {
        self.hash
    }

    #[inline]
    fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Compute signing hash from payload bytes.
#[inline]
fn compute_signing_hash(payload: &[u8]) -> B256 {
    let mut preimage = Vec::with_capacity(1 + payload.len());
    preimage.push(MICRO_TX_TYPE);
    preimage.extend_from_slice(payload);
    keccak256(&preimage)
}

/// Derive address from Ed25519 public key.
/// Uses keccak256 hash of the pubkey, takes last 20 bytes.
#[inline]
fn pubkey_to_address(pubkey: &[u8; 32]) -> Address {
    let hash = keccak256(pubkey);
    Address::from_slice(&hash[12..])
}

/// Verify Ed25519 signature.
fn verify_ed25519_signature(
    pubkey: &[u8; PUBLIC_KEY_LENGTH],
    message: &B256,
    sig_bytes: &[u8; SIGNATURE_LENGTH],
) -> MicroTxResult<()> {
    let verifying_key =
        VerifyingKey::from_bytes(pubkey).map_err(|_| MicroTxError::InvalidPublicKey)?;

    let signature = Signature::from_bytes(sig_bytes);

    verifying_key
        .verify_strict(message.as_slice(), &signature)
        .map_err(|_| MicroTxError::SignatureVerificationFailed)
}

/// Create an unsigned transaction payload for signing.
pub fn create_unsigned_payload(
    chain_id: u64,
    from_pubkey: &[u8; 32],
    to: Address,
    amount: u128,
    timestamp: u64,
    data: &[u8],
) -> Vec<u8> {
    let data_len = data.len();
    let mut buf = Vec::with_capacity(OFF_DATA + data_len);

    buf.extend_from_slice(&chain_id.to_be_bytes());
    buf.extend_from_slice(from_pubkey);
    buf.extend_from_slice(to.as_slice());
    buf.extend_from_slice(&amount.to_be_bytes());
    buf.extend_from_slice(&timestamp.to_be_bytes());
    buf.extend_from_slice(&(data_len as u16).to_be_bytes());
    buf.extend_from_slice(data);

    buf
}

/// Compute the hash to sign for an unsigned payload.
pub fn hash_for_signing(payload: &[u8]) -> B256 {
    compute_signing_hash(payload)
}

/// Derive an address from an Ed25519 public key.
pub fn address_from_pubkey(pubkey: &[u8; 32]) -> Address {
    pubkey_to_address(pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    #[test]
    fn test_min_tx_size() {
        // chain_id(8) + from(32) + to(20) + amount(16) + timestamp(8) + data_len(2) + signature(64)
        assert_eq!(MIN_TX_SIZE, 8 + 32 + 20 + 16 + 8 + 2 + 64);
        assert_eq!(MIN_TX_SIZE, 150);
    }

    #[test]
    fn test_decode_too_short() {
        let bytes = vec![0u8; MIN_TX_SIZE - 1];
        assert!(matches!(
            SignedMicroTx::decode(&bytes),
            Err(MicroTxError::TooShort)
        ));
    }

    #[test]
    fn test_decode_size_mismatch() {
        let mut bytes = vec![0u8; MIN_TX_SIZE];
        // Set data_len to 10 but don't provide extra bytes
        bytes[84] = 0;
        bytes[85] = 10;
        assert!(matches!(
            SignedMicroTx::decode(&bytes),
            Err(MicroTxError::SizeMismatch)
        ));
    }

    #[test]
    fn test_create_unsigned_payload() {
        let chain_id = 1u64;
        let from_pubkey = [0x11u8; 32];
        let to = Address::repeat_byte(0x22);
        let amount = 1000u128;
        let timestamp = 1234567890u64;

        let payload = create_unsigned_payload(chain_id, &from_pubkey, to, amount, timestamp, &[]);

        assert_eq!(payload.len(), OFF_DATA); // 86 bytes for no data
        assert_eq!(
            &payload[OFF_CHAIN_ID..OFF_CHAIN_ID + 8],
            &chain_id.to_be_bytes()
        );
        assert_eq!(&payload[OFF_FROM..OFF_FROM + 32], &from_pubkey);
        assert_eq!(&payload[OFF_TO..OFF_TO + 20], to.as_slice());
    }

    #[test]
    fn test_micro_tx_type() {
        assert_eq!(MICRO_TX_TYPE, 0x83);
    }

    #[test]
    fn test_roundtrip_with_signature() {
        // Generate a random keypair
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let from_pubkey: [u8; 32] = verifying_key.to_bytes();

        let chain_id = 1u64;
        let to = Address::repeat_byte(0x22);
        let amount = 1000u128;
        let timestamp = 1234567890u64;

        // Create unsigned payload
        let payload = create_unsigned_payload(chain_id, &from_pubkey, to, amount, timestamp, &[]);

        // Sign it
        let signing_hash = hash_for_signing(&payload);
        let signature: ed25519_dalek::Signature = signing_key.sign(signing_hash.as_slice());

        // Build full transaction bytes
        let mut tx_bytes = payload.clone();
        tx_bytes.extend_from_slice(&signature.to_bytes());

        // Decode and verify
        let tx = SignedMicroTx::decode(&tx_bytes).expect("decode should succeed");

        assert_eq!(tx.chain_id(), chain_id);
        assert_eq!(tx.sender_pubkey(), &from_pubkey);
        assert_eq!(tx.recipient(), to);
        assert_eq!(tx.amount(), amount);
        assert_eq!(tx.timestamp(), timestamp);
        assert_eq!(tx.data().len(), 0);

        // Verify address derivation
        let expected_address = pubkey_to_address(&from_pubkey);
        assert_eq!(tx.sender(), expected_address);

        // Roundtrip encode
        let encoded = tx.encode();
        assert_eq!(encoded, tx_bytes);
    }

    #[test]
    fn test_invalid_signature() {
        let from_pubkey = [0x11u8; 32]; // Invalid pubkey (not on curve)
        let chain_id = 1u64;
        let to = Address::repeat_byte(0x22);
        let amount = 1000u128;
        let timestamp = 1234567890u64;

        let payload = create_unsigned_payload(chain_id, &from_pubkey, to, amount, timestamp, &[]);

        // Append garbage signature
        let mut tx_bytes = payload;
        tx_bytes.extend_from_slice(&[0u8; 64]);

        // Should fail with invalid pubkey
        let result = SignedMicroTx::decode(&tx_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_pubkey_to_address() {
        let pubkey = [0x42u8; 32];
        let address = pubkey_to_address(&pubkey);

        // Address should be last 20 bytes of keccak256(pubkey)
        let hash = keccak256(pubkey);
        let expected = Address::from_slice(&hash[12..]);
        assert_eq!(address, expected);
    }
}
