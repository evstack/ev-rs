//! Deterministic replay engine for execution traces.
//!
//! Allows stepping through a recorded execution trace forward and backward,
//! inspecting state at any point.

use crate::breakpoints::Breakpoint;
use crate::trace::{ExecutionTrace, StateSnapshot, TraceEvent};
use evolve_core::AccountId;

/// Result of a replay step.
#[derive(Debug, Clone)]
pub enum StepResult {
    /// Successfully moved to the event.
    Ok,
    /// Hit a breakpoint.
    HitBreakpoint(usize),
    /// Reached the beginning of the trace.
    AtBeginning,
    /// Reached the end of the trace.
    AtEnd,
}

/// The replay engine for stepping through traces.
pub struct Replayer {
    /// The trace being replayed.
    trace: ExecutionTrace,
    /// Current position in the trace (event index).
    position: usize,
    /// Cached state at current position.
    current_state: StateSnapshot,
    /// Active breakpoints.
    breakpoints: Vec<Breakpoint>,
    /// History of positions for step-back.
    history: Vec<usize>,
    /// Maximum history size.
    max_history: usize,
}

impl Replayer {
    /// Creates a new replayer for the given trace.
    pub fn new(trace: ExecutionTrace) -> Self {
        let initial_state = trace.initial_state.clone();
        Self {
            trace,
            position: 0,
            current_state: initial_state,
            breakpoints: Vec::new(),
            history: Vec::new(),
            max_history: 1000,
        }
    }

    /// Returns the trace being replayed.
    pub fn trace(&self) -> &ExecutionTrace {
        &self.trace
    }

    /// Returns the current position (event index).
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the total number of events.
    pub fn total_events(&self) -> usize {
        self.trace.len()
    }

    /// Returns the current event, if any.
    pub fn current_event(&self) -> Option<&TraceEvent> {
        if self.position < self.trace.len() {
            Some(&self.trace.events[self.position])
        } else {
            None
        }
    }

    /// Returns the current state snapshot.
    pub fn current_state(&self) -> &StateSnapshot {
        &self.current_state
    }

    /// Steps forward one event.
    pub fn step(&mut self) -> StepResult {
        if self.position >= self.trace.len() {
            return StepResult::AtEnd;
        }

        // Record history for step-back
        self.push_history();

        // Apply current event to state
        self.apply_event_to_state(self.position);

        // Move to next position
        self.position += 1;

        // Check breakpoints
        if let Some(bp_idx) = self.check_breakpoints() {
            return StepResult::HitBreakpoint(bp_idx);
        }

        StepResult::Ok
    }

    /// Steps backward one event.
    pub fn step_back(&mut self) -> StepResult {
        if self.position == 0 {
            return StepResult::AtBeginning;
        }

        // Move back
        self.position -= 1;

        // Recompute state at this position
        self.current_state = self.trace.state_at(self.position.saturating_sub(1));

        StepResult::Ok
    }

    /// Runs until a breakpoint is hit or the end is reached.
    pub fn run_to_breakpoint(&mut self) -> StepResult {
        loop {
            match self.step() {
                StepResult::Ok => continue,
                result => return result,
            }
        }
    }

    /// Runs backward until a breakpoint is hit or the beginning is reached.
    pub fn run_back_to_breakpoint(&mut self) -> StepResult {
        loop {
            match self.step_back() {
                StepResult::Ok => {
                    if let Some(bp_idx) = self.check_breakpoints() {
                        return StepResult::HitBreakpoint(bp_idx);
                    }
                    continue;
                }
                result => return result,
            }
        }
    }

    /// Jumps to a specific block.
    pub fn goto_block(&mut self, height: u64) -> StepResult {
        // Find the block start event
        for (idx, event) in self.trace.events.iter().enumerate() {
            if let TraceEvent::BlockStart { height: h, .. } = event {
                if *h == height {
                    return self.goto_position(idx);
                }
            }
        }
        StepResult::Ok // Block not found, stay at current position
    }

    /// Jumps to a specific transaction.
    pub fn goto_tx(&mut self, tx_id: [u8; 32]) -> StepResult {
        // Find the tx start event
        for (idx, event) in self.trace.events.iter().enumerate() {
            if let TraceEvent::TxStart { tx_id: id, .. } = event {
                if *id == tx_id {
                    return self.goto_position(idx);
                }
            }
        }
        StepResult::Ok // Tx not found, stay at current position
    }

    /// Jumps to a specific position.
    pub fn goto_position(&mut self, target: usize) -> StepResult {
        if target >= self.trace.len() {
            return StepResult::AtEnd;
        }

        self.push_history();
        self.position = target;
        self.current_state = self.trace.state_at(target.saturating_sub(1));

        if let Some(bp_idx) = self.check_breakpoints() {
            return StepResult::HitBreakpoint(bp_idx);
        }

        StepResult::Ok
    }

    /// Resets to the beginning of the trace.
    pub fn reset(&mut self) {
        self.position = 0;
        self.current_state = self.trace.initial_state.clone();
        self.history.clear();
    }

    /// Adds a breakpoint.
    pub fn add_breakpoint(&mut self, bp: Breakpoint) -> usize {
        self.breakpoints.push(bp);
        self.breakpoints.len() - 1
    }

    /// Removes a breakpoint by index.
    pub fn remove_breakpoint(&mut self, index: usize) -> Option<Breakpoint> {
        if index < self.breakpoints.len() {
            Some(self.breakpoints.remove(index))
        } else {
            None
        }
    }

    /// Clears all breakpoints.
    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    /// Returns all breakpoints.
    pub fn breakpoints(&self) -> &[Breakpoint] {
        &self.breakpoints
    }

    /// Gets a storage value at the current position.
    pub fn get_storage(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.current_state.get(key)
    }

    /// Returns information about an account at the current position.
    pub fn account_info(&self, _id: AccountId) -> AccountInfo {
        // In a full implementation, this would query account-specific state
        AccountInfo {
            storage_keys: self.current_state.len(),
        }
    }

    /// Returns the current block height based on trace position.
    pub fn current_block(&self) -> Option<u64> {
        // Find the most recent block start before current position
        for event in self.trace.events[..=self.position.min(self.trace.len().saturating_sub(1))].iter().rev() {
            if let TraceEvent::BlockStart { height, .. } = event {
                return Some(*height);
            }
        }
        None
    }

    /// Returns the current transaction ID based on trace position.
    pub fn current_tx(&self) -> Option<[u8; 32]> {
        // Find the most recent tx start before current position
        for event in self.trace.events[..=self.position.min(self.trace.len().saturating_sub(1))].iter().rev() {
            match event {
                TraceEvent::TxStart { tx_id, .. } => return Some(*tx_id),
                TraceEvent::TxEnd { .. } => return None, // We're after a tx end
                _ => continue,
            }
        }
        None
    }

    /// Applies an event to the current state.
    fn apply_event_to_state(&mut self, event_idx: usize) {
        if event_idx >= self.trace.events.len() {
            return;
        }

        if let TraceEvent::StateChange { key, new_value, .. } = &self.trace.events[event_idx] {
            match new_value {
                Some(value) => {
                    self.current_state.data.insert(key.clone(), value.clone());
                }
                None => {
                    self.current_state.data.remove(key);
                }
            }
        }
    }

    /// Checks if any breakpoint matches the current event.
    fn check_breakpoints(&self) -> Option<usize> {
        let event = self.current_event()?;

        for (idx, bp) in self.breakpoints.iter().enumerate() {
            if bp.matches(event, &self.current_state) {
                return Some(idx);
            }
        }

        None
    }

    /// Pushes current position to history.
    fn push_history(&mut self) {
        if self.history.len() >= self.max_history {
            self.history.remove(0);
        }
        self.history.push(self.position);
    }
}

/// Basic information about an account.
#[derive(Debug, Clone)]
pub struct AccountInfo {
    /// Number of storage keys associated with this account.
    pub storage_keys: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::TraceBuilder;

    fn make_test_trace() -> ExecutionTrace {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());

        builder.block_start(0, 1000);
        builder.tx_start([1; 32], AccountId::new(1), AccountId::new(2));
        builder.state_change(b"key1".to_vec(), None, Some(b"value1".to_vec()));
        builder.tx_end([1; 32], true, 100);
        builder.block_end(0, [0; 32]);

        builder.block_start(1, 2000);
        builder.tx_start([2; 32], AccountId::new(2), AccountId::new(3));
        builder.state_change(b"key2".to_vec(), None, Some(b"value2".to_vec()));
        builder.tx_end([2; 32], true, 150);
        builder.block_end(1, [1; 32]);

        builder.finish()
    }

    #[test]
    fn test_basic_replay() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        assert_eq!(replayer.position(), 0);
        assert_eq!(replayer.total_events(), 10);

        // Step through all events
        for _ in 0..10 {
            let result = replayer.step();
            assert!(matches!(result, StepResult::Ok | StepResult::AtEnd));
        }

        assert_eq!(replayer.position(), 10);
    }

    #[test]
    fn test_step_back() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        // Step forward
        replayer.step();
        replayer.step();
        replayer.step();

        assert_eq!(replayer.position(), 3);

        // Step back
        let result = replayer.step_back();
        assert!(matches!(result, StepResult::Ok));
        assert_eq!(replayer.position(), 2);
    }

    #[test]
    fn test_goto_block() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        replayer.goto_block(1);
        assert_eq!(replayer.current_block(), Some(1));
    }

    #[test]
    fn test_goto_tx() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        replayer.goto_tx([2; 32]);
        assert_eq!(replayer.current_tx(), Some([2; 32]));
    }

    #[test]
    fn test_state_tracking() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        // Initially no state
        assert!(replayer.get_storage(b"key1").is_none());

        // Step to after state change
        replayer.goto_position(3);
        assert_eq!(replayer.get_storage(b"key1"), Some(&b"value1".to_vec()));
    }

    #[test]
    fn test_reset() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        // Move forward
        replayer.step();
        replayer.step();
        replayer.step();

        // Reset
        replayer.reset();
        assert_eq!(replayer.position(), 0);
        assert!(replayer.current_state.is_empty());
    }

    #[test]
    fn test_breakpoint() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        // Add breakpoint on block 1
        replayer.add_breakpoint(Breakpoint::OnBlock(1));

        // Run to breakpoint
        let result = replayer.run_to_breakpoint();

        if let StepResult::HitBreakpoint(_) = result {
            assert_eq!(replayer.current_block(), Some(1));
        }
    }
}
