//! # Evolve Simulation CLI
//!
//! Command-line interface for running simulations, replaying traces,
//! and generating performance reports.

mod replay;
mod report;
mod run;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Evolve SDK simulation and debugging CLI.
#[derive(Parser)]
#[command(name = "evolve-sim")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a simulation with the specified parameters.
    Run {
        /// Seed for deterministic simulation (random if not specified).
        #[arg(short, long)]
        seed: Option<u64>,

        /// Number of blocks to simulate.
        #[arg(short = 'n', long, default_value = "100")]
        blocks: u64,

        /// Maximum transactions per block.
        #[arg(long, default_value = "10")]
        max_txs: usize,

        /// Enable tracing and save to file.
        #[arg(short, long)]
        trace: Option<PathBuf>,

        /// Storage read fault probability (0.0 - 1.0).
        #[arg(long, default_value = "0.0")]
        read_fault_prob: f64,

        /// Storage write fault probability (0.0 - 1.0).
        #[arg(long, default_value = "0.0")]
        write_fault_prob: f64,

        /// Output format for report (text, json).
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Replay a saved trace for debugging.
    Replay {
        /// Path to the trace file.
        #[arg(short, long)]
        trace: PathBuf,

        /// Jump to specific block height.
        #[arg(short, long)]
        block: Option<u64>,

        /// Set breakpoint on errors.
        #[arg(long)]
        break_on_error: bool,

        /// Set breakpoint on specific block.
        #[arg(long)]
        break_on_block: Option<u64>,

        /// Set breakpoint on storage key prefix.
        #[arg(long)]
        break_on_key: Option<String>,

        /// Interactive mode.
        #[arg(short, long)]
        interactive: bool,

        /// Output trace summary only.
        #[arg(long)]
        summary: bool,
    },

    /// Generate a report from simulation metrics or trace.
    Report {
        /// Path to trace file (optional, can generate from simulation).
        #[arg(short, long)]
        trace: Option<PathBuf>,

        /// Output format (text, json, csv).
        #[arg(short, long, default_value = "text")]
        format: String,

        /// Output file (stdout if not specified).
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show per-block breakdown.
        #[arg(long)]
        per_block: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run {
            seed,
            blocks,
            max_txs,
            trace,
            read_fault_prob,
            write_fault_prob,
            output,
        } => run::execute(run::RunConfig {
            seed,
            blocks,
            max_txs,
            trace_output: trace,
            read_fault_prob,
            write_fault_prob,
            output_format: output,
        }),

        Commands::Replay {
            trace,
            block,
            break_on_error,
            break_on_block,
            break_on_key,
            interactive,
            summary,
        } => replay::execute(replay::ReplayConfig {
            trace_path: trace,
            jump_to_block: block,
            break_on_error,
            break_on_block,
            break_on_key,
            interactive,
            summary_only: summary,
        }),

        Commands::Report {
            trace,
            format,
            output,
            per_block,
        } => report::execute(report::ReportConfig {
            trace_path: trace,
            output_format: format,
            output_path: output,
            per_block,
        }),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parse_replay_with_all_flags() {
        let cli = Cli::parse_from([
            "evolve-sim",
            "replay",
            "--trace",
            "trace.bin",
            "--block",
            "42",
            "--break-on-error",
            "--break-on-block",
            "99",
            "--break-on-key",
            "abc",
            "--interactive",
            "--summary",
        ]);

        match cli.command {
            Commands::Replay {
                trace,
                block,
                break_on_error,
                break_on_block,
                break_on_key,
                interactive,
                summary,
            } => {
                assert_eq!(trace, PathBuf::from("trace.bin"));
                assert_eq!(block, Some(42));
                assert!(break_on_error);
                assert_eq!(break_on_block, Some(99));
                assert_eq!(break_on_key.as_deref(), Some("abc"));
                assert!(interactive);
                assert!(summary);
            }
            _ => panic!("expected replay command"),
        }
    }

}
