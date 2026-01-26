//! Run simulation command.

// Testing CLI - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]

use evolve_debugger::{save_trace, StateSnapshot, TraceBuilder, TraceFormat};
use evolve_simulator::{MetricsConfig, SimConfig, Simulator, StorageConfig, TimeConfig};
use std::path::PathBuf;

/// Configuration for the run command.
pub struct RunConfig {
    pub seed: Option<u64>,
    pub blocks: u64,
    pub max_txs: usize,
    pub trace_output: Option<PathBuf>,
    pub read_fault_prob: f64,
    pub write_fault_prob: f64,
    pub output_format: String,
}

/// Execute the run command.
pub fn execute(config: RunConfig) -> Result<(), String> {
    // Determine seed
    let seed = config.seed.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42)
    });

    println!("Starting simulation with seed: {seed}");
    println!("Blocks to simulate: {}", config.blocks);
    println!();

    // Create simulator config
    let sim_config = SimConfig {
        time: TimeConfig {
            ticks_per_block: 10,
            tick_ms: 100,
            initial_timestamp_ms: 0,
        },
        storage: StorageConfig {
            read_fault_prob: config.read_fault_prob,
            write_fault_prob: config.write_fault_prob,
            log_operations: false,
        },
        metrics: MetricsConfig::default(),
        max_blocks: config.blocks,
        max_txs_per_block: config.max_txs,
        stop_on_error: false,
    };

    // Create simulator
    let mut simulator = Simulator::new(seed, sim_config);

    // Create trace builder if tracing enabled
    let mut trace_builder = config
        .trace_output
        .as_ref()
        .map(|_| TraceBuilder::new(seed, StateSnapshot::empty()));

    // Run simulation
    let start_time = std::time::Instant::now();

    for block_height in 0..config.blocks {
        // Record block start if tracing
        if let Some(ref mut builder) = trace_builder {
            builder.block_start(block_height, simulator.time().now_ms());
        }

        // Advance simulator
        if let Err(e) = simulator.run_blocks(1) {
            return Err(format!("Simulation failed at block {block_height}: {e:?}"));
        }

        // Record block end if tracing
        if let Some(ref mut builder) = trace_builder {
            builder.block_end(block_height, [0; 32]); // TODO: compute actual state hash
        }

        // Progress indicator
        if block_height > 0 && block_height % 10 == 0 {
            print!(".");
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }

    let elapsed = start_time.elapsed();
    println!();
    println!();

    // Generate report
    let report = simulator.generate_report();

    match config.output_format.as_str() {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
            );
        }
        _ => {
            println!("=== Simulation Report ===");
            println!();
            println!("Seed: {seed}");
            println!("Total blocks: {}", report.total_blocks);
            println!("Total transactions: {}", report.total_txs);
            println!("Avg txs/block: {:.2}", report.avg_txs_per_block);
            println!("Avg gas/tx: {:.2}", report.avg_gas_per_tx);
            println!();
            println!("Timing:");
            println!("  Avg block time: {:.2}ms", report.avg_block_time_ms);
            println!("  P50 block time: {:.2}ms", report.p50_block_time_ms);
            println!("  P99 block time: {:.2}ms", report.p99_block_time_ms);
            println!();
            println!("Storage:");
            println!("  State size: {} bytes", report.state_size_bytes);
            println!("  Ops per tx: {:.2}", report.storage_ops_per_tx);
            println!();
            println!("Wall clock time: {:.2?}", elapsed);
            println!(
                "Time compression: {:.1}x",
                (config.blocks as f64 * 1000.0) / elapsed.as_millis() as f64
            );
        }
    }

    // Save trace if requested
    if let (Some(path), Some(builder)) = (config.trace_output, trace_builder) {
        let trace = builder.finish();
        let format = if path.to_string_lossy().ends_with(".json") {
            TraceFormat::JsonPretty
        } else if path.to_string_lossy().ends_with(".gz") {
            TraceFormat::BinaryCompressed
        } else {
            TraceFormat::Binary
        };

        save_trace(&trace, &path, format).map_err(|e| format!("Failed to save trace: {e}"))?;

        println!();
        println!("Trace saved to: {}", path.display());
        println!("Events recorded: {}", trace.len());
    }

    // Print reproduction command
    println!();
    println!("To reproduce this simulation:");
    println!("  evolve-sim run --seed {seed} --blocks {}", config.blocks);

    Ok(())
}
