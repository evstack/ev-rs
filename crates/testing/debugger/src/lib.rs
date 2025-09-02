//! # Evolve Debugger
//!
//! Time-travel debugging for Evolve SDK simulations.
//!
//! ## Features
//!
//! - **Trace Recording**: Capture detailed execution traces including all state changes
//! - **Binary/JSON Formats**: Store traces in compact binary or readable JSON format
//! - **Deterministic Replay**: Step through execution forward and backward
//! - **Breakpoints**: Pause at specific blocks, transactions, accounts, or conditions
//! - **State Inspection**: Examine and compare state at any point in execution
//!
//! ## Example
//!
//! ```ignore
//! use evolve_debugger::{TraceBuilder, Replayer, Breakpoint, StateSnapshot};
//!
//! // Build a trace during simulation
//! let mut builder = TraceBuilder::new(seed, StateSnapshot::empty());
//! builder.block_start(0, timestamp);
//! builder.tx_start(tx_id, sender, recipient);
//! builder.state_change(key, old_value, new_value);
//! builder.tx_end(tx_id, success, gas_used);
//! builder.block_end(0, state_hash);
//!
//! let trace = builder.finish();
//!
//! // Save trace to file
//! save_trace(&trace, Path::new("debug.trace"), TraceFormat::BinaryCompressed)?;
//!
//! // Later, replay for debugging
//! let trace = load_trace(Path::new("debug.trace"))?;
//! let mut replayer = Replayer::new(trace);
//!
//! // Add breakpoint on errors
//! replayer.add_breakpoint(Breakpoint::on_error());
//!
//! // Run until breakpoint
//! replayer.run_to_breakpoint();
//!
//! // Inspect state
//! let value = replayer.get_storage(b"my_key");
//! ```

pub mod breakpoints;
pub mod format;
pub mod inspector;
pub mod replay;
pub mod trace;

pub use breakpoints::{Breakpoint, BreakpointBuilder};
pub use format::{
    deserialize_trace, detect_format_from_extension, format_extension, load_trace, save_trace,
    serialize_trace, FormatError, TraceFormat,
};
pub use inspector::{KeyChange, StateDiff, StateInspector, StateSummary, TraceQuery};
pub use replay::{AccountInfo, Replayer, StepResult};
pub use trace::{ExecutionTrace, StateSnapshot, TraceBuilder, TraceEvent, TraceMetadata};

/// Re-export evolve_simulator for integration.
pub use evolve_simulator;

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::AccountId;
    use std::path::Path;
    use tempfile::TempDir;

    fn make_test_trace() -> ExecutionTrace {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());

        builder.block_start(0, 1000);
        builder.tx_start([1; 32], AccountId::new(1), AccountId::new(2));
        builder.state_change(b"key".to_vec(), None, Some(b"value".to_vec()));
        builder.tx_end([1; 32], true, 100);
        builder.block_end(0, [0; 32]);

        builder.finish()
    }

    #[test]
    fn test_full_workflow() {
        // Build trace
        let trace = make_test_trace();
        assert_eq!(trace.len(), 5);

        // Save and load
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.trace.gz");

        save_trace(&trace, &path, TraceFormat::BinaryCompressed).unwrap();
        let loaded = load_trace(&path).unwrap();

        assert_eq!(trace.seed, loaded.seed);
        assert_eq!(trace.len(), loaded.len());
    }

    #[test]
    fn test_replay_and_inspect() {
        let trace = make_test_trace();
        let mut replayer = Replayer::new(trace);

        // Step through
        replayer.step();
        replayer.step();
        replayer.step();

        // Check state after state change
        assert_eq!(replayer.get_storage(b"key"), Some(&b"value".to_vec()));

        // Step back
        replayer.step_back();
        replayer.step_back();

        // State should be restored
        assert_eq!(replayer.get_storage(b"key"), None);
    }

    #[test]
    fn test_breakpoint_on_error() {
        let mut builder = TraceBuilder::new(42, StateSnapshot::empty());
        builder.block_start(0, 1000);
        builder.tx_start([1; 32], AccountId::new(1), AccountId::new(2));
        builder.error(1, "test error");
        builder.tx_end([1; 32], false, 50);
        builder.block_end(0, [0; 32]);

        let trace = builder.finish();
        let mut replayer = Replayer::new(trace);

        replayer.add_breakpoint(Breakpoint::on_error());

        let result = replayer.run_to_breakpoint();
        assert!(matches!(result, StepResult::HitBreakpoint(_)));
    }

    #[test]
    fn test_state_inspector() {
        let trace = make_test_trace();
        let inspector = StateInspector::at_end(&trace);

        let summary = inspector.summary();
        assert_eq!(summary.total_keys, 1);
    }

    #[test]
    fn test_trace_query() {
        let trace = make_test_trace();

        let blocks = TraceQuery::new(&trace).blocks().count();
        assert_eq!(blocks, 2); // start + end

        let state_changes = TraceQuery::new(&trace).state_changes().count();
        assert_eq!(state_changes, 1);
    }

    #[test]
    fn test_format_detection() {
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace")),
            Some(TraceFormat::Binary)
        );
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace.gz")),
            Some(TraceFormat::BinaryCompressed)
        );
        assert_eq!(
            detect_format_from_extension(Path::new("test.trace.json")),
            Some(TraceFormat::Json)
        );
    }
}
