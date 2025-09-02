//! In-memory cache layer for storage operations.
//!
//! Provides two cache implementations:
//! - `DbCache`: Single-lock cache for simpler use cases
//! - `ShardedDbCache`: 256-shard cache for high-concurrency scenarios
//!
//! Both reduce lock contention on the main ADB RwLock and avoid async overhead
//! for repeated reads of the same keys.

use hashbrown::HashMap;
use parking_lot::RwLock;
use std::sync::Arc;

/// Maximum number of entries in the cache before eviction kicks in.
/// At ~4KB average value size, 16K entries = ~64MB memory footprint.
const DEFAULT_MAX_ENTRIES: usize = 16_384;

/// Number of shards for ShardedDbCache (256 = indexed by first byte of key)
const NUM_SHARDS: usize = 256;

/// Cached value - either present with data or confirmed absent (negative cache).
#[derive(Clone, Debug)]
pub enum CachedValue {
    /// Key exists with this value
    Present(Vec<u8>),
    /// Key confirmed to not exist (negative cache)
    Absent,
}

/// Thread-safe read cache for storage values.
///
/// Uses parking_lot::RwLock for fast synchronous access.
/// Supports both positive caching (key exists) and negative caching (key confirmed absent).
pub struct DbCache {
    entries: RwLock<HashMap<Vec<u8>, CachedValue>>,
    max_entries: usize,
}

impl DbCache {
    /// Creates a new cache with the specified maximum entry count.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(max_entries / 2)),
            max_entries,
        }
    }

    /// Creates a new cache with default settings (64MB equivalent).
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }

    /// Gets a cached value if present.
    /// Returns None if the key is not in cache (need to check storage).
    /// Returns Some(CachedValue::Present(data)) if the key exists.
    /// Returns Some(CachedValue::Absent) if the key is confirmed to not exist.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<CachedValue> {
        self.entries.read().get(key).cloned()
    }

    /// Inserts a value into the cache.
    /// If cache is full, clears half the entries (simple eviction strategy).
    pub fn insert(&self, key: Vec<u8>, value: CachedValue) {
        let mut entries = self.entries.write();

        // Simple eviction: if at capacity, clear half
        // This is O(n) but happens rarely and avoids LRU tracking overhead
        if entries.len() >= self.max_entries {
            let to_remove: Vec<_> = entries
                .keys()
                .take(self.max_entries / 2)
                .cloned()
                .collect();
            for k in to_remove {
                entries.remove(&k);
            }
        }

        entries.insert(key, value);
    }

    /// Inserts a present value (key exists with data).
    #[inline]
    pub fn insert_present(&self, key: Vec<u8>, value: Vec<u8>) {
        self.insert(key, CachedValue::Present(value));
    }

    /// Inserts an absent marker (key confirmed to not exist).
    #[inline]
    pub fn insert_absent(&self, key: Vec<u8>) {
        self.insert(key, CachedValue::Absent);
    }

    /// Invalidates a specific key from the cache.
    pub fn invalidate(&self, key: &[u8]) {
        self.entries.write().remove(key);
    }

    /// Invalidates all entries in the cache.
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// Returns the current number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }
}

impl Default for DbCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// A single shard in the sharded cache.
struct CacheShard {
    entries: RwLock<HashMap<Vec<u8>, CachedValue>>,
    max_entries: usize,
}

impl CacheShard {
    fn new(max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(max_entries / 2)),
            max_entries,
        }
    }

    #[inline]
    fn get(&self, key: &[u8]) -> Option<CachedValue> {
        self.entries.read().get(key).cloned()
    }

    fn insert(&self, key: Vec<u8>, value: CachedValue) {
        let mut entries = self.entries.write();

        // Simple eviction: if at capacity, clear half
        if entries.len() >= self.max_entries {
            let to_remove: Vec<_> = entries
                .keys()
                .take(self.max_entries / 2)
                .cloned()
                .collect();
            for k in to_remove {
                entries.remove(&k);
            }
        }

        entries.insert(key, value);
    }

    fn invalidate(&self, key: &[u8]) {
        self.entries.write().remove(key);
    }

    fn clear(&self) {
        self.entries.write().clear();
    }

    fn len(&self) -> usize {
        self.entries.read().len()
    }
}

/// High-performance sharded cache with 256 independent shards.
///
/// Keys are distributed across shards by their first byte, enabling:
/// - Parallel reads/writes to different key ranges without lock contention
/// - Near-linear scaling with concurrent access patterns
/// - Suitable for multi-threaded transaction validation and execution
///
/// # Sharding Strategy
///
/// Shard index = first byte of key (0-255). Keys with the same first byte
/// share a shard. This works well for:
/// - Account-prefixed keys (different accounts → different shards)
/// - Module-prefixed keys (different modules → different shards)
///
/// For empty keys, shard 0 is used.
pub struct ShardedDbCache {
    shards: Box<[CacheShard; NUM_SHARDS]>,
}

impl ShardedDbCache {
    /// Creates a new sharded cache with the specified max entries per shard.
    pub fn new(max_entries_per_shard: usize) -> Self {
        // Use array initialization to avoid heap allocation per shard
        let shards: [CacheShard; NUM_SHARDS] =
            std::array::from_fn(|_| CacheShard::new(max_entries_per_shard));
        Self {
            shards: Box::new(shards),
        }
    }

    /// Creates a new sharded cache with default settings.
    /// Total capacity: 256 shards × 64 entries = ~16K entries (~64MB at 4KB/value)
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES / NUM_SHARDS)
    }

    /// Returns the shard index for a given key.
    #[inline]
    fn shard_index(key: &[u8]) -> usize {
        key.first().copied().unwrap_or(0) as usize
    }

    /// Gets a cached value if present.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<CachedValue> {
        self.shards[Self::shard_index(key)].get(key)
    }

    /// Inserts a value into the appropriate shard.
    pub fn insert(&self, key: Vec<u8>, value: CachedValue) {
        self.shards[Self::shard_index(&key)].insert(key, value);
    }

    /// Inserts a present value (key exists with data).
    #[inline]
    pub fn insert_present(&self, key: Vec<u8>, value: Vec<u8>) {
        self.insert(key, CachedValue::Present(value));
    }

    /// Inserts an absent marker (key confirmed to not exist).
    #[inline]
    pub fn insert_absent(&self, key: Vec<u8>) {
        self.insert(key, CachedValue::Absent);
    }

    /// Invalidates a specific key from its shard.
    pub fn invalidate(&self, key: &[u8]) {
        self.shards[Self::shard_index(key)].invalidate(key);
    }

    /// Invalidates all entries in all shards.
    pub fn clear(&self) {
        for shard in self.shards.iter() {
            shard.clear();
        }
    }

    /// Returns the total number of cached entries across all shards.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.len()).sum()
    }

    /// Returns true if all shards are empty.
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|s| s.len() == 0)
    }

    /// Prefetches multiple keys in parallel by grouping them by shard.
    /// This is useful for cache warming before transaction execution.
    pub fn prefetch_keys<F>(&self, keys: &[Vec<u8>], fetch_fn: F)
    where
        F: Fn(&[u8]) -> Option<Vec<u8>> + Sync,
    {
        // Group keys by shard to minimize lock acquisitions
        let mut shard_keys: [Vec<&Vec<u8>>; NUM_SHARDS] = std::array::from_fn(|_| Vec::new());
        for key in keys {
            shard_keys[Self::shard_index(key)].push(key);
        }

        // Process each shard's keys
        for (shard_idx, keys) in shard_keys.iter().enumerate() {
            if keys.is_empty() {
                continue;
            }

            // Fetch and cache values for this shard
            for key in keys {
                // Skip if already cached
                if self.shards[shard_idx].get(key).is_some() {
                    continue;
                }

                // Fetch from storage and cache
                match fetch_fn(key) {
                    Some(value) => self.shards[shard_idx].insert((*key).clone(), CachedValue::Present(value)),
                    None => self.shards[shard_idx].insert((*key).clone(), CachedValue::Absent),
                }
            }
        }
    }
}

impl Default for ShardedDbCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// A wrapper that combines storage with an in-memory cache.
/// The cache is checked first on reads, reducing async overhead.
pub struct CachedStorage<S> {
    storage: S,
    cache: Arc<DbCache>,
}

impl<S: Clone> Clone for CachedStorage<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<S> CachedStorage<S> {
    /// Creates a new cached storage wrapper.
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            cache: Arc::new(DbCache::with_defaults()),
        }
    }

    /// Creates a new cached storage wrapper with a custom cache.
    pub fn with_cache(storage: S, cache: Arc<DbCache>) -> Self {
        Self { storage, cache }
    }

    /// Returns a reference to the underlying storage.
    pub fn inner(&self) -> &S {
        &self.storage
    }

    /// Returns a mutable reference to the underlying storage.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    /// Returns a reference to the cache.
    pub fn cache(&self) -> &Arc<DbCache> {
        &self.cache
    }

    /// Invalidates cache entries for the given keys.
    /// Call this after writes to ensure cache consistency.
    pub fn invalidate_keys(&self, keys: impl IntoIterator<Item = impl AsRef<[u8]>>) {
        for key in keys {
            self.cache.invalidate(key.as_ref());
        }
    }
}

// Implement ReadonlyKV for CachedStorage
impl<S: evolve_core::ReadonlyKV> evolve_core::ReadonlyKV for CachedStorage<S> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, evolve_core::ErrorCode> {
        // Check cache first (fast, synchronous)
        if let Some(cached) = self.cache.get(key) {
            return match cached {
                CachedValue::Present(data) => Ok(Some(data)),
                CachedValue::Absent => Ok(None),
            };
        }

        // Cache miss - fetch from underlying storage
        let result = self.storage.get(key)?;

        // Cache the result
        match &result {
            Some(data) => self.cache.insert_present(key.to_vec(), data.clone()),
            None => self.cache.insert_absent(key.to_vec()),
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = DbCache::with_defaults();

        // Test insert and get
        cache.insert_present(b"key1".to_vec(), b"value1".to_vec());

        match cache.get(b"key1") {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value1"),
            _ => panic!("Expected present value"),
        }

        // Test absent marker
        cache.insert_absent(b"key2".to_vec());
        match cache.get(b"key2") {
            Some(CachedValue::Absent) => {}
            _ => panic!("Expected absent marker"),
        }

        // Test cache miss
        assert!(cache.get(b"key3").is_none());
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = DbCache::with_defaults();

        cache.insert_present(b"key1".to_vec(), b"value1".to_vec());
        assert!(cache.get(b"key1").is_some());

        cache.invalidate(b"key1");
        assert!(cache.get(b"key1").is_none());
    }

    #[test]
    fn test_cache_eviction() {
        let cache = DbCache::new(10); // Small cache for testing

        // Fill the cache
        for i in 0..10 {
            cache.insert_present(format!("key{i}").into_bytes(), vec![i as u8]);
        }
        assert_eq!(cache.len(), 10);

        // Insert one more, should trigger eviction
        cache.insert_present(b"overflow".to_vec(), b"value".to_vec());

        // Cache should have been reduced
        assert!(cache.len() < 10);
    }

    #[test]
    fn test_cache_clear() {
        let cache = DbCache::with_defaults();

        cache.insert_present(b"key1".to_vec(), b"value1".to_vec());
        cache.insert_present(b"key2".to_vec(), b"value2".to_vec());

        cache.clear();
        assert!(cache.is_empty());
    }

    // ShardedDbCache tests

    #[test]
    fn test_sharded_cache_basic_operations() {
        let cache = ShardedDbCache::with_defaults();

        // Test insert and get
        cache.insert_present(b"key1".to_vec(), b"value1".to_vec());

        match cache.get(b"key1") {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value1"),
            _ => panic!("Expected present value"),
        }

        // Test absent marker
        cache.insert_absent(b"key2".to_vec());
        match cache.get(b"key2") {
            Some(CachedValue::Absent) => {}
            _ => panic!("Expected absent marker"),
        }

        // Test cache miss
        assert!(cache.get(b"key3").is_none());
    }

    #[test]
    fn test_sharded_cache_different_shards() {
        let cache = ShardedDbCache::with_defaults();

        // Keys with different first bytes go to different shards
        let key_a = b"\x00key_a".to_vec(); // Shard 0
        let key_b = b"\xFFkey_b".to_vec(); // Shard 255

        cache.insert_present(key_a.clone(), b"value_a".to_vec());
        cache.insert_present(key_b.clone(), b"value_b".to_vec());

        match cache.get(&key_a) {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value_a"),
            _ => panic!("Expected value_a"),
        }

        match cache.get(&key_b) {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"value_b"),
            _ => panic!("Expected value_b"),
        }
    }

    #[test]
    fn test_sharded_cache_invalidation() {
        let cache = ShardedDbCache::with_defaults();

        cache.insert_present(b"key1".to_vec(), b"value1".to_vec());
        assert!(cache.get(b"key1").is_some());

        cache.invalidate(b"key1");
        assert!(cache.get(b"key1").is_none());
    }

    #[test]
    fn test_sharded_cache_clear() {
        let cache = ShardedDbCache::with_defaults();

        // Insert keys into different shards
        for i in 0..10 {
            let mut key = vec![i as u8]; // First byte determines shard
            key.extend_from_slice(b"_suffix");
            cache.insert_present(key, vec![i]);
        }

        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_sharded_cache_len() {
        let cache = ShardedDbCache::with_defaults();

        // Insert keys into different shards
        for i in 0..10 {
            let mut key = vec![i as u8];
            key.extend_from_slice(b"_key");
            cache.insert_present(key, vec![i]);
        }

        assert_eq!(cache.len(), 10);
    }

    #[test]
    fn test_sharded_cache_prefetch() {
        let cache = ShardedDbCache::with_defaults();

        let keys = vec![
            b"\x00key1".to_vec(),
            b"\x01key2".to_vec(),
            b"\x00key3".to_vec(), // Same shard as key1
        ];

        // Mock fetch function
        cache.prefetch_keys(&keys, |key| {
            Some(format!("value_for_{}", String::from_utf8_lossy(key)).into_bytes())
        });

        // All keys should be cached
        for key in &keys {
            assert!(cache.get(key).is_some());
        }
    }

    #[test]
    fn test_sharded_cache_empty_key() {
        let cache = ShardedDbCache::with_defaults();

        // Empty key should go to shard 0
        cache.insert_present(vec![], b"empty_key_value".to_vec());

        match cache.get(&[]) {
            Some(CachedValue::Present(v)) => assert_eq!(v, b"empty_key_value"),
            _ => panic!("Expected value for empty key"),
        }
    }
}
