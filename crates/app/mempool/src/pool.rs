//! Mempool implementation.
//!
//! Thread-safe in-memory transaction pool with gas price ordering.

// Mempool is not part of consensus - transactions are not committed to state.
#![allow(clippy::disallowed_types)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::sync::Arc;

use alloy_primitives::{Address, B256};
use tokio::sync::RwLock;

use crate::error::{MempoolError, MempoolResult};
use crate::tx::TxContext;
use evolve_stf_traits::TxDecoder;
use evolve_tx::{SignatureVerifierRegistry, TxEnvelope, TypedTransaction, TypedTxDecoder};

/// A transaction ordered by gas price (highest first).
#[derive(Clone)]
struct GasPricedTx {
    /// Transaction hash for lookup.
    hash: B256,
    /// Effective gas price for ordering.
    gas_price: u128,
    /// Nonce for secondary ordering (within same sender).
    nonce: u64,
}

impl PartialEq for GasPricedTx {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Eq for GasPricedTx {}

impl PartialOrd for GasPricedTx {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GasPricedTx {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher gas price first
        match self.gas_price.cmp(&other.gas_price) {
            Ordering::Equal => {
                // For same gas price, lower nonce first (reversed for max-heap)
                other.nonce.cmp(&self.nonce)
            }
            other_ord => other_ord,
        }
    }
}

/// In-memory transaction mempool.
///
/// Provides thread-safe storage and retrieval of pending transactions,
/// ordered by effective gas price for block production.
pub struct Mempool {
    /// Transactions indexed by hash.
    by_hash: HashMap<B256, Arc<TxContext>>,
    /// Priority queue ordered by gas price (highest first).
    by_gas_price: BinaryHeap<GasPricedTx>,
    /// Transactions by sender, ordered by nonce.
    by_sender: HashMap<Address, BTreeMap<u64, B256>>,
    /// Chain ID for validation.
    chain_id: u64,
    /// Transaction decoder (type filtering).
    decoder: TypedTxDecoder,
    /// Signature/chain ID verifier registry.
    verifier: SignatureVerifierRegistry,
    /// Current base fee for effective gas price calculation.
    base_fee: u128,
}

impl Mempool {
    /// Create a new mempool for the given chain ID.
    pub fn new(chain_id: u64) -> Self {
        Self {
            by_hash: HashMap::new(),
            by_gas_price: BinaryHeap::new(),
            by_sender: HashMap::new(),
            chain_id,
            decoder: TypedTxDecoder::ethereum(),
            verifier: SignatureVerifierRegistry::ethereum(chain_id),
            base_fee: 0,
        }
    }

    /// Create a new mempool with a specific base fee.
    pub fn with_base_fee(chain_id: u64, base_fee: u128) -> Self {
        Self {
            by_hash: HashMap::new(),
            by_gas_price: BinaryHeap::new(),
            by_sender: HashMap::new(),
            chain_id,
            decoder: TypedTxDecoder::ethereum(),
            verifier: SignatureVerifierRegistry::ethereum(chain_id),
            base_fee,
        }
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Get the current base fee.
    pub fn base_fee(&self) -> u128 {
        self.base_fee
    }

    /// Update the base fee (e.g., after a new block).
    pub fn set_base_fee(&mut self, base_fee: u128) {
        self.base_fee = base_fee;
        // Note: This doesn't re-sort existing transactions.
        // For MVP, this is acceptable. Production would need to rebuild the heap.
    }

    /// Get the number of pending transactions.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Check if the mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Add a raw transaction to the mempool.
    ///
    /// Decodes, validates signature/chain ID, and inserts into the pool.
    /// Returns the transaction hash on success.
    pub fn add_raw(&mut self, raw: &[u8]) -> MempoolResult<B256> {
        // Decode the transaction with type filtering
        let mut input = raw;
        let envelope = self
            .decoder
            .decode(&mut input)
            .map_err(|e| MempoolError::DecodeFailed(format!("{:?}", e)))?;
        if !input.is_empty() {
            return Err(MempoolError::DecodeFailed(
                "trailing bytes after transaction decode".to_string(),
            ));
        }

        self.add_envelope(envelope)
    }

    /// Add a decoded transaction envelope to the mempool.
    pub fn add_envelope(&mut self, envelope: TxEnvelope) -> MempoolResult<B256> {
        // Verify chain ID
        self.verifier
            .verify(&envelope)
            .map_err(|_| match envelope.chain_id() {
                Some(id) => MempoolError::InvalidChainId {
                    expected: self.chain_id,
                    actual: id,
                },
                None => MempoolError::MissingChainId,
            })?;

        let hash = envelope.tx_hash();

        // Check for duplicates
        if self.by_hash.contains_key(&hash) {
            return Err(MempoolError::AlreadyExists);
        }

        // Create mempool transaction
        let tx = TxContext::new(envelope, self.base_fee)
            .ok_or(MempoolError::ContractCreationNotSupported)?;

        let sender = tx.sender_address();
        let nonce = tx.nonce();
        let gas_price = tx.effective_gas_price();

        let tx = Arc::new(tx);

        // Insert into indices
        self.by_hash.insert(hash, tx);
        self.by_gas_price.push(GasPricedTx {
            hash,
            gas_price,
            nonce,
        });
        self.by_sender
            .entry(sender)
            .or_default()
            .insert(nonce, hash);

        Ok(hash)
    }

    /// Get a transaction by hash.
    pub fn get(&self, hash: &B256) -> Option<Arc<TxContext>> {
        self.by_hash.get(hash).cloned()
    }

    /// Check if a transaction exists in the mempool.
    pub fn contains(&self, hash: &B256) -> bool {
        self.by_hash.contains_key(hash)
    }

    /// Remove a transaction by hash.
    ///
    /// Returns the removed transaction if it existed.
    pub fn remove(&mut self, hash: &B256) -> Option<Arc<TxContext>> {
        let tx = self.by_hash.remove(hash)?;

        // Remove from sender index
        let sender = tx.sender_address();
        if let Some(nonces) = self.by_sender.get_mut(&sender) {
            nonces.remove(&tx.nonce());
            if nonces.is_empty() {
                self.by_sender.remove(&sender);
            }
        }

        // Note: We don't remove from by_gas_price heap.
        // The heap entry becomes stale and will be skipped during selection.
        // This is a common optimization for heaps that don't support removal.

        Some(tx)
    }

    /// Remove multiple transactions by hash.
    pub fn remove_many(&mut self, hashes: &[B256]) {
        for hash in hashes {
            self.remove(hash);
        }
    }

    /// Select up to `limit` transactions for block inclusion.
    ///
    /// Returns transactions ordered by effective gas price (highest first),
    /// skipping any that have been removed.
    pub fn select(&mut self, limit: usize) -> Vec<Arc<TxContext>> {
        let mut selected = Vec::with_capacity(limit);

        while selected.len() < limit {
            match self.by_gas_price.pop() {
                Some(entry) => {
                    // Check if transaction still exists (may have been removed)
                    if let Some(tx) = self.by_hash.get(&entry.hash) {
                        selected.push(tx.clone());
                    }
                    // If not found, the entry was stale - continue to next
                }
                None => break, // Heap is empty
            }
        }

        selected
    }

    /// Peek at the highest priority transaction without removing it.
    pub fn peek(&self) -> Option<Arc<TxContext>> {
        // We can't easily peek with stale entry handling, so just get from hash map
        // This is O(n) but peek is rarely used
        self.by_gas_price
            .peek()
            .and_then(|entry| self.by_hash.get(&entry.hash).cloned())
    }

    /// Get all transaction hashes for a sender.
    pub fn get_sender_txs(&self, sender: &Address) -> Vec<B256> {
        self.by_sender
            .get(sender)
            .map(|nonces| nonces.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Clear all transactions from the mempool.
    pub fn clear(&mut self) {
        self.by_hash.clear();
        self.by_gas_price.clear();
        self.by_sender.clear();
    }
}

/// Thread-safe shared mempool.
pub type SharedMempool = Arc<RwLock<Mempool>>;

/// Create a new shared mempool.
pub fn new_shared_mempool(chain_id: u64) -> SharedMempool {
    Arc::new(RwLock::new(Mempool::new(chain_id)))
}

/// Create a new shared mempool with a specific base fee.
pub fn new_shared_mempool_with_base_fee(chain_id: u64, base_fee: u128) -> SharedMempool {
    Arc::new(RwLock::new(Mempool::with_base_fee(chain_id, base_fee)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_priced_tx_ordering() {
        let high = GasPricedTx {
            hash: B256::ZERO,
            gas_price: 100,
            nonce: 0,
        };
        let low = GasPricedTx {
            hash: B256::repeat_byte(1),
            gas_price: 50,
            nonce: 0,
        };

        // Higher gas price should be greater (comes first in max-heap)
        assert!(high > low);
    }

    #[test]
    fn test_same_gas_price_lower_nonce_first() {
        let nonce_0 = GasPricedTx {
            hash: B256::ZERO,
            gas_price: 100,
            nonce: 0,
        };
        let nonce_1 = GasPricedTx {
            hash: B256::repeat_byte(1),
            gas_price: 100,
            nonce: 1,
        };

        // Lower nonce should be greater (comes first in max-heap)
        assert!(nonce_0 > nonce_1);
    }

    #[test]
    fn test_mempool_new() {
        let pool = Mempool::new(1);
        assert_eq!(pool.chain_id(), 1);
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }
}
