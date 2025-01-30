use evolve_core::{ErrorCode, ReadonlyKV};
use std::collections::HashMap;

/// Represents a single key change so that it can be undone later.
#[derive(Debug)]
pub enum StateChange {
    Set {
        key: Vec<u8>,
        previous_value: Option<Vec<u8>>,
    },
    Remove {
        key: Vec<u8>,
        previous_value: Option<Vec<u8>>,
    },
}

impl StateChange {
    fn revert(self, map: &mut HashMap<Vec<u8>, Vec<u8>>) {
        match self {
            StateChange::Set { key, previous_value } => {
                // "Set" was performed. Reverting means:
                // If we had a previous value, restore it.
                // If we didn't, remove the key from map.
                match previous_value {
                    Some(old) => {
                        map.insert(key, old);
                    }
                    None => {
                        map.remove(&key);
                    }
                }
            }
            StateChange::Remove { key, previous_value } => {
                // "Remove" was performed. Reverting means:
                // If we had a previous value, re-insert it.
                // If we didn't, do nothing (it was absent before).
                if let Some(old) = previous_value {
                    map.insert(key, old);
                }
            }
        }
    }
}

/// A checkpoint provides an in-memory overlay that can be restored
/// to a prior state by "undoing" the logged changes.
pub struct Checkpoint<'a, S> {
    /// The in-memory overlay for changes not yet committed to the backing store.
    map: HashMap<Vec<u8>, Vec<u8>>,
    /// Ordered list of changes (for undo).
    change_list: Vec<StateChange>,

    /// The read-only backing store. We query it if `map` doesn’t have the key.
    storage: &'a S,
}

impl<'a, S> Checkpoint<'a, S> {
    pub fn new(storage: &'a S) -> Self {
        Self {
            storage,
            map: HashMap::new(),
            change_list: Vec::new(),
        }
    }
}

impl<'a, S: ReadonlyKV> Checkpoint<'a, S> {
    /// Retrieves a value first from local changes, else falls back to storage.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        if let Some(value) = self.map.get(key) {
            Ok(Some(value.clone()))
        } else {
            self.storage.get(key)
        }
    }

    /// Sets a key in the overlay and records the previous value so we can revert.
    pub fn set(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), ErrorCode> {
        // Capture the old value from either the overlay or the underlying storage
        let old_value = self
            .map
            .get(key)
            .cloned()
            .or_else(|| self.storage.get(key).ok().flatten());

        // Log the change
        self.change_list.push(StateChange::Set {
            key: key.to_vec(),
            previous_value: old_value,
        });

        // Actually update the overlay
        self.map.insert(key.to_vec(), value);

        Ok(())
    }

    /// Removes a key from the overlay and records the previous value so we can revert.
    pub fn remove(&mut self, key: &[u8]) -> Result<(), ErrorCode> {
        let old_value = self
            .map
            .get(key)
            .cloned()
            .or_else(|| self.storage.get(key).ok().flatten());

        self.change_list.push(StateChange::Remove {
            key: key.to_vec(),
            previous_value: old_value,
        });

        self.map.remove(key);
        Ok(())
    }

    /// Returns a "bookmark" in the change list we can restore to.
    pub fn checkpoint(&self) -> u64 {
        self.change_list.len() as u64
    }

    /// Restores state by popping and reverting changes until we're back to `checkpoint`.
    pub fn restore(&mut self, checkpoint: u64) {
        // Pop changes until we're back to the checkpoint index
        while self.change_list.len() as u64 > checkpoint {
            let change = self.change_list.pop().unwrap();
            change.revert(&mut self.map);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; // bring in the Checkpoint and StateChange
    use evolve_core::{ErrorCode, ReadonlyKV};
    use std::collections::HashMap;

    /// A simple in-memory mock that implements `ReadonlyKV`.
    /// It is strictly read-only for our demonstration, meaning you can
    /// only set initial data via `new_with_data`, and `get` is the only
    /// method that matters for `ReadonlyKV`.
    struct MockReadonlyKV {
        data: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl MockReadonlyKV {
        /// Constructs an empty storage.
        fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }

        /// Constructs storage with some initial key-value pairs.
        fn new_with_data(pairs: &[(&[u8], &[u8])]) -> Self {
            let mut data = HashMap::new();
            for (k, v) in pairs {
                data.insert(k.to_vec(), v.to_vec());
            }
            Self { data }
        }
    }

    impl ReadonlyKV for MockReadonlyKV {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
            Ok(self.data.get(key).cloned())
        }
    }

    /// Basic test: set and get a key without checkpoint/restore.
    #[test]
    fn test_basic_set_get() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        // Key doesn't exist yet
        assert_eq!(cp.get(b"hello").unwrap(), None);

        // Set the key
        cp.set(b"hello", b"world".to_vec()).unwrap();

        // Now we should see it in overlay
        assert_eq!(cp.get(b"hello").unwrap(), Some(b"world".to_vec()));
    }

    /// Test removing a key and verifying it no longer appears in the overlay.
    #[test]
    fn test_remove() {
        // Start with a backing store that already has "alpha" => "beta".
        let storage = MockReadonlyKV::new_with_data(&[(b"alpha", b"beta")]);
        let mut cp = Checkpoint::new(&storage);

        // Key "alpha" should come from the backing store
        assert_eq!(cp.get(b"alpha").unwrap(), Some(b"beta".to_vec()));

        // Remove "alpha"
        cp.remove(b"alpha").unwrap();

        // Now "alpha" is gone (overlaid removal).
        assert_eq!(cp.get(b"alpha").unwrap(), None);
    }

    /// Test that checkpointing and restoring reverts set/remove operations.
    #[test]
    fn test_checkpoint_restore() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        // Set key1 => "A"
        cp.set(b"key1", b"A".to_vec()).unwrap();
        // key1 is "A"
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"A".to_vec()));

        // Take a checkpoint here
        let c1 = cp.checkpoint();

        // Set key1 => "B" and key2 => "ZZ"
        cp.set(b"key1", b"B".to_vec()).unwrap();
        cp.set(b"key2", b"ZZ".to_vec()).unwrap();

        // Now:
        //  key1 => "B"
        //  key2 => "ZZ"
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"B".to_vec()));
        assert_eq!(cp.get(b"key2").unwrap(), Some(b"ZZ".to_vec()));

        // Restore back to c1
        cp.restore(c1);

        // After restoring:
        //   key1 should go back to "A"
        //   key2 was never set prior to c1, so it should disappear
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"A".to_vec()));
        assert_eq!(cp.get(b"key2").unwrap(), None);
    }

    /// Test multiple checkpoints and partial restores.
    #[test]
    fn test_multiple_checkpoints() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        // Start with nothing, set key1 => "one"
        cp.set(b"key1", b"one".to_vec()).unwrap();
        // checkpoint #1
        let c1 = cp.checkpoint();

        // Overwrite key1 => "uno"
        cp.set(b"key1", b"uno".to_vec()).unwrap();
        // checkpoint #2
        let c2 = cp.checkpoint();

        // Overwrite key1 => "eins"
        cp.set(b"key1", b"eins".to_vec()).unwrap();

        // Confirm current state
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"eins".to_vec()));

        // Restore to checkpoint #2 => "uno"
        cp.restore(c2);
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"uno".to_vec()));

        // Restore to checkpoint #1 => "one"
        cp.restore(c1);
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"one".to_vec()));
    }

    /// Test removing a key that was originally in the store, and then restoring.
    #[test]
    fn test_remove_and_restore() {
        // backing store has key => "store_val"
        let storage = MockReadonlyKV::new_with_data(&[(b"key", b"store_val")]);
        let mut cp = Checkpoint::new(&storage);

        // checkpoint #1
        let c1 = cp.checkpoint();

        // Remove key
        cp.remove(b"key").unwrap();
        assert_eq!(cp.get(b"key").unwrap(), None);

        // Restore to c1
        cp.restore(c1);
        // key should come back from backing store
        assert_eq!(cp.get(b"key").unwrap(), Some(b"store_val".to_vec()));
    }

    /// Test removing a key that doesn't exist in either store or overlay,
    /// then restoring (should be a no-op).
    #[test]
    fn test_remove_nonexistent_key() {
        let storage = MockReadonlyKV::new(); // empty store
        let mut cp = Checkpoint::new(&storage);

        let c1 = cp.checkpoint();
        cp.remove(b"nonexistent").unwrap();

        // Should still be None
        assert_eq!(cp.get(b"nonexistent").unwrap(), None);

        // restore
        cp.restore(c1);

        // Still None
        assert_eq!(cp.get(b"nonexistent").unwrap(), None);
    }

    /// Test setting a key multiple times and verify that each restore
    /// undoes the last set, returning the value to prior state.
    #[test]
    fn test_multiple_sets_of_same_key() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        cp.set(b"k", b"v1".to_vec()).unwrap();
        let c1 = cp.checkpoint();

        // Overwrite: k => "v2"
        cp.set(b"k", b"v2".to_vec()).unwrap();
        let c2 = cp.checkpoint();

        // Overwrite: k => "v3"
        cp.set(b"k", b"v3".to_vec()).unwrap();
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v3".to_vec()));

        // Restore to c2 => "v2"
        cp.restore(c2);
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v2".to_vec()));

        // Restore to c1 => "v1"
        cp.restore(c1);
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v1".to_vec()));
    }
}
