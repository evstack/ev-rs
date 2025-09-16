//! Operations infrastructure for Evolve nodes.
//!
//! This crate provides production-ready operational components:
//!
//! - **Config**: YAML-based configuration with fail-fast validation
//! - **Shutdown**: Graceful shutdown with SIGTERM/SIGINT handling
//! - **Startup**: Pre-flight checks for storage and state integrity
//!
//! # Example
//!
//! ```no_run
//! use evolve_operations::{
//!     config::load_config,
//!     shutdown::{ShutdownCoordinator, SignalHandler},
//!     startup::run_startup_sequence,
//! };
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Load and validate configuration
//!     let config = load_config("config.yaml")?;
//!
//!     // Run startup checks
//!     let result = run_startup_sequence::<fn() -> Result<(), &'static str>, &'static str>(&config, None)?;
//!     for warning in &result.warnings {
//!         eprintln!("Warning: {}", warning);
//!     }
//!
//!     // Set up shutdown handling
//!     let signal_handler = SignalHandler::new();
//!     let coordinator = ShutdownCoordinator::new(
//!         Duration::from_secs(config.operations.shutdown_timeout_secs)
//!     );
//!
//!     // Start the signal handler
//!     signal_handler.start();
//!
//!     // Wait for shutdown signal
//!     let mut rx = signal_handler.subscribe();
//!     rx.changed().await?;
//!
//!     // Perform graceful shutdown
//!     coordinator.shutdown().await;
//!
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod errors;
pub mod observability;
pub mod shutdown;
pub mod startup;

pub use config::{load_config, NodeConfig, ObservabilityConfig};
pub use errors::{ConfigError, ShutdownError, StartupError};
pub use observability::{
    init_logging, init_logging_from_config, parse_level, LogFormat, LogLevelSwitch, Logger,
    MetricsRegistry, NodeMetrics,
};
pub use shutdown::{ShutdownAware, ShutdownCoordinator, SignalHandler};
pub use startup::{run_startup_sequence, run_startup_with_mkdir, StartupResult};
