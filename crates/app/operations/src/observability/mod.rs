//! Observability infrastructure for Evolve nodes.
//!
//! This module provides production-ready observability components:
//!
//! - **Logging**: Structured JSON logging with runtime-adjustable levels
//! - **Metrics**: Prometheus-compatible metrics collection and export
//! - **Health**: Health status reporting for monitoring systems

pub mod logging;
pub mod metrics;

pub use logging::{
    init_logging, init_logging_from_config, parse_level, LogFormat, LogLevelSwitch, Logger,
};
pub use metrics::{MetricsRegistry, NodeMetrics};
