use evolve_core::events_api::Event;
use evolve_core::{ErrorCode, Message, ReadonlyKV, SdkResult};
use evolve_server_core::StateChange as CoreStateChange;
use std::collections::HashMap;

/// Represents one change to the overlay so it can be undone.
#[derive(Debug)]
enum StateChange {
    /// We set a key to a new value. `previous_value` is what it was logically
    /// (including falling back to storage if needed).
    Set {
        key: Vec<u8>,
        previous_value: Option<Message>,
    },
    /// We removed a key. `previous_value` is the old value if it existed.
    Remove {
        key: Vec<u8>,
        previous_value: Option<Message>,
    },
}

impl StateChange {
    /// Undo this change by reverting the overlay to its prior state.
    fn revert(self, overlay: &mut HashMap<Vec<u8>, Option<Message>>) {
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

/// Defines a checkpoint at a certain point in time of the execution state
/// which can be used to roll back `ExecutionState`.
pub struct Checkpoint {
    undo_log_index: usize,
    events_index: usize,
    unique_objects_created: u64,
}

/// The checkpoint overlay for a read-only store `S`.
/// The overlay is tracked in `overlay`, which can have:
///  - `Some(value)` => key is set to `value`
///  - `None` => key is explicitly removed / tombstone
///  - no entry => fallback to the underlying store
#[derive(Debug)]
pub struct ExecutionState<'a, S> {
    /// The underlying store to fall back to.
    base_storage: &'a S,
    /// The in-memory overlay that records changes.
    overlay: HashMap<Vec<u8>, Option<Message>>,
    /// The log of state changes (undo log) used for checkpoint restore.
    undo_log: Vec<StateChange>,
    /// Events emitted.
    events: Vec<Event>,
    /// Number of unique objects created.
    unique_objects: u64,
}

impl<'a, S> ExecutionState<'a, S> {
    pub fn new(base_storage: &'a S) -> Self {
        Self {
            base_storage,
            overlay: HashMap::new(),
            undo_log: Vec::new(),
            events: Vec::new(),
            unique_objects: 0,
        }
    }

    /// Adds an event to the list of the current execution events.
    pub fn emit_event(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Pops the events
    pub fn pop_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Creates a new unique object id.
    pub fn next_unique_object_id(&mut self) -> u64 {
        self.unique_objects += 1;
        self.unique_objects
    }

    /// Reports the number of objects created during this execution.
    pub fn created_unique_objects(&self) -> u64 {
        self.unique_objects
    }

    pub fn into_changes(self) -> SdkResult<Vec<CoreStateChange>> {
        // The final overlay is stored in self.overlay.
        // For each key in the overlay:
        //   - If the value is Some(v), then we want to persist a "set" operation.
        //   - If the value is None, then we want to persist a "remove" operation.
        self.overlay
            .into_iter()
            .map(|(key, maybe_value)| match maybe_value {
                Some(value) => {
                    let value = value.into_bytes()?;
                    Ok(CoreStateChange::Set { key, value })
                }
                None => Ok(CoreStateChange::Remove { key }),
            })
            .collect()
    }
}

impl<S: ReadonlyKV> ExecutionState<'_, S> {
    /// Retrieves the "logical" value for `key`.
    ///  1) If overlay has key => check if it's Some(...) or None.
    ///  2) Else fallback to the underlying store.
    pub fn get(&self, key: &[u8]) -> Result<Option<Message>, ErrorCode> {
        if let Some(opt) = self.overlay.get(key) {
            // If the overlay says Some(v), return that.
            // If the overlay says None, it's explicitly removed => return None.
            return Ok(opt.clone());
        }
        // No overlay entry => fallback to underlying store
        self.base_storage
            .get(key)
            .map(|value| value.map(Message::from_bytes))
    }

    /// Sets `key` to `value`, recording the old logical value for undo.
    pub fn set(&mut self, key: &[u8], value: Message) -> Result<(), ErrorCode> {
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
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            undo_log_index: self.undo_log.len(),
            events_index: self.events.len(),
            unique_objects_created: self.unique_objects,
        }
    }

    /// Restores the overlay to the given checkpoint by popping changes.
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        while self.undo_log.len() > checkpoint.undo_log_index {
            let change = self.undo_log.pop().unwrap();
            change.revert(&mut self.overlay);
        }

        // truncate events to the given checkpoint
        self.events.truncate(checkpoint.events_index);

        // restore to previous unique object.
        self.unique_objects = checkpoint.unique_objects_created;
    }
}

impl<S: ReadonlyKV> ReadonlyKV for ExecutionState<'_, S> {
    fn get(&self, key: &[u8]) -> SdkResult<Option<Vec<u8>>> {
        self.get(key)?.map(|b| b.into_bytes()).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // bring in the Checkpoint and StateChange
    use evolve_core::{ErrorCode, Message, ReadonlyKV};
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
        let mut cp = ExecutionState::new(&storage);

        // Key doesn't exist yet.
        assert!(cp.get(b"hello").unwrap().is_none());

        // Set the key (store "world" as a Message).
        cp.set(b"hello", Message::from_bytes(b"world".to_vec()))
            .unwrap();

        // Now we should see it in the overlay. Compare via as_bytes().
        let val = cp.get(b"hello").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"world");
    }

    /// Test removing a key and verifying it no longer appears in the overlay.
    #[test]
    fn test_remove() {
        // Start with a backing store that already has "alpha" => "beta".
        let storage = MockReadonlyKV::new_with_data(&[(b"alpha", b"beta")]);
        let mut cp = ExecutionState::new(&storage);

        // Key "alpha" should come from the backing store.
        let val = cp.get(b"alpha").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"beta");

        // Remove "alpha".
        cp.remove(b"alpha").unwrap();

        // Now "alpha" is gone (overlaid removal).
        assert!(cp.get(b"alpha").unwrap().is_none());
    }

    /// Test that checkpointing and restoring reverts set/remove operations.
    #[test]
    fn test_checkpoint_restore() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Set key1 => "A".
        cp.set(b"key1", Message::from_bytes(b"A".to_vec())).unwrap();
        let val = cp.get(b"key1").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"A");

        // Take a checkpoint here.
        let c1 = cp.checkpoint();

        // Set key1 => "B" and key2 => "ZZ".
        cp.set(b"key1", Message::from_bytes(b"B".to_vec())).unwrap();
        cp.set(b"key2", Message::from_bytes(b"ZZ".to_vec()))
            .unwrap();

        // Now:
        //  key1 => "B"
        //  key2 => "ZZ"
        let val1 = cp.get(b"key1").unwrap();
        assert!(val1.is_some());
        assert_eq!(val1.unwrap().as_bytes().unwrap(), b"B");

        let val2 = cp.get(b"key2").unwrap();
        assert!(val2.is_some());
        assert_eq!(val2.unwrap().as_bytes().unwrap(), b"ZZ");

        // Restore back to c1.
        cp.restore(c1);

        // After restoring:
        //   key1 => "A"
        //   key2 => None
        let val1_restored = cp.get(b"key1").unwrap();
        assert!(val1_restored.is_some());
        assert_eq!(val1_restored.unwrap().as_bytes().unwrap(), b"A");

        let val2_restored = cp.get(b"key2").unwrap();
        assert!(val2_restored.is_none());
    }

    /// Test multiple checkpoints and partial restores.
    #[test]
    fn test_multiple_checkpoints() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Start with nothing, set key1 => "one".
        cp.set(b"key1", Message::from_bytes(b"one".to_vec()))
            .unwrap();
        // checkpoint #1.
        let c1 = cp.checkpoint();

        // Overwrite key1 => "uno".
        cp.set(b"key1", Message::from_bytes(b"uno".to_vec()))
            .unwrap();
        // checkpoint #2.
        let c2 = cp.checkpoint();

        // Overwrite key1 => "eins".
        cp.set(b"key1", Message::from_bytes(b"eins".to_vec()))
            .unwrap();

        // Confirm current state.
        let val = cp.get(b"key1").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"eins");

        // Restore to checkpoint #2 => "uno".
        cp.restore(c2);
        let val = cp.get(b"key1").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"uno");

        // Restore to checkpoint #1 => "one".
        cp.restore(c1);
        let val = cp.get(b"key1").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"one");
    }

    /// Test removing a key that was originally in the store, and then restoring.
    #[test]
    fn test_remove_and_restore() {
        // Backing store has key => "store_val".
        let storage = MockReadonlyKV::new_with_data(&[(b"key", b"store_val")]);
        let mut cp = ExecutionState::new(&storage);

        // checkpoint #1.
        let c1 = cp.checkpoint();

        // Remove key.
        cp.remove(b"key").unwrap();
        assert!(cp.get(b"key").unwrap().is_none());

        // Restore to c1.
        cp.restore(c1);

        // Key should come back from the backing store.
        let val = cp.get(b"key").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"store_val");
    }

    /// Test removing a key that doesn't exist in either store or overlay,
    /// then restoring (should be a no-op).
    #[test]
    fn test_remove_nonexistent_key() {
        let storage = MockReadonlyKV::new(); // empty store.
        let mut cp = ExecutionState::new(&storage);

        let c1 = cp.checkpoint();
        cp.remove(b"nonexistent").unwrap();

        // Should still be None.
        assert!(cp.get(b"nonexistent").unwrap().is_none());

        // Restore.
        cp.restore(c1);

        // Still None.
        assert!(cp.get(b"nonexistent").unwrap().is_none());
    }

    /// Test setting a key multiple times and verify that each restore
    /// undoes the last set, returning the value to its prior state.
    #[test]
    fn test_multiple_sets_of_same_key() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Set k => "v1".
        cp.set(b"k", Message::from_bytes(b"v1".to_vec())).unwrap();
        let c1 = cp.checkpoint();

        // Overwrite k => "v2".
        cp.set(b"k", Message::from_bytes(b"v2".to_vec())).unwrap();
        let c2 = cp.checkpoint();

        // Overwrite k => "v3".
        cp.set(b"k", Message::from_bytes(b"v3".to_vec())).unwrap();

        // k should now be "v3".
        let val = cp.get(b"k").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"v3");

        // Restore to c2 => k => "v2".
        cp.restore(c2);
        let val = cp.get(b"k").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"v2");

        // Restore to c1 => k => "v1".
        cp.restore(c1);
        let val = cp.get(b"k").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"v1");
    }
}
