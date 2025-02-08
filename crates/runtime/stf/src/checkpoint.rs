use evolve_core::{ErrorCode, ReadonlyKV};
use evolve_server_core::{StateChange as CoreStateChange, WritableKV};
use std::collections::HashMap;

/// Represents one change to the overlay so it can be undone.
#[derive(Debug)]
enum StateChange {
    /// We set a key to a new value. `previous_value` is what it was logically
    /// (including falling back to storage if needed).
    Set {
        key: Vec<u8>,
        previous_value: Option<Vec<u8>>,
    },
    /// We removed a key. `previous_value` is the old value if it existed.
    Remove {
        key: Vec<u8>,
        previous_value: Option<Vec<u8>>,
    },
}

impl StateChange {
    /// Undo this change by reverting the overlay to its prior state.
    fn revert(self, overlay: &mut HashMap<Vec<u8>, Option<Vec<u8>>>) {
        match self {
            StateChange::Set {
                key,
                previous_value,
            } => {
                // We "Set" this key. Now revert it to previous_value.
                match previous_value {
                    Some(old_value) => {
                        overlay.insert(key, Some(old_value));
                    }
                    None => {
                        // If there was no old_value, remove from overlay (fallback to store).
                        overlay.remove(&key);
                    }
                }
            }
            StateChange::Remove {
                key,
                previous_value,
            } => {
                // We "Remove"d this key. Now revert it to previous_value.
                match previous_value {
                    Some(old_value) => {
                        overlay.insert(key, Some(old_value));
                    }
                    None => {
                        // If old_value was None, remove from overlay to revert.
                        overlay.remove(&key);
                    }
                }
            }
        }
    }
}

/// The checkpoint overlay for a read-only store `S`.
/// The overlay is tracked in `overlay`, which can have:
///  - `Some(value)` => key is set to `value`
///  - `None` => key is explicitly removed / tombstone
///  - no entry => fallback to the underlying store
pub struct Checkpoint<'a, S> {
    /// The underlying store to fallback to.
    base_storage: &'a S,
    /// The in-memory overlay that records changes.
    overlay: HashMap<Vec<u8>, Option<Vec<u8>>>,
    /// The log of state changes (undo log) used for checkpoint restore.
    undo_log: Vec<StateChange>,
}

impl<'a, S> Checkpoint<'a, S> {
    pub fn new(base_storage: &'a S) -> Self {
        Self {
            base_storage,
            overlay: HashMap::new(),
            undo_log: Vec::new(),
        }
    }

    pub fn into_changes(self) -> Vec<CoreStateChange> {
        // The final overlay is stored in self.overlay.
        // For each key in the overlay:
        //   - If the value is Some(v), then we want to persist a "set" operation.
        //   - If the value is None, then we want to persist a "remove" operation.
        self.overlay
            .into_iter()
            .map(|(key, maybe_value)| match maybe_value {
                Some(value) => CoreStateChange::Set { key, value },
                None => CoreStateChange::Remove { key },
            })
            .collect()
    }
}

impl<'a, S: ReadonlyKV> Checkpoint<'a, S> {
    /// Retrieves the "logical" value for `key`.
    ///  1) If overlay has key => check if it's Some(...) or None.
    ///  2) Else fallback to the underlying store.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        if let Some(opt) = self.overlay.get(key) {
            // If the overlay says Some(v), return that.
            // If the overlay says None, it's explicitly removed => return None.
            return Ok(opt.clone());
        }
        // No overlay entry => fallback to underlying store
        self.base_storage.get(key)
    }

    /// Sets `key` to `value`, recording the old logical value for undo.
    pub fn set(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), ErrorCode> {
        // Get old logical value (including store fallback).
        let old_value = self.get(key)?;

        // Record that we changed the value
        self.undo_log.push(StateChange::Set {
            key: key.to_vec(),
            previous_value: old_value,
        });

        // Actually set in overlay (Some(...) => not removed).
        self.overlay.insert(key.to_vec(), Some(value));
        Ok(())
    }

    /// Removes `key` from the "logical" store, recording old value for undo.
    pub fn remove(&mut self, key: &[u8]) -> Result<(), ErrorCode> {
        // Get old logical value.
        let old_value = self.get(key)?;

        self.undo_log.push(StateChange::Remove {
            key: key.to_vec(),
            previous_value: old_value,
        });

        // Insert a "tombstone" => explicitly removed.
        self.overlay.insert(key.to_vec(), None);
        Ok(())
    }

    /// Returns a checkpoint ID (just an index in the undo log).
    pub fn checkpoint(&self) -> u64 {
        self.undo_log.len() as u64
    }

    /// Restores the overlay to the given checkpoint by popping changes.
    pub fn restore(&mut self, checkpoint: u64) {
        while self.undo_log.len() as u64 > checkpoint {
            let change = self.undo_log.pop().unwrap();
            change.revert(&mut self.overlay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // bring in the Checkpoint and StateChange
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

        // Key doesn't exist yet.
        assert_eq!(cp.get(b"hello").unwrap(), None);

        // Set the key.
        cp.set(b"hello", b"world".to_vec()).unwrap();

        // Now we should see it in the overlay.
        assert_eq!(cp.get(b"hello").unwrap(), Some(b"world".to_vec()));
    }

    /// Test removing a key and verifying it no longer appears in the overlay.
    #[test]
    fn test_remove() {
        // Start with a backing store that already has "alpha" => "beta".
        let storage = MockReadonlyKV::new_with_data(&[(b"alpha", b"beta")]);
        let mut cp = Checkpoint::new(&storage);

        // Key "alpha" should come from the backing store.
        assert_eq!(cp.get(b"alpha").unwrap(), Some(b"beta".to_vec()));

        // Remove "alpha".
        cp.remove(b"alpha").unwrap();

        // Now "alpha" is gone (overlaid removal).
        assert_eq!(cp.get(b"alpha").unwrap(), None);
    }

    /// Test that checkpointing and restoring reverts set/remove operations.
    #[test]
    fn test_checkpoint_restore() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        // Set key1 => "A".
        cp.set(b"key1", b"A".to_vec()).unwrap();
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"A".to_vec()));

        // Take a checkpoint here.
        let c1 = cp.checkpoint();

        // Set key1 => "B" and key2 => "ZZ".
        cp.set(b"key1", b"B".to_vec()).unwrap();
        cp.set(b"key2", b"ZZ".to_vec()).unwrap();

        // Now:
        //  key1 => "B"
        //  key2 => "ZZ"
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"B".to_vec()));
        assert_eq!(cp.get(b"key2").unwrap(), Some(b"ZZ".to_vec()));

        // Restore back to c1.
        cp.restore(c1);

        // After restoring:
        //   key1 should go back to "A"
        //   key2 was never set prior to c1, so it should disappear.
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"A".to_vec()));
        assert_eq!(cp.get(b"key2").unwrap(), None);
    }

    /// Test multiple checkpoints and partial restores.
    #[test]
    fn test_multiple_checkpoints() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        // Start with nothing, set key1 => "one".
        cp.set(b"key1", b"one".to_vec()).unwrap();
        // checkpoint #1.
        let c1 = cp.checkpoint();

        // Overwrite key1 => "uno".
        cp.set(b"key1", b"uno".to_vec()).unwrap();
        // checkpoint #2.
        let c2 = cp.checkpoint();

        // Overwrite key1 => "eins".
        cp.set(b"key1", b"eins".to_vec()).unwrap();

        // Confirm current state.
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"eins".to_vec()));

        // Restore to checkpoint #2 => "uno".
        cp.restore(c2);
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"uno".to_vec()));

        // Restore to checkpoint #1 => "one".
        cp.restore(c1);
        assert_eq!(cp.get(b"key1").unwrap(), Some(b"one".to_vec()));
    }

    /// Test removing a key that was originally in the store, and then restoring.
    #[test]
    fn test_remove_and_restore() {
        // Backing store has key => "store_val".
        let storage = MockReadonlyKV::new_with_data(&[(b"key", b"store_val")]);
        let mut cp = Checkpoint::new(&storage);

        // checkpoint #1.
        let c1 = cp.checkpoint();

        // Remove key.
        cp.remove(b"key").unwrap();
        assert_eq!(cp.get(b"key").unwrap(), None);

        // Restore to c1.
        cp.restore(c1);
        // Key should come back from the backing store.
        assert_eq!(cp.get(b"key").unwrap(), Some(b"store_val".to_vec()));
    }

    /// Test removing a key that doesn't exist in either store or overlay,
    /// then restoring (should be a no-op).
    #[test]
    fn test_remove_nonexistent_key() {
        let storage = MockReadonlyKV::new(); // empty store.
        let mut cp = Checkpoint::new(&storage);

        let c1 = cp.checkpoint();
        cp.remove(b"nonexistent").unwrap();

        // Should still be None.
        assert_eq!(cp.get(b"nonexistent").unwrap(), None);

        // Restore.
        cp.restore(c1);

        // Still None.
        assert_eq!(cp.get(b"nonexistent").unwrap(), None);
    }

    /// Test setting a key multiple times and verify that each restore
    /// undoes the last set, returning the value to its prior state.
    #[test]
    fn test_multiple_sets_of_same_key() {
        let storage = MockReadonlyKV::new();
        let mut cp = Checkpoint::new(&storage);

        cp.set(b"k", b"v1".to_vec()).unwrap();
        let c1 = cp.checkpoint();

        // Overwrite: k => "v2".
        cp.set(b"k", b"v2".to_vec()).unwrap();
        let c2 = cp.checkpoint();

        // Overwrite: k => "v3".
        cp.set(b"k", b"v3".to_vec()).unwrap();
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v3".to_vec()));

        // Restore to c2 => "v2".
        cp.restore(c2);
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v2".to_vec()));

        // Restore to c1 => "v1".
        cp.restore(c1);
        assert_eq!(cp.get(b"k").unwrap(), Some(b"v1".to_vec()));
    }
}
