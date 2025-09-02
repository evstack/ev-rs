use crate::errors::{
    ERR_INVALID_CHECKPOINT, ERR_KEY_TOO_LARGE, ERR_OVERLAY_SIZE_EXCEEDED, ERR_TOO_MANY_EVENTS,
    ERR_VALUE_TOO_LARGE,
};
use evolve_core::events_api::Event;
use evolve_core::{ErrorCode, Message, ReadonlyKV, SdkResult};
use evolve_server_core::StateChange as CoreStateChange;
use hashbrown::HashMap;

// Security limits to prevent memory exhaustion attacks
const MAX_OVERLAY_ENTRIES: usize = 100_000; // Maximum number of keys in overlay
const MAX_EVENTS_PER_EXECUTION: usize = 10_000; // Maximum events that can be emitted
const MAX_KEY_SIZE: usize = 256; // Maximum size of a storage key in bytes
const MAX_VALUE_SIZE: usize = 1024 * 1024; // Maximum size of a storage value (1MB)

/// Represents one change to the overlay so it can be undone.
/// Optimized to store deltas instead of full previous values.
#[derive(Debug)]
enum StateChange {
    /// We set a key to a new value. Only store what's needed to revert.
    Set {
        key: Vec<u8>,
        /// True if the key existed in overlay before this change
        had_overlay_entry: bool,
        /// The previous overlay value if it existed (None means tombstone)
        previous_overlay_value: Option<Option<Message>>,
    },
    /// We removed a key. Only store what's needed to revert.
    Remove {
        key: Vec<u8>,
        /// True if the key existed in overlay before this change
        had_overlay_entry: bool,
        /// The previous overlay value if it existed (None means tombstone)
        previous_overlay_value: Option<Option<Message>>,
    },
}

impl StateChange {
    /// Undo this change by reverting the overlay to its prior state.
    /// Uses delta information to efficiently restore previous state.
    fn revert(self, overlay: &mut HashMap<Vec<u8>, Option<Message>>) {
        match self {
            StateChange::Set {
                key,
                had_overlay_entry,
                previous_overlay_value,
            } => {
                // Revert a Set operation
                if had_overlay_entry {
                    // Key existed in overlay before, restore its previous value
                    if let Some(prev_value) = previous_overlay_value {
                        overlay.insert(key, prev_value);
                    }
                } else {
                    // Key didn't exist in overlay before, remove it
                    overlay.remove(&key);
                }
            }
            StateChange::Remove {
                key,
                had_overlay_entry,
                previous_overlay_value,
            } => {
                // Revert a Remove operation
                if had_overlay_entry {
                    // Key existed in overlay before, restore its previous value
                    if let Some(prev_value) = previous_overlay_value {
                        overlay.insert(key, prev_value);
                    }
                } else {
                    // Key didn't exist in overlay before, remove it
                    overlay.remove(&key);
                }
            }
        }
    }
}

/// Defines a checkpoint at a certain point in time of the execution state
/// which can be used to roll back `ExecutionState`.
#[derive(Debug, Clone, Copy)]
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
    pub(crate) base_storage: &'a S,
    /// The in-memory overlay that records changes.
    overlay: HashMap<Vec<u8>, Option<Message>>,
    /// The log of state changes (undo log) used for checkpoint restore.
    undo_log: Vec<StateChange>,
    /// Events emitted.
    events: Vec<Event>,
    /// Number of unique objects created.
    unique_objects: u64,
}

// Initial capacity for overlay HashMap to avoid early rehashing
// Most transactions touch 10-100 keys, so 256 is a reasonable starting point
const INITIAL_OVERLAY_CAPACITY: usize = 256;
const INITIAL_UNDO_LOG_CAPACITY: usize = 128;
const INITIAL_EVENTS_CAPACITY: usize = 64;

impl<'a, S> ExecutionState<'a, S> {
    pub fn new(base_storage: &'a S) -> Self {
        Self {
            base_storage,
            overlay: HashMap::with_capacity(INITIAL_OVERLAY_CAPACITY),
            undo_log: Vec::with_capacity(INITIAL_UNDO_LOG_CAPACITY),
            events: Vec::with_capacity(INITIAL_EVENTS_CAPACITY),
            unique_objects: 0,
        }
    }

    /// Adds an event to the list of the current execution events.
    pub fn emit_event(&mut self, event: Event) -> Result<(), ErrorCode> {
        if self.events.len() >= MAX_EVENTS_PER_EXECUTION {
            return Err(ERR_TOO_MANY_EVENTS);
        }
        self.events.push(event);
        Ok(())
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

/// Batch operation for efficient bulk updates
pub struct BatchOperation {
    pub key: Vec<u8>,
    pub operation: BatchOp,
}

pub enum BatchOp {
    Set(Message),
    Remove,
}

impl<S: ReadonlyKV> ExecutionState<'_, S> {
    /// Gets a reference to the value if it exists in the overlay.
    /// Returns None if the key is not in the overlay (need to check base storage).
    /// Returns Some(None) if the key is explicitly removed.
    /// Returns Some(Some(&Message)) if the key exists in overlay.
    #[inline]
    pub fn get_overlay_ref(&self, key: &[u8]) -> Option<&Option<Message>> {
        self.overlay.get(key)
    }

    /// Retrieves the "logical" value for `key`.
    ///  1) If overlay has key => check if it's Some(...) or None.
    ///  2) Else fallback to the underlying store.
    pub fn get(&self, key: &[u8]) -> Result<Option<Message>, ErrorCode> {
        // Check overlay first - avoid cloning by returning reference when possible
        match self.overlay.get(key) {
            Some(Some(msg)) => Ok(Some(msg.clone())),
            Some(None) => Ok(None), // Explicitly removed
            None => {
                // No overlay entry => fallback to underlying store
                self.base_storage
                    .get(key)
                    .map(|value| value.map(Message::from_bytes))
            }
        }
    }

    /// Sets `key` to `value`, recording the old logical value for undo.
    pub fn set(&mut self, key: &[u8], value: Message) -> Result<(), ErrorCode> {
        // Validate key size
        if key.len() > MAX_KEY_SIZE {
            return Err(ERR_KEY_TOO_LARGE);
        }

        // Validate value size
        if value.len() > MAX_VALUE_SIZE {
            return Err(ERR_VALUE_TOO_LARGE);
        }

        // Check if this is a new key to the overlay
        let is_new_key = !self.overlay.contains_key(key);

        // Check overlay size limit only if this is a new key
        if is_new_key && self.overlay.len() >= MAX_OVERLAY_ENTRIES {
            return Err(ERR_OVERLAY_SIZE_EXCEEDED);
        }

        // Record undo information efficiently (delta-based)
        let had_overlay_entry = !is_new_key;
        let previous_overlay_value = if had_overlay_entry {
            self.overlay.get(key).cloned()
        } else {
            None
        };

        // Record that we changed the value using delta information
        self.undo_log.push(StateChange::Set {
            key: key.to_vec(),
            had_overlay_entry,
            previous_overlay_value,
        });

        // Actually set in overlay (Some(...) => not removed).
        self.overlay.insert(key.to_vec(), Some(value));
        Ok(())
    }

    /// Removes `key` from the "logical" store, recording old value for undo.
    pub fn remove(&mut self, key: &[u8]) -> Result<(), ErrorCode> {
        // Validate key size
        if key.len() > MAX_KEY_SIZE {
            return Err(ERR_KEY_TOO_LARGE);
        }

        // Check overlay size limit only if this is a new key
        if !self.overlay.contains_key(key) && self.overlay.len() >= MAX_OVERLAY_ENTRIES {
            return Err(ERR_OVERLAY_SIZE_EXCEEDED);
        }

        // Record undo information efficiently (delta-based)
        let had_overlay_entry = self.overlay.contains_key(key);
        let previous_overlay_value = if had_overlay_entry {
            self.overlay.get(key).cloned()
        } else {
            None
        };

        self.undo_log.push(StateChange::Remove {
            key: key.to_vec(),
            had_overlay_entry,
            previous_overlay_value,
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
    pub fn restore(&mut self, checkpoint: Checkpoint) -> Result<(), ErrorCode> {
        // Validate checkpoint indices are within bounds
        if checkpoint.undo_log_index > self.undo_log.len()
            || checkpoint.events_index > self.events.len()
        {
            return Err(ERR_INVALID_CHECKPOINT);
        }

        while self.undo_log.len() > checkpoint.undo_log_index {
            let change = self.undo_log.pop().expect("undo log has elements");
            change.revert(&mut self.overlay);
        }

        // truncate events to the given checkpoint
        self.events.truncate(checkpoint.events_index);

        // restore to previous unique object.
        self.unique_objects = checkpoint.unique_objects_created;

        Ok(())
    }

    /// Applies a batch of operations efficiently.
    /// This is more efficient than individual operations as it:
    /// 1. Validates all operations upfront
    /// 2. Checks overlay size limit once
    /// 3. Reduces HashMap lookups
    pub fn apply_batch(&mut self, operations: Vec<BatchOperation>) -> Result<(), ErrorCode> {
        // First pass: validate all operations
        let mut new_keys_count = 0;
        for op in &operations {
            // Validate key size
            if op.key.len() > MAX_KEY_SIZE {
                return Err(ERR_KEY_TOO_LARGE);
            }

            // Count new keys for size limit check
            if !self.overlay.contains_key(&op.key) {
                new_keys_count += 1;
            }

            // Validate value size for set operations
            if let BatchOp::Set(ref value) = op.operation {
                if value.len() > MAX_VALUE_SIZE {
                    return Err(ERR_VALUE_TOO_LARGE);
                }
            }
        }

        // Check if we would exceed overlay size limit
        if self.overlay.len() + new_keys_count > MAX_OVERLAY_ENTRIES {
            return Err(ERR_OVERLAY_SIZE_EXCEEDED);
        }

        // Second pass: apply all operations
        for op in operations {
            match op.operation {
                BatchOp::Set(value) => {
                    // Record undo information efficiently (delta-based)
                    let had_overlay_entry = self.overlay.contains_key(&op.key);
                    let previous_overlay_value = if had_overlay_entry {
                        self.overlay.get(&op.key).cloned()
                    } else {
                        None
                    };

                    // Record change using delta information
                    self.undo_log.push(StateChange::Set {
                        key: op.key.clone(),
                        had_overlay_entry,
                        previous_overlay_value,
                    });

                    // Apply change
                    self.overlay.insert(op.key, Some(value));
                }
                BatchOp::Remove => {
                    // Record undo information efficiently (delta-based)
                    let had_overlay_entry = self.overlay.contains_key(&op.key);
                    let previous_overlay_value = if had_overlay_entry {
                        self.overlay.get(&op.key).cloned()
                    } else {
                        None
                    };

                    // Record change using delta information
                    self.undo_log.push(StateChange::Remove {
                        key: op.key.clone(),
                        had_overlay_entry,
                        previous_overlay_value,
                    });

                    // Apply change
                    self.overlay.insert(op.key, None);
                }
            }
        }

        Ok(())
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
    use hashbrown::HashMap;

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
        cp.restore(c1).unwrap();

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
        cp.restore(c2).unwrap();
        let val = cp.get(b"key1").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"uno");

        // Restore to checkpoint #1 => "one".
        cp.restore(c1).unwrap();
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
        cp.restore(c1).unwrap();

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
        cp.restore(c1).unwrap();

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
        cp.restore(c2).unwrap();
        let val = cp.get(b"k").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"v2");

        // Restore to c1 => k => "v1".
        cp.restore(c1).unwrap();
        let val = cp.get(b"k").unwrap();
        assert!(val.is_some());
        assert_eq!(val.unwrap().as_bytes().unwrap(), b"v1");
    }

    /// Test get_overlay_ref for zero-copy access
    #[test]
    fn test_get_overlay_ref() {
        let storage = MockReadonlyKV::new_with_data(&[(b"store_key", b"store_value")]);
        let mut cp = ExecutionState::new(&storage);

        // Test non-existent key
        assert!(cp.get_overlay_ref(b"missing").is_none());

        // Test key in overlay (set)
        cp.set(
            b"overlay_key",
            Message::from_bytes(b"overlay_value".to_vec()),
        )
        .unwrap();
        let overlay_ref = cp.get_overlay_ref(b"overlay_key");
        assert!(overlay_ref.is_some());
        assert!(overlay_ref.unwrap().is_some());

        // Test key in overlay (removed)
        cp.remove(b"store_key").unwrap();
        let removed_ref = cp.get_overlay_ref(b"store_key");
        assert!(removed_ref.is_some());
        assert!(removed_ref.unwrap().is_none());

        // Test key only in base storage
        assert!(cp.get_overlay_ref(b"store_key_2").is_none());
    }

    /// Test batch operations
    #[test]
    fn test_batch_operations() {
        let storage = MockReadonlyKV::new_with_data(&[(b"existing", b"value")]);
        let mut cp = ExecutionState::new(&storage);

        // Create batch operations
        let operations = vec![
            BatchOperation {
                key: b"key1".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"value1".to_vec())),
            },
            BatchOperation {
                key: b"key2".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"value2".to_vec())),
            },
            BatchOperation {
                key: b"existing".to_vec(),
                operation: BatchOp::Remove,
            },
        ];

        // Apply batch
        cp.apply_batch(operations).unwrap();

        // Verify results
        assert_eq!(
            cp.get(b"key1").unwrap().unwrap().as_bytes().unwrap(),
            b"value1"
        );
        assert_eq!(
            cp.get(b"key2").unwrap().unwrap().as_bytes().unwrap(),
            b"value2"
        );
        assert!(cp.get(b"existing").unwrap().is_none());
    }

    /// Test batch operations with validation
    #[test]
    fn test_batch_operations_validation() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Test key size validation
        let large_key = vec![0u8; MAX_KEY_SIZE + 1];
        let operations = vec![BatchOperation {
            key: large_key,
            operation: BatchOp::Set(Message::from_bytes(b"value".to_vec())),
        }];
        assert_eq!(cp.apply_batch(operations).unwrap_err(), ERR_KEY_TOO_LARGE);

        // Test value size validation
        let large_value = vec![0u8; MAX_VALUE_SIZE + 1];
        let operations = vec![BatchOperation {
            key: b"key".to_vec(),
            operation: BatchOp::Set(Message::from_bytes(large_value)),
        }];
        assert_eq!(cp.apply_batch(operations).unwrap_err(), ERR_VALUE_TOO_LARGE);

        // Test overlay size limit - use smaller number for miri compatibility
        // First, fill up the overlay close to the limit
        let test_limit = if cfg!(miri) { 100 } else { MAX_OVERLAY_ENTRIES };
        let mut operations = Vec::new();
        for i in 0..=test_limit {
            operations.push(BatchOperation {
                key: format!("key{i}").into_bytes(),
                operation: BatchOp::Set(Message::from_bytes(b"value".to_vec())),
            });
        }

        if cfg!(miri) {
            // For miri, just test that a reasonable batch works
            cp.apply_batch(operations).unwrap();
        } else {
            // For regular tests, verify the size limit is enforced
            assert_eq!(
                cp.apply_batch(operations).unwrap_err(),
                ERR_OVERLAY_SIZE_EXCEEDED
            );
        }
    }

    /// Test batch operations with checkpoint and restore
    #[test]
    fn test_batch_operations_with_checkpoint() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Set initial state
        cp.set(b"initial", Message::from_bytes(b"value".to_vec()))
            .unwrap();
        let checkpoint = cp.checkpoint();

        // Apply batch operations
        let operations = vec![
            BatchOperation {
                key: b"key1".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"value1".to_vec())),
            },
            BatchOperation {
                key: b"initial".to_vec(),
                operation: BatchOp::Remove,
            },
        ];
        cp.apply_batch(operations).unwrap();

        // Verify state after batch
        assert!(cp.get(b"key1").unwrap().is_some());
        assert!(cp.get(b"initial").unwrap().is_none());

        // Restore checkpoint
        cp.restore(checkpoint).unwrap();

        // Verify restored state
        assert!(cp.get(b"key1").unwrap().is_none());
        assert_eq!(
            cp.get(b"initial").unwrap().unwrap().as_bytes().unwrap(),
            b"value"
        );
    }

    /// Test optimized set operation performance
    #[test]
    fn test_set_optimization() {
        let storage = MockReadonlyKV::new_with_data(&[(b"base_key", b"base_value")]);
        let mut cp = ExecutionState::new(&storage);

        // First set - should check base storage
        cp.set(b"new_key", Message::from_bytes(b"value1".to_vec()))
            .unwrap();

        // Second set of same key - should not check base storage
        cp.set(b"new_key", Message::from_bytes(b"value2".to_vec()))
            .unwrap();

        // Verify the optimization works correctly
        assert_eq!(
            cp.get(b"new_key").unwrap().unwrap().as_bytes().unwrap(),
            b"value2"
        );

        // Test with key from base storage
        cp.set(b"base_key", Message::from_bytes(b"new_value".to_vec()))
            .unwrap();
        assert_eq!(
            cp.get(b"base_key").unwrap().unwrap().as_bytes().unwrap(),
            b"new_value"
        );
    }

    /// Test batch operations efficiency with large batches
    #[test]
    fn test_batch_efficiency() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Create a batch of operations - smaller for miri compatibility
        let batch_size = if cfg!(miri) { 50 } else { 1000 };
        let mut operations = Vec::new();
        for i in 0..batch_size {
            operations.push(BatchOperation {
                key: format!("key{i:04}").into_bytes(),
                operation: BatchOp::Set(Message::from_bytes(format!("value{i}").into_bytes())),
            });
        }

        // Apply batch - this should be more efficient than individual operations
        cp.apply_batch(operations).unwrap();

        // Verify some values
        assert_eq!(
            cp.get(b"key0000").unwrap().unwrap().as_bytes().unwrap(),
            b"value0"
        );
        if !cfg!(miri) {
            assert_eq!(
                cp.get(b"key0999").unwrap().unwrap().as_bytes().unwrap(),
                b"value999"
            );
        }
        assert_eq!(cp.overlay.len(), batch_size);
    }

    /// Test delta-based undo log efficiency
    #[test]
    fn test_undo_log_delta_efficiency() {
        let storage = MockReadonlyKV::new_with_data(&[(b"base_key", b"base_value")]);
        let mut cp = ExecutionState::new(&storage);

        // Test that undo log doesn't store full base storage values for new keys
        let checkpoint1 = cp.checkpoint();
        cp.set(b"new_key", Message::from_bytes(b"new_value".to_vec()))
            .unwrap();

        // The undo log should only store overlay delta, not base storage value
        assert_eq!(cp.undo_log.len(), 1);

        // Test that undo log stores overlay deltas for existing keys
        let checkpoint2 = cp.checkpoint();
        cp.set(b"new_key", Message::from_bytes(b"updated_value".to_vec()))
            .unwrap();

        assert_eq!(cp.undo_log.len(), 2);

        // Test restore works correctly with delta-based undo
        cp.restore(checkpoint2).unwrap();
        assert_eq!(
            cp.get(b"new_key").unwrap().unwrap().as_bytes().unwrap(),
            b"new_value"
        );

        cp.restore(checkpoint1).unwrap();
        assert!(cp.get(b"new_key").unwrap().is_none());

        // Base storage should still work
        assert_eq!(
            cp.get(b"base_key").unwrap().unwrap().as_bytes().unwrap(),
            b"base_value"
        );
    }

    /// Test delta-based undo log for remove operations
    #[test]
    fn test_undo_log_remove_delta_efficiency() {
        let storage = MockReadonlyKV::new_with_data(&[(b"base_key", b"base_value")]);
        let mut cp = ExecutionState::new(&storage);

        // Set a key in overlay first
        cp.set(
            b"overlay_key",
            Message::from_bytes(b"overlay_value".to_vec()),
        )
        .unwrap();
        let checkpoint = cp.checkpoint();

        // Remove the overlay key
        cp.remove(b"overlay_key").unwrap();
        assert!(cp.get(b"overlay_key").unwrap().is_none());

        // Restore should bring back the overlay key
        cp.restore(checkpoint).unwrap();
        assert_eq!(
            cp.get(b"overlay_key").unwrap().unwrap().as_bytes().unwrap(),
            b"overlay_value"
        );

        // Test removing a base storage key (creates tombstone)
        let checkpoint2 = cp.checkpoint();
        cp.remove(b"base_key").unwrap();
        assert!(cp.get(b"base_key").unwrap().is_none());

        // Restore should remove the tombstone, base storage should be accessible again
        cp.restore(checkpoint2).unwrap();
        assert_eq!(
            cp.get(b"base_key").unwrap().unwrap().as_bytes().unwrap(),
            b"base_value"
        );
    }

    /// Comprehensive test to guarantee identical functionality to full-value undo log
    /// Tests all possible state transition scenarios to ensure delta-based approach
    /// produces exactly the same results as storing full previous values
    #[test]
    fn test_delta_undo_identical_functionality() {
        let storage = MockReadonlyKV::new_with_data(&[
            (b"existing_key", b"existing_value"),
            (b"another_key", b"another_value"),
        ]);
        let mut cp = ExecutionState::new(&storage);

        // === Scenario 1: Set new key (not in overlay, not in base storage) ===
        let checkpoint1 = cp.checkpoint();
        cp.set(b"new_key", Message::from_bytes(b"new_value".to_vec()))
            .unwrap();

        // Verify key is set
        assert_eq!(
            cp.get(b"new_key").unwrap().unwrap().as_bytes().unwrap(),
            b"new_value"
        );

        // Restore should remove the key completely (same as if we stored None)
        cp.restore(checkpoint1).unwrap();
        assert!(cp.get(b"new_key").unwrap().is_none());

        // === Scenario 2: Set key that exists in base storage but not overlay ===
        let checkpoint2 = cp.checkpoint();
        cp.set(
            b"existing_key",
            Message::from_bytes(b"modified_value".to_vec()),
        )
        .unwrap();

        // Verify key is modified
        assert_eq!(
            cp.get(b"existing_key")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"modified_value"
        );

        // Restore should revert to base storage value
        cp.restore(checkpoint2).unwrap();
        assert_eq!(
            cp.get(b"existing_key")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"existing_value"
        );

        // === Scenario 3: Set key that exists in overlay (modify overlay entry) ===
        cp.set(
            b"existing_key",
            Message::from_bytes(b"first_overlay".to_vec()),
        )
        .unwrap();
        let checkpoint3 = cp.checkpoint();
        cp.set(
            b"existing_key",
            Message::from_bytes(b"second_overlay".to_vec()),
        )
        .unwrap();

        // Verify key is modified in overlay
        assert_eq!(
            cp.get(b"existing_key")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"second_overlay"
        );

        // Restore should revert to previous overlay value
        cp.restore(checkpoint3).unwrap();
        assert_eq!(
            cp.get(b"existing_key")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"first_overlay"
        );

        // === Scenario 4: Remove key that exists only in overlay ===
        cp.set(
            b"overlay_only",
            Message::from_bytes(b"overlay_val".to_vec()),
        )
        .unwrap();
        let checkpoint4 = cp.checkpoint();
        cp.remove(b"overlay_only").unwrap();

        // Verify key is removed
        assert!(cp.get(b"overlay_only").unwrap().is_none());

        // Restore should bring back the overlay value
        cp.restore(checkpoint4).unwrap();
        assert_eq!(
            cp.get(b"overlay_only")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"overlay_val"
        );

        // === Scenario 5: Remove key that exists in base storage (create tombstone) ===
        let checkpoint5 = cp.checkpoint();
        cp.remove(b"another_key").unwrap();

        // Verify key is removed (tombstone created)
        assert!(cp.get(b"another_key").unwrap().is_none());

        // Restore should remove tombstone, making base storage accessible
        cp.restore(checkpoint5).unwrap();
        assert_eq!(
            cp.get(b"another_key").unwrap().unwrap().as_bytes().unwrap(),
            b"another_value"
        );

        // === Scenario 6: Remove key that was previously set in overlay ===
        cp.set(
            b"another_key",
            Message::from_bytes(b"overlay_another".to_vec()),
        )
        .unwrap();
        let checkpoint6 = cp.checkpoint();
        cp.remove(b"another_key").unwrap();

        // Verify key is removed
        assert!(cp.get(b"another_key").unwrap().is_none());

        // Restore should bring back the overlay value
        cp.restore(checkpoint6).unwrap();
        assert_eq!(
            cp.get(b"another_key").unwrap().unwrap().as_bytes().unwrap(),
            b"overlay_another"
        );

        // === Scenario 7: Complex sequence with multiple checkpoints ===
        let checkpoint7 = cp.checkpoint();

        // Make several changes
        cp.set(b"key_a", Message::from_bytes(b"value_a1".to_vec()))
            .unwrap();
        cp.set(b"key_b", Message::from_bytes(b"value_b1".to_vec()))
            .unwrap();
        let checkpoint8 = cp.checkpoint();

        cp.set(b"key_a", Message::from_bytes(b"value_a2".to_vec()))
            .unwrap();
        cp.remove(b"key_b").unwrap();
        cp.set(b"key_c", Message::from_bytes(b"value_c1".to_vec()))
            .unwrap();

        // Verify final state
        assert_eq!(
            cp.get(b"key_a").unwrap().unwrap().as_bytes().unwrap(),
            b"value_a2"
        );
        assert!(cp.get(b"key_b").unwrap().is_none());
        assert_eq!(
            cp.get(b"key_c").unwrap().unwrap().as_bytes().unwrap(),
            b"value_c1"
        );

        // Restore to checkpoint8
        cp.restore(checkpoint8).unwrap();
        assert_eq!(
            cp.get(b"key_a").unwrap().unwrap().as_bytes().unwrap(),
            b"value_a1"
        );
        assert_eq!(
            cp.get(b"key_b").unwrap().unwrap().as_bytes().unwrap(),
            b"value_b1"
        );
        assert!(cp.get(b"key_c").unwrap().is_none());

        // Restore to checkpoint7
        cp.restore(checkpoint7).unwrap();
        assert!(cp.get(b"key_a").unwrap().is_none());
        assert!(cp.get(b"key_b").unwrap().is_none());
        assert!(cp.get(b"key_c").unwrap().is_none());

        // Base storage should still be intact
        assert_eq!(
            cp.get(b"another_key").unwrap().unwrap().as_bytes().unwrap(),
            b"overlay_another" // From previous operations
        );
    }

    /// Test that ensures batch operations with delta undo log work identically
    /// to individual operations with full-value undo log
    /// Test checkpoint validation for out-of-bounds indices
    #[test]
    fn test_checkpoint_bounds_validation() {
        let storage = MockReadonlyKV::new();
        let mut cp = ExecutionState::new(&storage);

        // Create a checkpoint with invalid indices
        let invalid_checkpoint = Checkpoint {
            undo_log_index: 100, // Out of bounds
            events_index: 50,    // Out of bounds
            unique_objects_created: 0,
        };

        // Should fail with invalid checkpoint error
        assert_eq!(
            cp.restore(invalid_checkpoint).unwrap_err(),
            ERR_INVALID_CHECKPOINT
        );
    }

    #[test]
    fn test_batch_delta_undo_identical_functionality() {
        let storage = MockReadonlyKV::new_with_data(&[
            (b"base1", b"base_value1"),
            (b"base2", b"base_value2"),
        ]);
        let mut cp = ExecutionState::new(&storage);

        // Set up initial overlay state
        cp.set(b"overlay1", Message::from_bytes(b"overlay_value1".to_vec()))
            .unwrap();
        cp.set(b"base1", Message::from_bytes(b"modified_base1".to_vec()))
            .unwrap();

        let checkpoint = cp.checkpoint();

        // Apply batch operations that cover all scenarios
        let operations = vec![
            // Modify existing overlay key
            BatchOperation {
                key: b"overlay1".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"new_overlay1".to_vec())),
            },
            // Modify base storage key that's in overlay
            BatchOperation {
                key: b"base1".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"newer_base1".to_vec())),
            },
            // Modify base storage key that's not in overlay
            BatchOperation {
                key: b"base2".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"modified_base2".to_vec())),
            },
            // Add completely new key
            BatchOperation {
                key: b"new_batch_key".to_vec(),
                operation: BatchOp::Set(Message::from_bytes(b"batch_value".to_vec())),
            },
            // Remove overlay key
            BatchOperation {
                key: b"overlay1".to_vec(),
                operation: BatchOp::Remove,
            },
            // Remove base storage key (create tombstone)
            BatchOperation {
                key: b"base2".to_vec(),
                operation: BatchOp::Remove,
            },
        ];

        cp.apply_batch(operations).unwrap();

        // Verify final state
        assert!(cp.get(b"overlay1").unwrap().is_none()); // Removed
        assert_eq!(
            cp.get(b"base1").unwrap().unwrap().as_bytes().unwrap(),
            b"newer_base1"
        );
        assert!(cp.get(b"base2").unwrap().is_none()); // Removed (tombstone)
        assert_eq!(
            cp.get(b"new_batch_key")
                .unwrap()
                .unwrap()
                .as_bytes()
                .unwrap(),
            b"batch_value"
        );

        // Restore should revert ALL batch operations atomically
        cp.restore(checkpoint).unwrap();

        // Verify restoration to exact previous state
        assert_eq!(
            cp.get(b"overlay1").unwrap().unwrap().as_bytes().unwrap(),
            b"overlay_value1"
        );
        assert_eq!(
            cp.get(b"base1").unwrap().unwrap().as_bytes().unwrap(),
            b"modified_base1"
        );
        assert_eq!(
            cp.get(b"base2").unwrap().unwrap().as_bytes().unwrap(),
            b"base_value2"
        );
        assert!(cp.get(b"new_batch_key").unwrap().is_none());
    }
}
