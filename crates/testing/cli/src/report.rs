//! Report generation command.

use evolve_debugger::{load_trace, StateInspector};
use std::path::PathBuf;

/// Configuration for the report command.
pub struct ReportConfig {
    pub trace_path: Option<PathBuf>,
    pub output_format: String,
    pub output_path: Option<PathBuf>,
    pub per_block: bool,
}

/// Execute the report command.
pub fn execute(config: ReportConfig) -> Result<(), String> {
    let trace_path = config
        .trace_path
        .ok_or("Trace path is required for report generation")?;

    // Load trace
    let trace = load_trace(&trace_path).map_err(|e| format!("Failed to load trace: {e}"))?;

    // Generate report data
    let report = generate_report_data(&trace, config.per_block);

    // Format output
    let output = match config.output_format.as_str() {
        "json" => serde_json::to_string_pretty(&report)
            .map_err(|e| format!("Failed to serialize report: {e}"))?,
        "csv" => format_csv(&report),
        _ => format_text(&report),
    };

    // Write output
    match config.output_path {
        Some(path) => {
            std::fs::write(&path, output).map_err(|e| format!("Failed to write output: {e}"))?;
            println!("Report written to: {}", path.display());
        }
        None => {
            print!("{}", output);
        }
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct ReportData {
    seed: u64,
    summary: TraceSummary,
    events: EventBreakdown,
    state: StateSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_block: Option<Vec<BlockSummary>>,
}

#[derive(serde::Serialize)]
struct TraceSummary {
    total_events: usize,
    total_blocks: u64,
    total_txs: u64,
    total_state_changes: u64,
    total_errors: usize,
}

#[derive(serde::Serialize)]
struct EventBreakdown {
    block_events: usize,
    tx_events: usize,
    state_changes: usize,
    calls: usize,
    gas_charges: usize,
    events_emitted: usize,
    errors: usize,
}

#[derive(serde::Serialize)]
struct StateSummary {
    final_key_count: usize,
    final_size_bytes: usize,
    top_prefixes: Vec<(String, usize)>,
}

#[derive(serde::Serialize)]
struct BlockSummary {
    height: u64,
    tx_count: usize,
    state_changes: usize,
    errors: usize,
}

fn generate_report_data(
    trace: &evolve_debugger::ExecutionTrace,
    include_per_block: bool,
) -> ReportData {
    // Count event types
    let mut block_events = 0;
    let mut tx_events = 0;
    let mut state_changes = 0;
    let mut calls = 0;
    let mut gas_charges = 0;
    let mut events_emitted = 0;
    let mut errors = 0;

    for event in trace.iter() {
        match event {
            evolve_debugger::TraceEvent::BlockStart { .. }
            | evolve_debugger::TraceEvent::BlockEnd { .. } => block_events += 1,
            evolve_debugger::TraceEvent::TxStart { .. }
            | evolve_debugger::TraceEvent::TxEnd { .. } => tx_events += 1,
            evolve_debugger::TraceEvent::StateChange { .. } => state_changes += 1,
            evolve_debugger::TraceEvent::Call { .. }
            | evolve_debugger::TraceEvent::CallReturn { .. } => calls += 1,
            evolve_debugger::TraceEvent::GasCharge { .. } => gas_charges += 1,
            evolve_debugger::TraceEvent::EventEmitted { .. } => events_emitted += 1,
            evolve_debugger::TraceEvent::Error { .. } => errors += 1,
            _ => {}
        }
    }

    // State summary
    let inspector = StateInspector::at_end(trace);
    let state_summary = inspector.summary();

    let mut top_prefixes: Vec<_> = state_summary.prefix_counts.into_iter().collect();
    top_prefixes.sort_by(|a, b| b.1.cmp(&a.1));
    top_prefixes.truncate(10);

    // Per-block breakdown if requested
    let per_block = if include_per_block {
        Some(generate_per_block_summary(trace))
    } else {
        None
    };

    ReportData {
        seed: trace.seed,
        summary: TraceSummary {
            total_events: trace.len(),
            total_blocks: trace.metadata.total_blocks,
            total_txs: trace.metadata.total_txs,
            total_state_changes: trace.metadata.total_state_changes,
            total_errors: errors,
        },
        events: EventBreakdown {
            block_events,
            tx_events,
            state_changes,
            calls,
            gas_charges,
            events_emitted,
            errors,
        },
        state: StateSummary {
            final_key_count: state_summary.total_keys,
            final_size_bytes: state_summary.total_bytes,
            top_prefixes,
        },
        per_block,
    }
}

fn generate_per_block_summary(trace: &evolve_debugger::ExecutionTrace) -> Vec<BlockSummary> {
    let mut summaries = Vec::new();
    let mut tx_count = 0;
    let mut state_changes = 0;
    let mut errors = 0;

    for event in trace.iter() {
        match event {
            evolve_debugger::TraceEvent::BlockStart { .. } => {
                tx_count = 0;
                state_changes = 0;
                errors = 0;
            }
            evolve_debugger::TraceEvent::BlockEnd { height, .. } => {
                summaries.push(BlockSummary {
                    height: *height,
                    tx_count,
                    state_changes,
                    errors,
                });
            }
            evolve_debugger::TraceEvent::TxStart { .. } => {
                tx_count += 1;
            }
            evolve_debugger::TraceEvent::StateChange { .. } => {
                state_changes += 1;
            }
            evolve_debugger::TraceEvent::Error { .. } => {
                errors += 1;
            }
            _ => {}
        }
    }

    summaries
}

fn format_text(report: &ReportData) -> String {
    let mut output = String::new();

    output.push_str("=== Trace Report ===\n\n");
    output.push_str(&format!("Seed: {}\n\n", report.seed));

    output.push_str("Summary:\n");
    output.push_str(&format!(
        "  Total events: {}\n",
        report.summary.total_events
    ));
    output.push_str(&format!(
        "  Total blocks: {}\n",
        report.summary.total_blocks
    ));
    output.push_str(&format!(
        "  Total transactions: {}\n",
        report.summary.total_txs
    ));
    output.push_str(&format!(
        "  Total state changes: {}\n",
        report.summary.total_state_changes
    ));
    output.push_str(&format!(
        "  Total errors: {}\n\n",
        report.summary.total_errors
    ));

    output.push_str("Event Breakdown:\n");
    output.push_str(&format!("  Block events: {}\n", report.events.block_events));
    output.push_str(&format!(
        "  Transaction events: {}\n",
        report.events.tx_events
    ));
    output.push_str(&format!(
        "  State changes: {}\n",
        report.events.state_changes
    ));
    output.push_str(&format!("  Calls: {}\n", report.events.calls));
    output.push_str(&format!("  Gas charges: {}\n", report.events.gas_charges));
    output.push_str(&format!(
        "  Events emitted: {}\n",
        report.events.events_emitted
    ));
    output.push_str(&format!("  Errors: {}\n\n", report.events.errors));

    output.push_str("Final State:\n");
    output.push_str(&format!("  Key count: {}\n", report.state.final_key_count));
    output.push_str(&format!(
        "  Size: {} bytes\n",
        report.state.final_size_bytes
    ));

    if !report.state.top_prefixes.is_empty() {
        output.push_str("  Top prefixes:\n");
        for (prefix, count) in &report.state.top_prefixes {
            output.push_str(&format!("    {}: {}\n", prefix, count));
        }
    }

    if let Some(ref per_block) = report.per_block {
        output.push_str("\nPer-Block Summary:\n");
        output.push_str("  Height | Txs | State Changes | Errors\n");
        output.push_str("  -------+-----+---------------+-------\n");
        for block in per_block {
            output.push_str(&format!(
                "  {:>6} | {:>3} | {:>13} | {:>6}\n",
                block.height, block.tx_count, block.state_changes, block.errors
            ));
        }
    }

    output
}

fn format_csv(report: &ReportData) -> String {
    let mut output = String::new();

    // Header
    output.push_str("metric,value\n");

    // Summary
    output.push_str(&format!("seed,{}\n", report.seed));
    output.push_str(&format!("total_events,{}\n", report.summary.total_events));
    output.push_str(&format!("total_blocks,{}\n", report.summary.total_blocks));
    output.push_str(&format!("total_txs,{}\n", report.summary.total_txs));
    output.push_str(&format!(
        "total_state_changes,{}\n",
        report.summary.total_state_changes
    ));
    output.push_str(&format!("total_errors,{}\n", report.summary.total_errors));

    // Events
    output.push_str(&format!("block_events,{}\n", report.events.block_events));
    output.push_str(&format!("tx_events,{}\n", report.events.tx_events));
    output.push_str(&format!("state_changes,{}\n", report.events.state_changes));
    output.push_str(&format!("calls,{}\n", report.events.calls));
    output.push_str(&format!("gas_charges,{}\n", report.events.gas_charges));
    output.push_str(&format!(
        "events_emitted,{}\n",
        report.events.events_emitted
    ));
    output.push_str(&format!("errors,{}\n", report.events.errors));

    // State
    output.push_str(&format!(
        "final_key_count,{}\n",
        report.state.final_key_count
    ));
    output.push_str(&format!(
        "final_size_bytes,{}\n",
        report.state.final_size_bytes
    ));

    // Per-block if available
    if let Some(ref per_block) = report.per_block {
        output.push('\n');
        output.push_str("block_height,tx_count,state_changes,errors\n");
        for block in per_block {
            output.push_str(&format!(
                "{},{},{},{}\n",
                block.height, block.tx_count, block.state_changes, block.errors
            ));
        }
    }

    output
}
