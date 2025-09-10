use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::{Environment, EnvironmentQuery, SdkResult};

use crate::map::Map;
use crate::vector::Vector;

/// An un-ordered but iterable map, implemented atop `Vector` + `Map`.
///
/// - `keys_vec` holds all keys in an arbitrary (un-ordered) sequence for iteration.
/// - `index_map` maps a key `K` to the index it occupies in `keys_vec`.
/// - `values_map` maps a key `K` to its corresponding value `V`.
///
/// Removal uses a "swap remove" strategy:
///   1. Take the last key in `keys_vec`.
///   2. Overwrite the position of the key being removed with that last key.
///   3. Update `index_map` for that last key.
///   4. Pop the last key out of `keys_vec`.
///
/// Consequently, the iteration order is not stable, but insertion/removal are O(1) on average.
pub struct UnorderedMap<K, V> {
    /// Maps key -> index in `keys_vec`.
    index_map: Map<K, u64>,

    /// Holds the actual keys in an arbitrary sequence.
    keys_vec: Vector<K>,

    /// Maps key -> value.
    values_map: Map<K, V>,
}

impl<K, V> UnorderedMap<K, V>
where
    K: Encodable + Decodable + Clone + PartialEq,
    V: Encodable + Decodable,
{
    /// Creates a new UnorderedMap using the provided storage prefixes.
    ///
    /// You likely want 4 distinct prefixes so that:
    ///  - `index_map` is at prefix A
    ///  - `keys_vec` is at (B, B+1) (Vector requires two prefixes)
    ///  - `values_map` is at prefix C
    pub const fn new(
        index_map_prefix: u8,
        keys_vec_index_prefix: u8,
        keys_vec_elems_prefix: u8,
        values_map_prefix: u8,
    ) -> Self {
        UnorderedMap {
            index_map: Map::new(index_map_prefix),
            keys_vec: Vector::new(keys_vec_index_prefix, keys_vec_elems_prefix),
            values_map: Map::new(values_map_prefix),
        }
    }

    /// Returns the number of key-value pairs in this map.
    pub fn len(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
        self.keys_vec.len(env)
    }

    /// Returns true if the map is empty.
    pub fn is_empty(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<bool> {
        Ok(self.len(env)? == 0)
    }

    /// Returns the value for `key`, or `None` if not present.
    pub fn may_get(&self, key: &K, env: &mut dyn EnvironmentQuery) -> SdkResult<Option<V>> {
        self.values_map.may_get(key, env)
    }

    /// Returns the value for `key`, or an error if not present.
    pub fn get(&self, key: &K, env: &mut dyn EnvironmentQuery) -> SdkResult<V> {
        self.values_map.get(key, env)
    }

    /// Inserts `value` for the given `key`.
    ///
    /// - If the key was **not** present, returns `Ok(None)`.
    /// - If the key **was** present, updates the value and returns `Ok(Some(old_value))`.
    pub fn insert(&self, key: &K, value: &V, env: &mut dyn Environment) -> SdkResult<Option<V>> {
        // Check if this key already exists
        if self.index_map.may_get(key, env)?.is_some() {
            // The key exists; update its value and return the old value
            let old_value = self.values_map.get(key, env)?;
            self.values_map.set(key, value, env)?;
            Ok(Some(old_value))
        } else {
            // The key does not exist yet
            let new_index = self.keys_vec.len(env)?;
            // Insert the key in the vector of keys
            self.keys_vec.push(key, env)?;
            // Record the index where that key was placed
            self.index_map.set(key, &new_index, env)?;
            // Insert the value
            self.values_map.set(key, value, env)?;
            Ok(None)
        }
    }

    /// Removes the entry for `key` and returns its old value (if any).
    ///
    /// Uses "swap remove" to keep the vector of keys dense (no gaps).
    ///
    /// - If `key` is not present, returns `Ok(None)`.
    /// - If `key` is present, returns `Ok(Some(old_value))`.
    pub fn remove(&self, key: &K, env: &mut dyn Environment) -> SdkResult<Option<V>> {
        // Does the key exist in our index map?
        let Some(removed_index) = self.index_map.may_get(key, env)? else {
            // Key not in the map at all
            return Ok(None);
        };

        // Remove the old index from the index_map
        self.index_map.remove(key, env)?;

        // Remove and return the old value from values_map
        let old_value = self.values_map.get(key, env)?;
        self.values_map.remove(key, env)?;

        // We need to do the "swap remove" in the keys_vec
        let last_index = self.keys_vec.len(env)? - 1;
        if removed_index < last_index {
            // Get the last key in the vector
            let last_key = self.keys_vec.get(last_index, env)?;
            // Move `last_key` into the freed position
            self.keys_vec.elems.set(&removed_index, &last_key, env)?;

            // Update that key's index to removed_index
            self.index_map.set(&last_key, &removed_index, env)?;
        }
        // Now pop the last key from the vector (we either used it to fill the hole or it was the removed one)
        self.keys_vec.pop(env)?;

        Ok(Some(old_value))
    }

    pub fn update(
        &self,
        key: &K,
        update_fn: impl FnOnce(Option<V>) -> SdkResult<V>,
        backend: &mut dyn Environment,
    ) -> SdkResult<V> {
        let old_value = self.may_get(key, backend)?;
        let new_value = update_fn(old_value)?;
        self.insert(key, &new_value, backend)?;
        Ok(new_value)
    }

    /// Returns an iterator over `(K, V)` for all entries in this map, in an **arbitrary** order.
    ///
    /// **Note**: Each `next()` call does a storage read under the hood for the key and value.
    /// If you need all data in-memory, you may want to collect them first.
    pub fn iter<'a>(
        &'a self,
        env: &'a mut dyn EnvironmentQuery,
    ) -> SdkResult<UnorderedMapIter<'a, K, V>> {
        let length = self.len(env)?;
        Ok(UnorderedMapIter {
            map: self,
            env,
            index: 0,
            length,
        })
    }
}

/// An iterator over `(K, V)` in the `UnorderedMap`.
///
/// Each `next()` performs storage lookups.
pub struct UnorderedMapIter<'a, K, V> {
    map: &'a UnorderedMap<K, V>,
    env: &'a mut dyn EnvironmentQuery,
    index: u64,
    length: u64,
}

impl<K, V> Iterator for UnorderedMapIter<'_, K, V>
where
    K: Encodable + Decodable + Clone,
    V: Encodable + Decodable,
{
    type Item = SdkResult<(K, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.length {
            return None;
        }
        // Grab the key from the vector
        let maybe_key = self.map.keys_vec.may_get(self.index, self.env);
        self.index += 1;

        match maybe_key {
            Ok(Some(k)) => {
                // Grab the value from the values_map
                match self.map.values_map.may_get(&k, self.env) {
                    Ok(Some(v)) => Some(Ok((k, v))),
                    Ok(None) => {
                        // This "shouldn't" happen if everything is consistent, but let's handle it
                        Some(Err(crate::ERR_VALUE_MISSING))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
            Ok(None) => {
                // Also shouldn't happen if consistent, but handle it
                Some(Err(crate::ERR_KEY_MISSING))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Brings UnorderedMap, etc. into scope
    use crate::mocks::MockEnvironment;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::{ErrorCode, SdkResult};

    /// Simple data type for testing our UnorderedMap
    #[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone)]
    struct TestData {
        pub x: u32,
    }

    #[test]
    fn test_new() -> SdkResult<()> {
        // Just ensure we can construct an UnorderedMap
        let _map: UnorderedMap<u64, TestData> = UnorderedMap::new(1, 2, 3, 4);
        Ok(())
    }

    #[test]
    fn test_insert_and_get() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(10, 11, 12, 13);
        let mut env = MockEnvironment::new(1, 2);

        // Insert a new key -> returns None
        let inserted = map.insert(&42, &TestData { x: 100 }, &mut env)?;
        assert_eq!(inserted, None);

        // Check `get` and `may_get`
        let val = map.get(&42, &mut env)?;
        assert_eq!(val, TestData { x: 100 });

        let maybe_val = map.may_get(&42, &mut env)?;
        assert_eq!(maybe_val, Some(TestData { x: 100 }));

        // Non-existent key -> get should fail, may_get -> None
        let err = map.get(&999, &mut env).err().unwrap();
        // Check that we get ERR_NOT_FOUND error
        assert_eq!(err, crate::ERR_NOT_FOUND);

        let maybe_missing = map.may_get(&999, &mut env)?;
        assert_eq!(maybe_missing, None);

        Ok(())
    }

    #[test]
    fn test_insert_existing() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(20, 21, 22, 23);
        let mut env = MockEnvironment::new(1, 2);

        // Insert key=1
        map.insert(&1, &TestData { x: 123 }, &mut env)?;

        // Insert key=1 again with a different value
        let old_value = map.insert(&1, &TestData { x: 999 }, &mut env)?;
        // We should get back the old value
        assert_eq!(old_value, Some(TestData { x: 123 }));

        // Now retrieving key=1 should yield x=999
        let val = map.get(&1, &mut env)?;
        assert_eq!(val, TestData { x: 999 });

        Ok(())
    }

    #[test]
    fn test_remove_nonexistent() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(30, 31, 32, 33);
        let mut env = MockEnvironment::new(1, 2);

        // Removing a key that doesn't exist -> None
        let removed = map.remove(&123, &mut env)?;
        assert_eq!(removed, None);

        Ok(())
    }

    #[test]
    fn test_remove_existing() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(40, 41, 42, 43);
        let mut env = MockEnvironment::new(1, 2);

        // Insert some keys
        map.insert(&1, &TestData { x: 111 }, &mut env)?;
        map.insert(&2, &TestData { x: 222 }, &mut env)?;
        map.insert(&3, &TestData { x: 333 }, &mut env)?;

        // Remove key=2 -> should return old value { x: 222 }
        let removed = map.remove(&2, &mut env)?;
        assert_eq!(removed, Some(TestData { x: 222 }));

        // Now 2 is gone
        assert!(map.may_get(&2, &mut env)?.is_none());

        // Keys 1 and 3 remain
        assert_eq!(map.get(&1, &mut env)?, TestData { x: 111 });
        assert_eq!(map.get(&3, &mut env)?, TestData { x: 333 });

        // We expect the underlying vector used "swap remove"
        // so the iteration order might have changed, but 1 and 3 remain valid.

        // Remove everything else
        assert_eq!(map.remove(&3, &mut env)?, Some(TestData { x: 333 }));
        assert_eq!(map.remove(&1, &mut env)?, Some(TestData { x: 111 }));

        // Nothing left
        assert_eq!(map.len(&mut env)?, 0);
        assert!(map.is_empty(&mut env)?);

        Ok(())
    }

    #[test]
    fn test_iter() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(50, 51, 52, 53);
        let mut env = MockEnvironment::new(1, 2);

        // Insert a few items
        map.insert(&10, &TestData { x: 101 }, &mut env)?;
        map.insert(&20, &TestData { x: 202 }, &mut env)?;
        map.insert(&30, &TestData { x: 303 }, &mut env)?;

        // Because it's "unordered", we don't guarantee the iteration order.
        // We'll just gather all items and check we have the correct set.
        let mut found = vec![];
        for entry_res in map.iter(&mut env)? {
            let (k, v) = entry_res?;
            found.push((k, v));
        }

        // Sort for test determinism
        found.sort_by_key(|(k, _)| *k);
        assert_eq!(
            found,
            vec![
                (10, TestData { x: 101 }),
                (20, TestData { x: 202 }),
                (30, TestData { x: 303 }),
            ]
        );

        Ok(())
    }

    #[test]
    fn test_len_and_is_empty() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(60, 61, 62, 63);
        let mut env = MockEnvironment::new(1, 2);

        assert_eq!(map.len(&mut env)?, 0);
        assert!(map.is_empty(&mut env)?);

        map.insert(&1, &TestData { x: 111 }, &mut env)?;
        map.insert(&2, &TestData { x: 222 }, &mut env)?;

        assert_eq!(map.len(&mut env)?, 2);
        assert!(!map.is_empty(&mut env)?);

        // Remove one key
        map.remove(&1, &mut env)?;
        assert_eq!(map.len(&mut env)?, 1);

        // Remove last key
        map.remove(&2, &mut env)?;
        assert_eq!(map.len(&mut env)?, 0);
        assert!(map.is_empty(&mut env)?);

        Ok(())
    }

    #[test]
    fn test_different_prefixes() -> SdkResult<()> {
        // Two different UnorderedMaps with different prefixes should not collide in storage
        let map1 = UnorderedMap::<u64, TestData>::new(70, 71, 72, 73);
        let map2 = UnorderedMap::<u64, TestData>::new(80, 81, 82, 83);
        let mut env = MockEnvironment::new(1, 2);

        map1.insert(&1, &TestData { x: 111 }, &mut env)?;
        map2.insert(&1, &TestData { x: 222 }, &mut env)?;

        // Check each has its own stored value
        assert_eq!(map1.get(&1, &mut env)?, TestData { x: 111 });
        assert_eq!(map2.get(&1, &mut env)?, TestData { x: 222 });

        Ok(())
    }

    #[test]
    fn test_different_account_ids() -> SdkResult<()> {
        // Same map prefix, but used on two different environments
        // with distinct whoami() => separate storages in the mock env.
        let map = UnorderedMap::<u64, TestData>::new(90, 91, 92, 93);

        let mut env1 = MockEnvironment::new(1, 2);
        let mut env2 = MockEnvironment::new(999, 1000);

        // Insert key=1 in env1
        map.insert(&1, &TestData { x: 111 }, &mut env1)?;
        // Insert key=1 in env2
        map.insert(&1, &TestData { x: 222 }, &mut env2)?;

        // Each environment sees its own data
        assert_eq!(map.get(&1, &mut env1)?, TestData { x: 111 });
        assert_eq!(map.get(&1, &mut env2)?, TestData { x: 222 });

        Ok(())
    }

    #[test]
    fn test_environment_error_propagation() {
        // If your environment can fail (like our MockEnv with `should_fail`),
        // you can verify that errors bubble up properly.

        let map = UnorderedMap::<u64, TestData>::new(100, 101, 102, 103);
        let mut env = MockEnvironment::with_failure(); // triggers error code=99 on any storage op

        let result = map.insert(&123, &TestData { x: 999 }, &mut env);
        let err = result.expect_err("should fail");
        assert_eq!(err.code(), 99);
    }

    #[test]
    fn test_update() -> SdkResult<()> {
        let map = UnorderedMap::<u64, TestData>::new(110, 111, 112, 113);
        let mut env = MockEnvironment::new(1, 2);

        // 1) Update on a non-existent key:
        //    The closure sees old_value = None, returns a brand-new value.
        let new_val = map.update(
            &42,
            |old_value| {
                assert!(old_value.is_none(), "Key should not exist yet");
                // Return a new value
                Ok(TestData { x: 999 })
            },
            &mut env,
        )?;
        assert_eq!(new_val, TestData { x: 999 });
        // Now the map must contain this new value
        let stored_val = map.get(&42, &mut env)?;
        assert_eq!(stored_val, TestData { x: 999 });

        // 2) Update on an existing key:
        //    The closure sees the old value, mutates it, and returns the new one.
        let updated_val = map.update(
            &42,
            |old_value| {
                let old = old_value.expect("Key should exist now");
                assert_eq!(old, TestData { x: 999 });
                // Example: increment x by 1
                Ok(TestData { x: old.x + 1 })
            },
            &mut env,
        )?;
        assert_eq!(updated_val, TestData { x: 1000 });
        // Verify stored value
        let stored_val = map.get(&42, &mut env)?;
        assert_eq!(stored_val, TestData { x: 1000 });

        // 3) (Optional) You can also test returning an error from your closure
        //    to ensure it bubbles up correctly:

        let err_result = map.update(&42, |_old_value| Err(ErrorCode::new(123)), &mut env);
        assert!(err_result.is_err());
        let err = err_result.err().unwrap();
        assert_eq!(err.code(), 123);
        // The map state should remain unchanged due to the error
        let stored_val = map.get(&42, &mut env)?;
        assert_eq!(stored_val, TestData { x: 1000 });

        Ok(())
    }
}
