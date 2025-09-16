//! Configuration loading and validation.
//!
//! This module provides:
//! - Configuration types with serde support
//! - TOML file loading
//! - Fail-fast validation that collects all errors

mod loader;
pub mod types;
mod validation;

pub use loader::{load_config, load_config_from_str};
pub use types::{
    ChainConfig, NodeConfig, ObservabilityConfig, OperationsConfig, RpcConfig, StorageConfig,
};
pub use validation::validate_config;
