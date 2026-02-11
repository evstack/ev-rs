//! Generic mempool implementation.
//!
//! Thread-safe in-memory transaction pool with configurable ordering.

// Mempool is not part of consensus - transactions are not committed to state.
#![allow(clippy::disallowed_types)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::marker::PhantomData;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::{MempoolError, MempoolResult};
use crate::traits::{MempoolOps, MempoolTx, SenderKey};

/// An entry in the priority queue, holding the ordering key and tx hash.
#[derive(Clone)]
struct OrderedEntry<K> {
    /// Transaction hash for lookup.
    hash: [u8; 32],
    /// Ordering key for priority.
    key: K,
}

impl<K: PartialEq> PartialEq for OrderedEntry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl<K: Eq> Eq for OrderedEntry<K> {}

impl<K: PartialOrd> PartialOrd for OrderedEntry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.key.partial_cmp(&other.key)
    }
}

impl<K: Ord> Ord for OrderedEntry<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// Generic in-memory transaction mempool.
///
/// Stores transactions of type `T` and orders them by `T::OrderingKey`.
/// Higher ordering keys have higher priority in the max-heap.
pub struct Mempool<T: MempoolTx> {
    /// Transactions indexed by hash.
    by_hash: HashMap<[u8; 32], Arc<T>>,
    /// Priority queue ordered by key (highest first).
    by_priority: BinaryHeap<OrderedEntry<T::OrderingKey>>,
    /// Transactions by sender key, ordered by nonce-like u64.
    /// Only populated if T::sender_key() returns Some.
    by_sender: HashMap<SenderKey, BTreeMap<u64, [u8; 32]>>,
    /// Per-sender sequence counters to avoid collisions in by_sender keys.
    by_sender_seq: HashMap<SenderKey, u64>,
    /// Phantom marker for T.
    _marker: PhantomData<T>,
}

impl<T: MempoolTx> Default for Mempool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: MempoolTx> Mempool<T> {
    /// Create a new empty mempool.
    pub fn new() -> Self {
        Self {
            by_hash: HashMap::new(),
            by_priority: BinaryHeap::new(),
            by_sender: HashMap::new(),
            by_sender_seq: HashMap::new(),
            _marker: PhantomData,
        }
    }

    /// Get the number of pending transactions.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Check if the mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Add a verified transaction to the mempool.
    ///
    /// The transaction should already be decoded and verified by a gateway.
    /// Returns the transaction ID on success.
    pub fn add(&mut self, tx: T) -> MempoolResult<[u8; 32]> {
        let tx_id = tx.tx_id();

        // Check for duplicates
        if self.by_hash.contains_key(&tx_id) {
            return Err(MempoolError::AlreadyExists);
        }

        let ordering_key = tx.ordering_key();
        let sender_key = tx.sender_key();

        let tx = Arc::new(tx);

        // Insert into hash index
        self.by_hash.insert(tx_id, tx.clone());

        // Insert into priority queue
        self.by_priority.push(OrderedEntry {
            hash: tx_id,
            key: ordering_key,
        });

        // Insert into sender index if applicable
        if let Some(sender) = sender_key {
            // For sender tracking, we need a secondary key (like nonce).
            // We use a u64 derived from the ordering - for gas price ordering
            // this would be the nonce, for FIFO it's the timestamp.
            // This is a simplification; in practice you'd want a dedicated nonce accessor.
            let secondary_key = self.next_sender_sequence(&sender);
            self.by_sender
                .entry(sender)
                .or_default()
                .insert(secondary_key, tx_id);
        }

        Ok(tx_id)
    }

    /// Derive a secondary key for sender tracking.
    ///
    /// This is used for the BTreeMap within by_sender to maintain ordering.
    fn next_sender_sequence(&mut self, sender: &SenderKey) -> u64 {
        let next = self.by_sender_seq.entry(*sender).or_insert(0);
        let current = *next;
        debug_assert!(current != u64::MAX, "sender sequence overflow");
        *next = next.checked_add(1).expect("sender sequence overflow");
        current
    }

    /// Get a transaction by hash.
    pub fn get(&self, hash: &[u8; 32]) -> Option<Arc<T>> {
        self.by_hash.get(hash).cloned()
    }

    /// Check if a transaction exists in the mempool.
    pub fn contains(&self, hash: &[u8; 32]) -> bool {
        self.by_hash.contains_key(hash)
    }

    /// Remove a transaction by hash.
    ///
    /// Returns the removed transaction if it existed.
    pub fn remove(&mut self, hash: &[u8; 32]) -> Option<Arc<T>> {
        let tx = self.by_hash.remove(hash)?;

        // Remove from sender index if applicable
        if let Some(sender) = tx.sender_key() {
            if let Some(nonces) = self.by_sender.get_mut(&sender) {
                // Find and remove the entry with this hash
                nonces.retain(|_, h| h != hash);
                if nonces.is_empty() {
                    self.by_sender.remove(&sender);
                    self.by_sender_seq.remove(&sender);
                }
            }
        }

        // Note: We don't remove from by_priority heap.
        // The heap entry becomes stale and will be skipped during selection.
        // This is a common optimization for heaps that don't support removal.

        Some(tx)
    }

    /// Remove multiple transactions by hash.
    pub fn remove_many(&mut self, hashes: &[[u8; 32]]) {
        for hash in hashes {
            self.remove(hash);
        }
    }

    /// Select up to `limit` transactions for block inclusion.
    ///
    /// Returns transactions ordered by priority (highest first),
    /// skipping any that have been removed.
    pub fn select(&mut self, limit: usize) -> Vec<Arc<T>> {
        let mut selected = Vec::with_capacity(limit);

        while selected.len() < limit {
            match self.by_priority.pop() {
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

    /// Select transactions for block inclusion up to gas and count limits.
    ///
    /// Selects transactions in priority order until either:
    /// - The cumulative gas would exceed `max_gas`
    /// - The count reaches `max_txs`
    /// - The mempool is exhausted
    ///
    /// Transactions that exceed the remaining gas budget are skipped,
    /// allowing smaller transactions to fill the remaining space.
    ///
    /// # Arguments
    /// * `max_gas` - Maximum cumulative gas (0 means no gas limit)
    /// * `max_txs` - Maximum number of transactions (0 means no count limit)
    ///
    /// # Returns
    /// Tuple of (selected transactions, total gas of selected transactions)
    pub fn select_with_gas_budget(&mut self, max_gas: u64, max_txs: usize) -> (Vec<Arc<T>>, u64) {
        let capacity = if max_txs > 0 { max_txs } else { self.len() };
        let mut selected = Vec::with_capacity(capacity);
        let mut total_gas: u64 = 0;

        // Collect entries we pop but don't select (too large for remaining budget)
        let mut skipped_entries = Vec::new();

        while max_txs == 0 || selected.len() < max_txs {
            match self.by_priority.pop() {
                Some(entry) => {
                    // Check if transaction still exists (may have been removed)
                    if let Some(tx) = self.by_hash.get(&entry.hash) {
                        let tx_gas = tx.gas_limit();

                        // Check if this tx fits in remaining gas budget
                        if max_gas == 0 || total_gas.saturating_add(tx_gas) <= max_gas {
                            total_gas = total_gas.saturating_add(tx_gas);
                            selected.push(tx.clone());
                        } else {
                            // Tx too large for remaining budget - skip it
                            // but keep track to re-add to heap later
                            skipped_entries.push(entry);
                        }
                    }
                    // If not found, the entry was stale - continue to next
                }
                None => break, // Heap is empty
            }
        }

        // Re-add skipped entries back to the priority queue
        for entry in skipped_entries {
            self.by_priority.push(entry);
        }

        (selected, total_gas)
    }

    /// Peek at the highest priority transaction without removing it.
    pub fn peek(&self) -> Option<Arc<T>> {
        self.by_priority
            .peek()
            .and_then(|entry| self.by_hash.get(&entry.hash).cloned())
    }

    /// Get all transaction hashes for a sender.
    pub fn get_sender_txs(&self, sender: &SenderKey) -> Vec<[u8; 32]> {
        self.by_sender
            .get(sender)
            .map(|nonces| nonces.values().copied().collect())
            .unwrap_or_default()
    }

    /// Clear all transactions from the mempool.
    pub fn clear(&mut self) {
        self.by_hash.clear();
        self.by_priority.clear();
        self.by_sender.clear();
        self.by_sender_seq.clear();
    }
}

/// Implementation of `MempoolOps` for the default in-memory mempool.
impl<T: MempoolTx> MempoolOps<T> for Mempool<T> {
    fn add(&mut self, tx: T) -> Result<[u8; 32], MempoolError> {
        Mempool::add(self, tx)
    }

    fn select(&mut self, limit: usize) -> Vec<Arc<T>> {
        Mempool::select(self, limit)
    }

    fn select_with_gas_budget(&mut self, max_gas: u64, max_txs: usize) -> (Vec<Arc<T>>, u64) {
        Mempool::select_with_gas_budget(self, max_gas, max_txs)
    }

    fn remove_many(&mut self, hashes: &[[u8; 32]]) {
        Mempool::remove_many(self, hashes)
    }

    fn get(&self, hash: &[u8; 32]) -> Option<Arc<T>> {
        Mempool::get(self, hash)
    }

    fn contains(&self, hash: &[u8; 32]) -> bool {
        Mempool::contains(self, hash)
    }

    fn len(&self) -> usize {
        Mempool::len(self)
    }

    fn is_empty(&self) -> bool {
        Mempool::is_empty(self)
    }

    fn remove(&mut self, hash: &[u8; 32]) -> Option<Arc<T>> {
        Mempool::remove(self, hash)
    }

    fn clear(&mut self) {
        Mempool::clear(self)
    }
}

/// Thread-safe shared mempool.
///
/// Wraps any mempool implementation in `Arc<RwLock<M>>` for concurrent access.
pub type SharedMempool<M> = Arc<RwLock<M>>;

/// Create a new shared mempool with the default in-memory implementation.
pub fn new_shared_mempool<T: MempoolTx>() -> SharedMempool<Mempool<T>> {
    Arc::new(RwLock::new(Mempool::new()))
}

/// Create a shared mempool from a custom implementation.
pub fn shared_mempool_from<M>(mempool: M) -> SharedMempool<M> {
    Arc::new(RwLock::new(mempool))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{FifoOrdering, GasPriceOrdering};

    /// Test transaction with gas price ordering.
    #[derive(Clone)]
    struct TestGasTx {
        id: [u8; 32],
        gas_price: u128,
        nonce: u64,
        sender: SenderKey,
        gas_limit: u64,
    }

    impl TestGasTx {
        fn new(id: u8, gas_price: u128, nonce: u64, sender: &[u8]) -> Self {
            Self {
                id: [id; 32],
                gas_price,
                nonce,
                sender: SenderKey::new(sender).expect("sender key within bounds"),
                gas_limit: 21000, // Default gas limit
            }
        }

        fn with_gas_limit(mut self, gas_limit: u64) -> Self {
            self.gas_limit = gas_limit;
            self
        }
    }

    impl MempoolTx for TestGasTx {
        type OrderingKey = GasPriceOrdering;

        fn tx_id(&self) -> [u8; 32] {
            self.id
        }

        fn ordering_key(&self) -> Self::OrderingKey {
            GasPriceOrdering::new(self.gas_price, self.nonce)
        }

        fn sender_key(&self) -> Option<SenderKey> {
            Some(self.sender)
        }

        fn gas_limit(&self) -> u64 {
            self.gas_limit
        }
    }

    /// Test transaction with FIFO ordering.
    #[derive(Clone)]
    struct TestFifoTx {
        id: [u8; 32],
        timestamp: u64,
        gas_limit: u64,
    }

    impl MempoolTx for TestFifoTx {
        type OrderingKey = FifoOrdering;

        fn tx_id(&self) -> [u8; 32] {
            self.id
        }

        fn ordering_key(&self) -> Self::OrderingKey {
            FifoOrdering::new(self.timestamp)
        }

        fn gas_limit(&self) -> u64 {
            self.gas_limit
        }
    }

    #[test]
    fn test_mempool_new() {
        let pool: Mempool<TestGasTx> = Mempool::new();
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_mempool_add_and_get() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let tx = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]);

        let id = pool.add(tx.clone()).unwrap();
        assert_eq!(id, [1u8; 32]);
        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());

        let retrieved = pool.get(&id).unwrap();
        assert_eq!(retrieved.id, tx.id);
    }

    #[test]
    fn test_mempool_duplicate_rejected() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let tx = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]);

        pool.add(tx.clone()).unwrap();
        let result = pool.add(tx);
        assert!(matches!(result, Err(MempoolError::AlreadyExists)));
    }

    #[test]
    fn test_mempool_remove() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let tx = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]);

        pool.add(tx).unwrap();
        assert_eq!(pool.len(), 1);

        let removed = pool.remove(&[1u8; 32]);
        assert!(removed.is_some());
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_gas_price_ordering_selection() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add transactions with different gas prices
        let low = TestGasTx::new(1, 50, 0, &[0xAAu8; 20]);
        let high = TestGasTx::new(2, 100, 0, &[0xBBu8; 20]);
        let medium = TestGasTx::new(3, 75, 0, &[0xCCu8; 20]);

        pool.add(low).unwrap();
        pool.add(high).unwrap();
        pool.add(medium).unwrap();

        let selected = pool.select(3);
        assert_eq!(selected.len(), 3);

        // Should be ordered by gas price (highest first)
        assert_eq!(selected[0].gas_price, 100);
        assert_eq!(selected[1].gas_price, 75);
        assert_eq!(selected[2].gas_price, 50);
    }

    #[test]
    fn test_fifo_ordering_selection() {
        let mut pool: Mempool<TestFifoTx> = Mempool::new();

        // Add transactions with different timestamps
        let newer = TestFifoTx {
            id: [1u8; 32],
            timestamp: 3000,
            gas_limit: 21000,
        };
        let oldest = TestFifoTx {
            id: [2u8; 32],
            timestamp: 1000,
            gas_limit: 21000,
        };
        let middle = TestFifoTx {
            id: [3u8; 32],
            timestamp: 2000,
            gas_limit: 21000,
        };

        pool.add(newer).unwrap();
        pool.add(oldest).unwrap();
        pool.add(middle).unwrap();

        let selected = pool.select(3);
        assert_eq!(selected.len(), 3);

        // Should be ordered by timestamp (oldest first = FIFO)
        assert_eq!(selected[0].timestamp, 1000);
        assert_eq!(selected[1].timestamp, 2000);
        assert_eq!(selected[2].timestamp, 3000);
    }

    #[test]
    fn test_select_with_limit() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        for i in 0..10 {
            let tx = TestGasTx::new(i, (i as u128) * 10, 0, &[i; 20]);
            pool.add(tx).unwrap();
        }

        let selected = pool.select(3);
        assert_eq!(selected.len(), 3);

        // Highest gas prices
        assert_eq!(selected[0].gas_price, 90);
        assert_eq!(selected[1].gas_price, 80);
        assert_eq!(selected[2].gas_price, 70);
    }

    #[test]
    fn test_stale_entries_skipped() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let tx1 = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]);
        let tx2 = TestGasTx::new(2, 50, 0, &[0xBBu8; 20]);

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();

        // Remove the higher priority one
        pool.remove(&[1u8; 32]);

        // Should still get the remaining tx
        let selected = pool.select(1);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, [2u8; 32]);
    }

    #[test]
    fn test_sender_tracking() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let sender = SenderKey::new(&[0xAAu8; 20]).expect("sender key within bounds");

        let tx1 = TestGasTx::new(1, 100, 0, sender.as_slice());
        let tx2 = TestGasTx::new(2, 100, 1, sender.as_slice());

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();

        let sender_txs = pool.get_sender_txs(&sender);
        assert_eq!(sender_txs.len(), 2);
    }

    #[test]
    fn test_sender_tracking_no_collision_after_removal() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        let sender = SenderKey::new(&[0xAAu8; 20]).expect("sender key within bounds");

        let tx1 = TestGasTx::new(1, 100, 0, sender.as_slice());
        let tx2 = TestGasTx::new(2, 100, 1, sender.as_slice());
        let tx3 = TestGasTx::new(3, 100, 2, sender.as_slice());

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();
        pool.remove(&[1u8; 32]);
        pool.add(tx3).unwrap();

        let sender_txs = pool.get_sender_txs(&sender);
        assert_eq!(sender_txs.len(), 2);
        assert!(sender_txs.contains(&[2u8; 32]));
        assert!(sender_txs.contains(&[3u8; 32]));
    }

    #[test]
    fn test_clear() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        for i in 0..5 {
            let tx = TestGasTx::new(i, 100, i as u64, &[0xAAu8; 20]);
            pool.add(tx).unwrap();
        }

        assert_eq!(pool.len(), 5);
        pool.clear();
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_select_with_gas_budget_respects_limit() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add 3 txs with 10k gas each
        for i in 0..3 {
            let tx = TestGasTx::new(i, 100, i as u64, &[0xAAu8; 20]).with_gas_limit(10_000);
            pool.add(tx).unwrap();
        }

        // Request max 25k gas - should get 2 txs (20k total)
        let (selected, total_gas) = pool.select_with_gas_budget(25_000, 0);
        assert_eq!(selected.len(), 2);
        assert_eq!(total_gas, 20_000);
    }

    #[test]
    fn test_select_with_gas_budget_respects_count() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add 5 txs with 10k gas each
        for i in 0..5 {
            let tx = TestGasTx::new(i, 100, i as u64, &[0xAAu8; 20]).with_gas_limit(10_000);
            pool.add(tx).unwrap();
        }

        // Request max 2 txs with unlimited gas
        let (selected, total_gas) = pool.select_with_gas_budget(0, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(total_gas, 20_000);
    }

    #[test]
    fn test_select_with_gas_budget_skips_large_txs() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add txs with different gas limits, highest gas price first
        // tx1: high gas price, large gas limit (won't fit after tx2)
        let tx1 = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]).with_gas_limit(50_000);
        // tx2: medium gas price, small gas limit
        let tx2 = TestGasTx::new(2, 80, 0, &[0xBBu8; 20]).with_gas_limit(10_000);
        // tx3: low gas price, small gas limit (will fit in remaining)
        let tx3 = TestGasTx::new(3, 60, 0, &[0xCCu8; 20]).with_gas_limit(10_000);

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();
        pool.add(tx3).unwrap();

        // Request max 60k gas - tx1 (50k) + tx2 (10k) = 60k, tx3 skipped
        let (selected, total_gas) = pool.select_with_gas_budget(60_000, 0);
        assert_eq!(selected.len(), 2);
        assert_eq!(total_gas, 60_000);
        assert_eq!(selected[0].gas_price, 100); // tx1
        assert_eq!(selected[1].gas_price, 80); // tx2
    }

    #[test]
    fn test_select_with_gas_budget_fills_remaining_space() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add: large tx (50k), small tx (10k), small tx (10k)
        let tx1 = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]).with_gas_limit(50_000);
        let tx2 = TestGasTx::new(2, 80, 0, &[0xBBu8; 20]).with_gas_limit(10_000);
        let tx3 = TestGasTx::new(3, 60, 0, &[0xCCu8; 20]).with_gas_limit(10_000);

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();
        pool.add(tx3).unwrap();

        // Request max 25k gas - tx1 doesn't fit, tx2 (10k) + tx3 (10k) = 20k
        let (selected, total_gas) = pool.select_with_gas_budget(25_000, 0);
        assert_eq!(selected.len(), 2);
        assert_eq!(total_gas, 20_000);
        // tx1 was skipped because it's too large
        assert_eq!(selected[0].gas_price, 80); // tx2
        assert_eq!(selected[1].gas_price, 60); // tx3
    }

    #[test]
    fn test_select_with_gas_budget_zero_means_no_limit() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        for i in 0..5 {
            let tx = TestGasTx::new(i, 100, i as u64, &[0xAAu8; 20]).with_gas_limit(10_000);
            pool.add(tx).unwrap();
        }

        // Both limits at 0 means no limits
        let (selected, total_gas) = pool.select_with_gas_budget(0, 0);
        assert_eq!(selected.len(), 5);
        assert_eq!(total_gas, 50_000);
    }

    #[test]
    fn test_select_with_gas_budget_skipped_txs_remain_in_pool() {
        let mut pool: Mempool<TestGasTx> = Mempool::new();

        // Add one large tx and one small tx
        let tx1 = TestGasTx::new(1, 100, 0, &[0xAAu8; 20]).with_gas_limit(50_000);
        let tx2 = TestGasTx::new(2, 80, 0, &[0xBBu8; 20]).with_gas_limit(10_000);

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();

        // First selection: max 15k gas - only tx2 fits
        let (selected, _) = pool.select_with_gas_budget(15_000, 0);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].gas_price, 80);

        // tx1 should still be in pool (it was skipped, not removed)
        assert!(pool.contains(&[1u8; 32]));
    }
}
