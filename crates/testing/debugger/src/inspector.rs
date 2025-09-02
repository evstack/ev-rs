//! State inspection utilities for debugging.
//!
//! Provides tools for examining state at specific points in the trace.

use crate::trace::{ExecutionTrace, StateSnapshot, TraceEvent};
use evolve_core::AccountId;
use std::collections::HashMap;

/// Convert AccountId to serializable bytes.
fn account_to_bytes(id: AccountId) -> [u8; 16] {
    let bytes = id.as_bytes();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[..16]);
    arr
}

/// Inspector for examining trace and state.
pub struct StateInspector<'a> {
    trace: &'a ExecutionTrace,
    position: usize,
}

impl<'a> StateInspector<'a> {
    /// Creates a new inspector at a specific position.
    pub fn new(trace: &'a ExecutionTrace, position: usize) -> Self {
        Self { trace, position }
    }

    /// Creates an inspector at the beginning of the trace.
    pub fn at_start(trace: &'a ExecutionTrace) -> Self {
        Self::new(trace, 0)
    }

    /// Creates an inspector at the end of the trace.
    pub fn at_end(trace: &'a ExecutionTrace) -> Self {
        Self::new(trace, trace.len().saturating_sub(1))
    }

    /// Returns the state at the current position.
    pub fn state(&self) -> StateSnapshot {
        self.trace.state_at(self.position)
    }

    /// Returns the current event.
    pub fn event(&self) -> Option<&TraceEvent> {
        self.trace.events.get(self.position)
    }

    /// Gets a value from the state at current position.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.state().get(key).cloned()
    }

    /// Returns all keys in the state at current position.
    pub fn keys(&self) -> Vec<Vec<u8>> {
        self.state().data.keys().cloned().collect()
    }

    /// Returns all keys matching a prefix.
    pub fn keys_with_prefix(&self, prefix: &[u8]) -> Vec<Vec<u8>> {
        self.state()
            .data
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Searches for keys by pattern (simple substring match).
    pub fn search_keys(&self, pattern: &[u8]) -> Vec<Vec<u8>> {
        self.state()
            .data
            .keys()
            .filter(|k| contains_subsequence(k, pattern))
            .cloned()
            .collect()
    }

    /// Compares state at current position with state at another position.
    pub fn diff_with(&self, other_position: usize) -> StateDiff {
        let state1 = self.state();
        let state2 = self.trace.state_at(other_position);

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();

        // Find added and changed keys
        for (key, new_value) in &state2.data {
            match state1.data.get(key) {
                None => added.push(KeyChange {
                    key: key.clone(),
                    old_value: None,
                    new_value: Some(new_value.clone()),
                }),
                Some(old_value) if old_value != new_value => changed.push(KeyChange {
                    key: key.clone(),
                    old_value: Some(old_value.clone()),
                    new_value: Some(new_value.clone()),
                }),
                _ => {}
            }
        }

        // Find removed keys
        for (key, old_value) in &state1.data {
            if !state2.data.contains_key(key) {
                removed.push(KeyChange {
                    key: key.clone(),
                    old_value: Some(old_value.clone()),
                    new_value: None,
                });
            }
        }

        StateDiff {
            from_position: self.position,
            to_position: other_position,
            added,
            removed,
            changed,
        }
    }

    /// Returns a summary of the state at current position.
    pub fn summary(&self) -> StateSummary {
        let state = self.state();
        let total_keys = state.len();
        let total_bytes: usize = state.data.iter().map(|(k, v)| k.len() + v.len()).sum();

        // Analyze key prefixes
        let mut prefix_counts: HashMap<String, usize> = HashMap::new();
        for key in state.data.keys() {
            let prefix = extract_prefix(key);
            *prefix_counts.entry(prefix).or_insert(0) += 1;
        }

        StateSummary {
            position: self.position,
            total_keys,
            total_bytes,
            prefix_counts,
        }
    }
}

/// A difference between two states.
#[derive(Debug, Clone)]
pub struct StateDiff {
    /// Position of the first state.
    pub from_position: usize,
    /// Position of the second state.
    pub to_position: usize,
    /// Keys that were added.
    pub added: Vec<KeyChange>,
    /// Keys that were removed.
    pub removed: Vec<KeyChange>,
    /// Keys that were changed.
    pub changed: Vec<KeyChange>,
}

impl StateDiff {
    /// Returns true if there are no differences.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Returns the total number of changes.
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }

    /// Formats the diff as a human-readable string.
    pub fn to_string_pretty(&self) -> String {
        let mut output = format!(
            "State diff: {} -> {}\n",
            self.from_position, self.to_position
        );

        if !self.added.is_empty() {
            output.push_str(&format!("\nAdded ({}):\n", self.added.len()));
            for change in &self.added {
                output.push_str(&format!("  + {}\n", format_key(&change.key)));
            }
        }

        if !self.removed.is_empty() {
            output.push_str(&format!("\nRemoved ({}):\n", self.removed.len()));
            for change in &self.removed {
                output.push_str(&format!("  - {}\n", format_key(&change.key)));
            }
        }

        if !self.changed.is_empty() {
            output.push_str(&format!("\nChanged ({}):\n", self.changed.len()));
            for change in &self.changed {
                output.push_str(&format!("  ~ {}\n", format_key(&change.key)));
            }
        }

        output
    }
}

/// A change to a specific key.
#[derive(Debug, Clone)]
pub struct KeyChange {
    /// The key that changed.
    pub key: Vec<u8>,
    /// Old value (None if added).
    pub old_value: Option<Vec<u8>>,
    /// New value (None if removed).
    pub new_value: Option<Vec<u8>>,
}

/// Summary statistics about state.
#[derive(Debug, Clone)]
pub struct StateSummary {
    /// Position in trace.
    pub position: usize,
    /// Total number of keys.
    pub total_keys: usize,
    /// Total size in bytes.
    pub total_bytes: usize,
    /// Count of keys by prefix.
    pub prefix_counts: HashMap<String, usize>,
}

impl StateSummary {
    /// Formats the summary as a human-readable string.
    pub fn to_string_pretty(&self) -> String {
        let mut output = format!(
            "State Summary at position {}:\n\
             - Total keys: {}\n\
             - Total bytes: {}\n",
            self.position, self.total_keys, self.total_bytes
        );

        if !self.prefix_counts.is_empty() {
            output.push_str("\nKeys by prefix:\n");
            let mut prefixes: Vec<_> = self.prefix_counts.iter().collect();
            prefixes.sort_by(|a, b| b.1.cmp(a.1));

            for (prefix, count) in prefixes.iter().take(10) {
                output.push_str(&format!("  {}: {}\n", prefix, count));
            }

            if self.prefix_counts.len() > 10 {
                output.push_str(&format!(
                    "  ... and {} more prefixes\n",
                    self.prefix_counts.len() - 10
                ));
            }
        }

        output
    }
}

/// Query builder for searching traces.
pub struct TraceQuery<'a> {
    trace: &'a ExecutionTrace,
    filters: Vec<Box<dyn Fn(&TraceEvent) -> bool + 'a>>,
}

impl<'a> TraceQuery<'a> {
    /// Creates a new query over a trace.
    pub fn new(trace: &'a ExecutionTrace) -> Self {
        Self {
            trace,
            filters: Vec::new(),
        }
    }

    /// Filters to only block events.
    pub fn blocks(mut self) -> Self {
        self.filters.push(Box::new(|e| e.is_block_boundary()));
        self
    }

    /// Filters to only transaction events.
    pub fn transactions(mut self) -> Self {
        self.filters.push(Box::new(|e| e.is_tx_boundary()));
        self
    }

    /// Filters to only state changes.
    pub fn state_changes(mut self) -> Self {
        self.filters.push(Box::new(|e| e.is_state_change()));
        self
    }

    /// Filters to events involving an account.
    pub fn involving(mut self, account: AccountId) -> Self {
        let account_bytes = account_to_bytes(account);
        self.filters.push(Box::new(move |e| match e {
            TraceEvent::TxStart {
                sender, recipient, ..
            } => *sender == account_bytes || *recipient == account_bytes,
            TraceEvent::Call { from, to, .. } => *from == account_bytes || *to == account_bytes,
            _ => false,
        }));
        self
    }

    /// Filters to state changes on keys with a prefix.
    pub fn key_prefix(mut self, prefix: Vec<u8>) -> Self {
        self.filters.push(Box::new(
            move |e| matches!(e, TraceEvent::StateChange { key, .. } if key.starts_with(&prefix)),
        ));
        self
    }

    /// Filters to only errors.
    pub fn errors(mut self) -> Self {
        self.filters
            .push(Box::new(|e| matches!(e, TraceEvent::Error { .. })));
        self
    }

    /// Executes the query and returns matching events with their indices.
    pub fn execute(self) -> Vec<(usize, &'a TraceEvent)> {
        self.trace
            .events
            .iter()
            .enumerate()
            .filter(|(_, event)| self.filters.iter().all(|f| f(event)))
            .collect()
    }

    /// Returns the count of matching events.
    pub fn count(self) -> usize {
        self.execute().len()
    }
}

// Helper functions

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn extract_prefix(key: &[u8]) -> String {
    // Try to find a common separator (: or /)
    for (i, &byte) in key.iter().enumerate() {
        if byte == b':' || byte == b'/' {
            if let Ok(s) = std::str::from_utf8(&key[..=i]) {
                return s.to_string();
            }
        }
    }

    // Fall back to first 8 bytes or the whole key
    let prefix_len = key.len().min(8);
    if let Ok(s) = std::str::from_utf8(&key[..prefix_len]) {
        s.to_string()
    } else {
        format!("{:02x?}", &key[..prefix_len])
    }
}

fn format_key(key: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(key) {
        format!("\"{}\"", s)
    } else {
        format!("{:02x?}", key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::TraceBuilder;

    fn make_test_trace() -> ExecutionTrace {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());

        builder.block_start(0, 1000);
        builder.tx_start([1; 32], AccountId::new(1), AccountId::new(2));
        builder.state_change(b"balance:user1".to_vec(), None, Some(b"100".to_vec()));
        builder.state_change(b"balance:user2".to_vec(), None, Some(b"200".to_vec()));
        builder.tx_end([1; 32], true, 100);
        builder.block_end(0, [0; 32]);

        builder.block_start(1, 2000);
        builder.tx_start([2; 32], AccountId::new(2), AccountId::new(3));
        builder.state_change(
            b"balance:user1".to_vec(),
            Some(b"100".to_vec()),
            Some(b"50".to_vec()),
        );
        builder.error(1, "test error");
        builder.tx_end([2; 32], false, 50);
        builder.block_end(1, [1; 32]);

        builder.finish()
    }

    #[test]
    fn test_inspector_state() {
        let trace = make_test_trace();
        let inspector = StateInspector::new(&trace, 4);

        let state = inspector.state();
        assert!(state.get(b"balance:user1").is_some());
        assert!(state.get(b"balance:user2").is_some());
    }

    #[test]
    fn test_inspector_keys_with_prefix() {
        let trace = make_test_trace();
        let inspector = StateInspector::at_end(&trace);

        let keys = inspector.keys_with_prefix(b"balance:");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_state_diff() {
        let trace = make_test_trace();
        let inspector = StateInspector::new(&trace, 4);

        // Compare state at position 4 with state at position 8
        let diff = inspector.diff_with(8);

        // User1's balance should have changed
        assert!(diff.changed.iter().any(|c| c.key == b"balance:user1"));
    }

    #[test]
    fn test_state_summary() {
        let trace = make_test_trace();
        let inspector = StateInspector::at_end(&trace);

        let summary = inspector.summary();
        assert_eq!(summary.total_keys, 2);
        assert!(summary.total_bytes > 0);
    }

    #[test]
    fn test_trace_query_blocks() {
        let trace = make_test_trace();
        let results = TraceQuery::new(&trace).blocks().execute();

        // Should have 4 block events (2 starts + 2 ends)
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_trace_query_state_changes() {
        let trace = make_test_trace();
        let results = TraceQuery::new(&trace).state_changes().execute();

        // Should have 3 state changes
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_trace_query_errors() {
        let trace = make_test_trace();
        let results = TraceQuery::new(&trace).errors().execute();

        // Should have 1 error
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_trace_query_key_prefix() {
        let trace = make_test_trace();
        let results = TraceQuery::new(&trace)
            .key_prefix(b"balance:".to_vec())
            .execute();

        // All state changes are on balance: keys
        assert_eq!(results.len(), 3);
    }
}
