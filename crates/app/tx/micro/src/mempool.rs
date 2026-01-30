//! Mempool transaction wrapper for micro transactions.
//!
//! Wraps a `SignedMicroTx` and implements the `MempoolTx` trait
//! for use with the generic mempool.

use alloy_primitives::Address;
use evolve_mempool::{FifoOrdering, MempoolTx};

use crate::{MicroTx, SignedMicroTx};

/// A verified micro transaction ready for mempool storage.
///
/// Wraps a `SignedMicroTx` for use with the generic mempool.
/// Uses FIFO ordering (oldest timestamp = highest priority).
#[derive(Clone, Debug)]
pub struct MicroTxContext {
    /// The signed micro transaction.
    inner: SignedMicroTx,
}

impl MicroTxContext {
    /// Create a new context from a signed transaction.
    ///
    /// The transaction should already be verified by the gateway.
    pub fn new(inner: SignedMicroTx) -> Self {
        Self { inner }
    }

    /// Get the inner signed transaction.
    pub fn inner(&self) -> &SignedMicroTx {
        &self.inner
    }

    /// Get the sender address.
    pub fn sender(&self) -> Address {
        self.inner.sender()
    }

    /// Get the recipient address.
    pub fn recipient(&self) -> Address {
        self.inner.recipient()
    }

    /// Get the transfer amount.
    pub fn amount(&self) -> u128 {
        self.inner.amount()
    }

    /// Get the timestamp.
    pub fn timestamp(&self) -> u64 {
        self.inner.timestamp()
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.inner.chain_id()
    }

    /// Get the transaction data.
    pub fn data(&self) -> &[u8] {
        self.inner.data()
    }
}

impl MempoolTx for MicroTxContext {
    type OrderingKey = FifoOrdering;

    fn tx_id(&self) -> [u8; 32] {
        self.inner.tx_hash().0
    }

    fn ordering_key(&self) -> Self::OrderingKey {
        FifoOrdering::new(self.inner.timestamp())
    }

    fn sender_key(&self) -> Option<[u8; 20]> {
        // Micro transactions use pubkey-derived addresses
        Some(self.inner.sender().0 .0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_unsigned_payload, hash_for_signing};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn create_test_context(timestamp: u64) -> MicroTxContext {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let from_pubkey: [u8; 32] = verifying_key.to_bytes();

        let chain_id = 1u64;
        let to = Address::repeat_byte(0x22);
        let amount = 1000u128;

        let payload = create_unsigned_payload(chain_id, &from_pubkey, to, amount, timestamp, &[]);
        let signing_hash = hash_for_signing(&payload);
        let signature = signing_key.sign(signing_hash.as_slice());

        let mut tx_bytes = payload;
        tx_bytes.extend_from_slice(&signature.to_bytes());

        let tx = SignedMicroTx::decode(&tx_bytes).unwrap();
        MicroTxContext::new(tx)
    }

    #[test]
    fn test_context_accessors() {
        let ctx = create_test_context(1234567890);

        assert_eq!(ctx.timestamp(), 1234567890);
        assert_eq!(ctx.chain_id(), 1);
        assert_eq!(ctx.amount(), 1000);
        assert_eq!(ctx.recipient(), Address::repeat_byte(0x22));
    }

    #[test]
    fn test_mempool_tx_impl() {
        let ctx = create_test_context(1000);

        // tx_id should be 32 bytes
        let id = ctx.tx_id();
        assert_eq!(id.len(), 32);

        // ordering_key should wrap timestamp
        let key = ctx.ordering_key();
        assert_eq!(key.timestamp(), 1000);

        // sender_key should be 20 bytes
        let sender = ctx.sender_key().unwrap();
        assert_eq!(sender.len(), 20);
    }

    #[test]
    fn test_fifo_ordering() {
        let older = create_test_context(1000);
        let newer = create_test_context(2000);

        // Older should have higher priority (greater ordering key)
        assert!(older.ordering_key() > newer.ordering_key());
    }
}
