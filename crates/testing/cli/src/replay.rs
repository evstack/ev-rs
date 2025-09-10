//! Replay trace command.

use evolve_debugger::{
    load_trace, Breakpoint, Replayer, StateInspector, StepResult, TraceEvent, TraceQuery,
};
use std::path::PathBuf;

/// Configuration for the replay command.
pub struct ReplayConfig {
    pub trace_path: PathBuf,
    pub jump_to_block: Option<u64>,
    pub break_on_error: bool,
    pub break_on_block: Option<u64>,
    pub break_on_key: Option<String>,
    pub interactive: bool,
    pub summary_only: bool,
}

/// Execute the replay command.
pub fn execute(config: ReplayConfig) -> Result<(), String> {
    // Load trace
    println!("Loading trace from: {}", config.trace_path.display());
    let trace = load_trace(&config.trace_path).map_err(|e| format!("Failed to load trace: {e}"))?;

    println!("Trace loaded successfully");
    println!("  Seed: {}", trace.seed);
    println!("  Events: {}", trace.len());
    println!("  Blocks: {}", trace.metadata.total_blocks);
    println!("  Transactions: {}", trace.metadata.total_txs);
    println!("  State changes: {}", trace.metadata.total_state_changes);
    println!();

    // Summary only mode
    if config.summary_only {
        print_trace_summary(&trace);
        return Ok(());
    }

    // Create replayer
    let mut replayer = Replayer::new(trace.clone());

    // Add breakpoints
    if config.break_on_error {
        replayer.add_breakpoint(Breakpoint::on_error());
        println!("Breakpoint set: on error");
    }
    if let Some(block) = config.break_on_block {
        replayer.add_breakpoint(Breakpoint::on_block(block));
        println!("Breakpoint set: block {block}");
    }
    if let Some(ref key) = config.break_on_key {
        replayer.add_breakpoint(Breakpoint::OnStoragePrefix(key.as_bytes().to_vec()));
        println!("Breakpoint set: storage key prefix \"{key}\"");
    }

    // Jump to specific block if requested
    if let Some(block) = config.jump_to_block {
        println!("Jumping to block {block}...");
        replayer.goto_block(block);
        print_current_position(&replayer);
    }

    // Interactive mode
    if config.interactive {
        run_interactive_mode(&mut replayer)?;
    } else if !replayer.breakpoints().is_empty() {
        // Non-interactive with breakpoints: run to first breakpoint
        println!();
        println!("Running to breakpoint...");
        let result = replayer.run_to_breakpoint();
        match result {
            StepResult::HitBreakpoint(idx) => {
                println!("Hit breakpoint {} at position {}", idx, replayer.position());
                print_current_position(&replayer);
                print_current_event(&replayer);
            }
            StepResult::AtEnd => {
                println!("Reached end of trace without hitting breakpoint");
            }
            _ => {}
        }
    }

    // Print final state summary
    println!();
    let inspector = StateInspector::at_end(&trace);
    let summary = inspector.summary();
    println!("Final state:");
    println!("  Total keys: {}", summary.total_keys);
    println!("  Total bytes: {}", summary.total_bytes);

    // Print reproduction command
    println!();
    println!("To reproduce from seed:");
    println!("  evolve-sim run --seed {}", trace.seed);

    Ok(())
}

fn print_trace_summary(trace: &evolve_debugger::ExecutionTrace) {
    println!("=== Trace Summary ===");
    println!();

    // Count event types
    let blocks = TraceQuery::new(trace).blocks().count();
    let txs = TraceQuery::new(trace).transactions().count();
    let state_changes = TraceQuery::new(trace).state_changes().count();
    let errors = TraceQuery::new(trace).errors().count();

    println!("Event breakdown:");
    println!("  Block events: {}", blocks);
    println!("  Transaction events: {}", txs);
    println!("  State changes: {}", state_changes);
    println!("  Errors: {}", errors);
    println!();

    // Show first few events
    println!("First 10 events:");
    for (idx, event) in trace.iter().enumerate().take(10) {
        println!("  [{idx}] {}", format_event(event));
    }
    if trace.len() > 10 {
        println!("  ... and {} more events", trace.len() - 10);
    }
    println!();

    // Show any errors
    if errors > 0 {
        println!("Errors found:");
        for (idx, event) in TraceQuery::new(trace).errors().execute() {
            if let TraceEvent::Error { code, message, .. } = event {
                println!("  [{idx}] Error {code}: {message}");
            }
        }
        println!();
    }

    // State summary at end
    let inspector = StateInspector::at_end(trace);
    let summary = inspector.summary();
    println!("Final state:");
    println!("  Total keys: {}", summary.total_keys);
    println!("  Total bytes: {} bytes", summary.total_bytes);
    if !summary.prefix_counts.is_empty() {
        println!("  Key prefixes:");
        let mut prefixes: Vec<_> = summary.prefix_counts.iter().collect();
        prefixes.sort_by(|a, b| b.1.cmp(a.1));
        for (prefix, count) in prefixes.iter().take(5) {
            println!("    {}: {}", prefix, count);
        }
    }
}

fn print_current_position(replayer: &Replayer) {
    println!(
        "Position: {}/{}",
        replayer.position(),
        replayer.total_events()
    );
    if let Some(block) = replayer.current_block() {
        println!("Current block: {block}");
    }
    if let Some(tx) = replayer.current_tx() {
        println!("Current tx: {:02x}{:02x}...", tx[0], tx[1]);
    }
}

fn print_current_event(replayer: &Replayer) {
    if let Some(event) = replayer.current_event() {
        println!("Event: {}", format_event(event));
    }
}

fn format_event(event: &TraceEvent) -> String {
    match event {
        TraceEvent::BlockStart {
            height,
            timestamp_ms,
            ..
        } => {
            format!("BlockStart(height={height}, ts={timestamp_ms})")
        }
        TraceEvent::BlockEnd { height, .. } => {
            format!("BlockEnd(height={height})")
        }
        TraceEvent::TxStart { tx_id, .. } => {
            format!("TxStart(id={:02x}{:02x}...)", tx_id[0], tx_id[1])
        }
        TraceEvent::TxEnd {
            tx_id,
            success,
            gas_used,
            ..
        } => {
            format!(
                "TxEnd(id={:02x}{:02x}..., success={success}, gas={gas_used})",
                tx_id[0], tx_id[1]
            )
        }
        TraceEvent::StateChange { key, .. } => {
            let key_str = String::from_utf8_lossy(key);
            if key_str.len() > 20 {
                format!("StateChange(key=\"{}...\")", &key_str[..20])
            } else {
                format!("StateChange(key=\"{key_str}\")")
            }
        }
        TraceEvent::Call { function_id, .. } => {
            format!("Call(fn={function_id})")
        }
        TraceEvent::CallReturn { success, .. } => {
            format!("CallReturn(success={success})")
        }
        TraceEvent::GasCharge {
            amount, remaining, ..
        } => {
            format!("GasCharge(amount={amount}, remaining={remaining})")
        }
        TraceEvent::EventEmitted { name, .. } => {
            format!("Event(\"{name}\")")
        }
        TraceEvent::Error { code, message, .. } => {
            format!("Error({code}: \"{message}\")")
        }
        TraceEvent::Checkpoint { id, .. } => {
            format!("Checkpoint({id})")
        }
        TraceEvent::Rollback { to_checkpoint, .. } => {
            format!("Rollback(to={to_checkpoint})")
        }
    }
}

fn run_interactive_mode(replayer: &mut Replayer) -> Result<(), String> {
    use std::io::{self, BufRead, Write};

    println!();
    println!("=== Interactive Replay Mode ===");
    println!("Commands: step (s), back (b), goto <n>, run (r), state, event, quit (q)");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("[{}/{}] > ", replayer.position(), replayer.total_events());
        stdout.flush().ok();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            break;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "s" | "step" => match replayer.step() {
                StepResult::Ok => print_current_event(replayer),
                StepResult::HitBreakpoint(idx) => {
                    println!("Hit breakpoint {idx}");
                    print_current_event(replayer);
                }
                StepResult::AtEnd => println!("At end of trace"),
                StepResult::AtBeginning => println!("At beginning of trace"),
            },
            "b" | "back" => match replayer.step_back() {
                StepResult::Ok => print_current_event(replayer),
                StepResult::AtBeginning => println!("At beginning of trace"),
                _ => {}
            },
            "goto" if parts.len() > 1 => {
                if let Ok(pos) = parts[1].parse::<usize>() {
                    replayer.goto_position(pos);
                    print_current_position(replayer);
                } else {
                    println!("Invalid position");
                }
            }
            "r" | "run" => match replayer.run_to_breakpoint() {
                StepResult::HitBreakpoint(idx) => {
                    println!("Hit breakpoint {idx}");
                    print_current_event(replayer);
                }
                StepResult::AtEnd => println!("Reached end of trace"),
                _ => {}
            },
            "state" => {
                let state = replayer.current_state();
                println!("Current state: {} keys", state.len());
                for (key, value) in state.data.iter().take(10) {
                    let key_str = String::from_utf8_lossy(key);
                    let value_str = String::from_utf8_lossy(value);
                    println!("  {key_str} = {value_str}");
                }
                if state.len() > 10 {
                    println!("  ... and {} more keys", state.len() - 10);
                }
            }
            "event" => {
                print_current_event(replayer);
            }
            "pos" | "position" => {
                print_current_position(replayer);
            }
            "reset" => {
                replayer.reset();
                println!("Reset to beginning");
            }
            "q" | "quit" | "exit" => {
                break;
            }
            "help" | "?" => {
                println!("Commands:");
                println!("  s, step      - Step forward one event");
                println!("  b, back      - Step backward one event");
                println!("  goto <n>     - Jump to position n");
                println!("  r, run       - Run to next breakpoint");
                println!("  state        - Show current state");
                println!("  event        - Show current event");
                println!("  pos          - Show current position");
                println!("  reset        - Reset to beginning");
                println!("  q, quit      - Exit interactive mode");
            }
            _ => {
                println!("Unknown command. Type 'help' for available commands.");
            }
        }
    }

    Ok(())
}
