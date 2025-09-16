//! Error types for the operations crate.

use thiserror::Error;

/// Configuration-related errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// File I/O error when loading config.
    #[error("failed to read config file '{path}': {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },

    /// YAML parsing error.
    #[error("failed to parse config file '{path}': {source}")]
    Parse {
        path: String,
        source: serde_yaml::Error,
    },

    /// Validation failed with one or more errors.
    #[error("config validation failed:\n{}", .0.join("\n"))]
    ValidationFailed(Vec<String>),
}

/// Startup-related errors.
#[derive(Debug, Error)]
pub enum StartupError {
    /// Storage path does not exist.
    #[error("storage path does not exist: {0}")]
    PathNotFound(String),

    /// Storage path is not a directory.
    #[error("storage path is not a directory: {0}")]
    NotADirectory(String),

    /// Storage path is not writable.
    #[error("storage path is not writable: {0}")]
    NotWritable(String),

    /// Insufficient disk space.
    #[error(
        "insufficient disk space at {path}: {available_mb}MB available, {required_mb}MB required"
    )]
    InsufficientDiskSpace {
        path: String,
        available_mb: u64,
        required_mb: u64,
    },

    /// Storage integrity check failed.
    #[error("storage integrity check failed: {0}")]
    IntegrityCheckFailed(String),

    /// General startup failure.
    #[error("startup failed: {0}")]
    Failed(String),
}

/// Shutdown-related errors.
#[derive(Debug, Error)]
pub enum ShutdownError {
    /// Shutdown timed out.
    #[error("shutdown timed out after {0} seconds")]
    Timeout(u64),

    /// Component failed to shut down cleanly.
    #[error("component '{name}' failed to shut down: {reason}")]
    ComponentFailed { name: String, reason: String },
}
