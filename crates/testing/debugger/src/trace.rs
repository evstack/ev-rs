//! Execution trace recording for time-travel debugging.
//!
//! This module captures a detailed log of all state changes and operations
//! during simulation, enabling deterministic replay and debugging.

use evolve_core::AccountId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Convert AccountId to serializable bytes.
fn account_to_bytes(id: AccountId) -> [u8; 16] {
    let bytes = id.as_bytes();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[..16]);
    arr
}

/// Convert bytes back to AccountId.
#[allow(dead_code)]
fn bytes_to_account(bytes: [u8; 16]) -> AccountId {
    let value = u128::from_be_bytes(bytes);
    AccountId::new(value)
}

/// A complete execution trace capturing all state transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    /// The seed used to generate this trace.
    pub seed: u64,
    /// Git commit hash for exact reproduction (optional).
    pub commit_hash: Option<[u8; 20]>,
    /// Snapshot of the initial state.
    pub initial_state: StateSnapshot,
    /// All events in execution order.
    pub events: Vec<TraceEvent>,
    /// Metadata about the trace.
    pub metadata: TraceMetadata,
}

impl ExecutionTrace {
    /// Creates a new empty trace.
    pub fn new(seed: u64, initial_state: StateSnapshot) -> Self {
        Self {
            seed,
            commit_hash: None,
            initial_state,
            events: Vec::new(),
            metadata: TraceMetadata::default(),
        }
    }

    /// Creates a trace with commit hash for exact reproduction.
    pub fn with_commit(seed: u64, commit_hash: [u8; 20], initial_state: StateSnapshot) -> Self {
        Self {
            seed,
            commit_hash: Some(commit_hash),
            initial_state,
            events: Vec::new(),
            metadata: TraceMetadata::default(),
        }
    }

    /// Adds an event to the trace.
    pub fn push_event(&mut self, event: TraceEvent) {
        self.events.push(event);
    }

    /// Returns the number of events in the trace.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns true if the trace has no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns an iterator over events.
    pub fn iter(&self) -> impl Iterator<Item = &TraceEvent> {
        self.events.iter()
    }

    /// Returns events for a specific block height.
    pub fn events_for_block(&self, height: u64) -> impl Iterator<Item = &TraceEvent> {
        self.events.iter().filter(move |e| match e {
            TraceEvent::BlockStart { height: h, .. } => *h == height,
            TraceEvent::BlockEnd { height: h, .. } => *h == height,
            _ => false,
        })
    }

    /// Returns events for a specific transaction.
    pub fn events_for_tx(&self, tx_id: &[u8; 32]) -> impl Iterator<Item = &TraceEvent> {
        let tx_id = *tx_id;
        self.events.iter().filter(move |e| match e {
            TraceEvent::TxStart { tx_id: id, .. } => *id == tx_id,
            TraceEvent::TxEnd { tx_id: id, .. } => *id == tx_id,
            _ => false,
        })
    }

    /// Computes the state at a specific event index.
    pub fn state_at(&self, event_index: usize) -> StateSnapshot {
        let mut state = self.initial_state.clone();

        for event in self.events.iter().take(event_index + 1) {
            if let TraceEvent::StateChange { key, new_value, .. } = event {
                match new_value {
                    Some(value) => {
                        state.data.insert(key.clone(), value.clone());
                    }
                    None => {
                        state.data.remove(key);
                    }
                }
            }
        }

        state
    }

    /// Returns the final state after all events.
    pub fn final_state(&self) -> StateSnapshot {
        self.state_at(self.events.len().saturating_sub(1))
    }
}

/// Metadata about the trace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceMetadata {
    /// Total blocks traced.
    pub total_blocks: u64,
    /// Total transactions traced.
    pub total_txs: u64,
    /// Total state changes recorded.
    pub total_state_changes: u64,
    /// Total events emitted.
    pub total_events_emitted: u64,
    /// Trace start timestamp (wall clock).
    pub start_timestamp_ms: u64,
    /// Trace end timestamp (wall clock).
    pub end_timestamp_ms: u64,
    /// Size of the trace in bytes (when serialized).
    pub serialized_size_bytes: u64,
}

/// A snapshot of the complete state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// All key-value pairs in the state.
    pub data: HashMap<Vec<u8>, Vec<u8>>,
    /// Block height at this snapshot.
    pub height: u64,
    /// Timestamp at this snapshot.
    pub timestamp_ms: u64,
}

impl StateSnapshot {
    /// Creates an empty state snapshot.
    pub fn empty() -> Self {
        Self {
            data: HashMap::new(),
            height: 0,
            timestamp_ms: 0,
        }
    }

    /// Creates a snapshot from existing state data.
    pub fn from_data(data: HashMap<Vec<u8>, Vec<u8>>, height: u64, timestamp_ms: u64) -> Self {
        Self {
            data,
            height,
            timestamp_ms,
        }
    }

    /// Returns the number of keys in the snapshot.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the snapshot is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Gets a value from the snapshot.
    pub fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.data.get(key)
    }

    /// Computes a hash of the snapshot for comparison.
    pub fn hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();

        // Sort keys for deterministic ordering
        let mut keys: Vec<_> = self.data.keys().collect();
        keys.sort();

        for key in keys {
            if let Some(value) = self.data.get(key) {
                hasher.update(key);
                hasher.update(value);
            }
        }

        hasher.finalize().into()
    }
}

/// An event in the execution trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceEvent {
    /// Block execution started.
    BlockStart {
        height: u64,
        timestamp_ms: u64,
        event_index: usize,
    },

    /// Transaction execution started.
    TxStart {
        tx_id: [u8; 32],
        /// Sender account ID as bytes (u128 big-endian).
        sender: [u8; 16],
        /// Recipient account ID as bytes (u128 big-endian).
        recipient: [u8; 16],
        event_index: usize,
    },

    /// State was modified.
    StateChange {
        key: Vec<u8>,
        old_value: Option<Vec<u8>>,
        new_value: Option<Vec<u8>>,
        event_index: usize,
    },

    /// A call was made between accounts.
    Call {
        /// Caller account ID as bytes (u128 big-endian).
        from: [u8; 16],
        /// Callee account ID as bytes (u128 big-endian).
        to: [u8; 16],
        function_id: u64,
        data_hash: [u8; 32],
        event_index: usize,
    },

    /// A call returned.
    CallReturn {
        success: bool,
        data_hash: Option<[u8; 32]>,
        event_index: usize,
    },

    /// Gas was charged.
    GasCharge {
        amount: u64,
        remaining: u64,
        operation: String,
        event_index: usize,
    },

    /// An event was emitted.
    EventEmitted {
        name: String,
        data_hash: [u8; 32],
        event_index: usize,
    },

    /// Transaction execution ended.
    TxEnd {
        tx_id: [u8; 32],
        success: bool,
        gas_used: u64,
        event_index: usize,
    },

    /// Block execution ended.
    BlockEnd {
        height: u64,
        state_hash: [u8; 32],
        event_index: usize,
    },

    /// Checkpoint created.
    Checkpoint { id: u64, event_index: usize },

    /// Rollback to checkpoint.
    Rollback {
        to_checkpoint: u64,
        event_index: usize,
    },

    /// Error occurred.
    Error {
        code: u16,
        message: String,
        event_index: usize,
    },
}

impl TraceEvent {
    /// Returns the event index.
    pub fn event_index(&self) -> usize {
        match self {
            TraceEvent::BlockStart { event_index, .. } => *event_index,
            TraceEvent::TxStart { event_index, .. } => *event_index,
            TraceEvent::StateChange { event_index, .. } => *event_index,
            TraceEvent::Call { event_index, .. } => *event_index,
            TraceEvent::CallReturn { event_index, .. } => *event_index,
            TraceEvent::GasCharge { event_index, .. } => *event_index,
            TraceEvent::EventEmitted { event_index, .. } => *event_index,
            TraceEvent::TxEnd { event_index, .. } => *event_index,
            TraceEvent::BlockEnd { event_index, .. } => *event_index,
            TraceEvent::Checkpoint { event_index, .. } => *event_index,
            TraceEvent::Rollback { event_index, .. } => *event_index,
            TraceEvent::Error { event_index, .. } => *event_index,
        }
    }

    /// Returns true if this is a block boundary event.
    pub fn is_block_boundary(&self) -> bool {
        matches!(
            self,
            TraceEvent::BlockStart { .. } | TraceEvent::BlockEnd { .. }
        )
    }

    /// Returns true if this is a transaction boundary event.
    pub fn is_tx_boundary(&self) -> bool {
        matches!(self, TraceEvent::TxStart { .. } | TraceEvent::TxEnd { .. })
    }

    /// Returns true if this is a state change event.
    pub fn is_state_change(&self) -> bool {
        matches!(self, TraceEvent::StateChange { .. })
    }
}

/// Builder for constructing traces incrementally.
pub struct TraceBuilder {
    trace: ExecutionTrace,
    event_counter: usize,
}

impl TraceBuilder {
    /// Creates a new trace builder.
    pub fn new(seed: u64, initial_state: StateSnapshot) -> Self {
        Self {
            trace: ExecutionTrace::new(seed, initial_state),
            event_counter: 0,
        }
    }

    /// Records a block start.
    pub fn block_start(&mut self, height: u64, timestamp_ms: u64) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::BlockStart {
            height,
            timestamp_ms,
            event_index,
        });
        self.trace.metadata.total_blocks += 1;
    }

    /// Records a block end.
    pub fn block_end(&mut self, height: u64, state_hash: [u8; 32]) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::BlockEnd {
            height,
            state_hash,
            event_index,
        });
    }

    /// Records a transaction start.
    pub fn tx_start(&mut self, tx_id: [u8; 32], sender: AccountId, recipient: AccountId) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::TxStart {
            tx_id,
            sender: account_to_bytes(sender),
            recipient: account_to_bytes(recipient),
            event_index,
        });
        self.trace.metadata.total_txs += 1;
    }

    /// Records a transaction end.
    pub fn tx_end(&mut self, tx_id: [u8; 32], success: bool, gas_used: u64) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::TxEnd {
            tx_id,
            success,
            gas_used,
            event_index,
        });
    }

    /// Records a state change.
    pub fn state_change(
        &mut self,
        key: Vec<u8>,
        old_value: Option<Vec<u8>>,
        new_value: Option<Vec<u8>>,
    ) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::StateChange {
            key,
            old_value,
            new_value,
            event_index,
        });
        self.trace.metadata.total_state_changes += 1;
    }

    /// Records a call.
    pub fn call(&mut self, from: AccountId, to: AccountId, function_id: u64, data_hash: [u8; 32]) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::Call {
            from: account_to_bytes(from),
            to: account_to_bytes(to),
            function_id,
            data_hash,
            event_index,
        });
    }

    /// Records a call return.
    pub fn call_return(&mut self, success: bool, data_hash: Option<[u8; 32]>) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::CallReturn {
            success,
            data_hash,
            event_index,
        });
    }

    /// Records a gas charge.
    pub fn gas_charge(&mut self, amount: u64, remaining: u64, operation: impl Into<String>) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::GasCharge {
            amount,
            remaining,
            operation: operation.into(),
            event_index,
        });
    }

    /// Records an emitted event.
    pub fn event_emitted(&mut self, name: impl Into<String>, data_hash: [u8; 32]) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::EventEmitted {
            name: name.into(),
            data_hash,
            event_index,
        });
        self.trace.metadata.total_events_emitted += 1;
    }

    /// Records an error.
    pub fn error(&mut self, code: u16, message: impl Into<String>) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::Error {
            code,
            message: message.into(),
            event_index,
        });
    }

    /// Records a checkpoint.
    pub fn checkpoint(&mut self, id: u64) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace
            .push_event(TraceEvent::Checkpoint { id, event_index });
    }

    /// Records a rollback.
    pub fn rollback(&mut self, to_checkpoint: u64) {
        let event_index = self.event_counter;
        self.event_counter += 1;
        self.trace.push_event(TraceEvent::Rollback {
            to_checkpoint,
            event_index,
        });
    }

    /// Finishes building and returns the trace.
    pub fn finish(self) -> ExecutionTrace {
        self.trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_trace() {
        let trace = ExecutionTrace::new(42, StateSnapshot::empty());
        assert!(trace.is_empty());
        assert_eq!(trace.seed, 42);
    }

    #[test]
    fn test_trace_builder() {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());

        builder.block_start(0, 1000);
        builder.tx_start([1; 32], AccountId::new(1), AccountId::new(2));
        builder.state_change(b"key".to_vec(), None, Some(b"value".to_vec()));
        builder.tx_end([1; 32], true, 100);
        builder.block_end(0, [0; 32]);

        let trace = builder.finish();

        assert_eq!(trace.len(), 5);
        assert_eq!(trace.metadata.total_blocks, 1);
        assert_eq!(trace.metadata.total_txs, 1);
        assert_eq!(trace.metadata.total_state_changes, 1);
    }

    #[test]
    fn test_state_at() {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());

        builder.state_change(b"key1".to_vec(), None, Some(b"value1".to_vec()));
        builder.state_change(b"key2".to_vec(), None, Some(b"value2".to_vec()));
        builder.state_change(
            b"key1".to_vec(),
            Some(b"value1".to_vec()),
            Some(b"updated".to_vec()),
        );

        let trace = builder.finish();

        // State at index 0: key1 = value1
        let state0 = trace.state_at(0);
        assert_eq!(state0.get(b"key1"), Some(&b"value1".to_vec()));
        assert_eq!(state0.get(b"key2"), None);

        // State at index 1: key1 = value1, key2 = value2
        let state1 = trace.state_at(1);
        assert_eq!(state1.get(b"key1"), Some(&b"value1".to_vec()));
        assert_eq!(state1.get(b"key2"), Some(&b"value2".to_vec()));

        // State at index 2: key1 = updated, key2 = value2
        let state2 = trace.state_at(2);
        assert_eq!(state2.get(b"key1"), Some(&b"updated".to_vec()));
        assert_eq!(state2.get(b"key2"), Some(&b"value2".to_vec()));
    }

    #[test]
    fn test_state_snapshot_hash() {
        let mut state1 = StateSnapshot::empty();
        state1.data.insert(b"a".to_vec(), b"1".to_vec());
        state1.data.insert(b"b".to_vec(), b"2".to_vec());

        let mut state2 = StateSnapshot::empty();
        state2.data.insert(b"b".to_vec(), b"2".to_vec());
        state2.data.insert(b"a".to_vec(), b"1".to_vec());

        // Same data should produce same hash regardless of insertion order
        assert_eq!(state1.hash(), state2.hash());

        // Different data should produce different hash
        let mut state3 = StateSnapshot::empty();
        state3.data.insert(b"a".to_vec(), b"different".to_vec());
        assert_ne!(state1.hash(), state3.hash());
    }
}
