//! Startup sequence orchestration.

use crate::config::NodeConfig;
use crate::errors::StartupError;
use crate::startup::checks::{check_disk_space, check_storage_path};

/// Result of running startup checks.
#[derive(Debug)]
pub struct StartupResult {
    /// Warnings that were generated during startup checks.
    /// These are non-fatal issues that should be logged.
    pub warnings: Vec<String>,
}

impl StartupResult {
    fn new() -> Self {
        Self {
            warnings: Vec::new(),
        }
    }

    fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

/// Run all startup checks based on the configuration.
///
/// This function:
/// 1. Validates the storage path exists and is writable
/// 2. Checks for sufficient disk space
/// 3. Optionally runs a custom integrity check
///
/// # Arguments
///
/// * `config` - The node configuration
/// * `integrity_check` - Optional closure to check storage integrity
///
/// # Returns
///
/// Returns `StartupResult` containing any warnings, or an error if
/// a critical check fails.
pub fn run_startup_sequence<F, E>(
    config: &NodeConfig,
    integrity_check: Option<F>,
) -> Result<StartupResult, StartupError>
where
    F: FnOnce() -> Result<(), E>,
    E: std::fmt::Display,
{
    if !config.operations.startup_checks {
        log::info!("Startup checks disabled via configuration");
        return Ok(StartupResult::new());
    }

    log::info!("Running startup checks...");
    let mut result = StartupResult::new();

    // Check storage path
    log::debug!("Checking storage path: {}", config.storage.path);
    check_storage_path(&config.storage.path)?;
    log::info!("Storage path check passed: {}", config.storage.path);

    // Check disk space
    log::debug!(
        "Checking disk space (required: {}MB)",
        config.operations.min_disk_space_mb
    );
    if let Some(warning) =
        check_disk_space(&config.storage.path, config.operations.min_disk_space_mb)?
    {
        log::warn!("{}", warning);
        result.add_warning(warning);
    } else {
        log::info!("Disk space check passed");
    }

    // Run integrity check if provided
    if let Some(check) = integrity_check {
        log::debug!("Running storage integrity check");
        super::checks::check_state_integrity(check)?;
        log::info!("Storage integrity check passed");
    }

    log::info!(
        "Startup checks complete ({} warnings)",
        result.warnings.len()
    );
    Ok(result)
}

/// A simplified startup runner that creates the storage directory if needed.
///
/// Use this for development or when you want automatic directory creation.
pub fn run_startup_with_mkdir(config: &NodeConfig) -> Result<StartupResult, StartupError> {
    if !config.operations.startup_checks {
        return Ok(StartupResult::new());
    }

    // Create directory if it doesn't exist
    let path = std::path::Path::new(&config.storage.path);
    if !path.exists() {
        log::info!("Creating storage directory: {}", config.storage.path);
        std::fs::create_dir_all(path).map_err(|e| {
            StartupError::Failed(format!(
                "Failed to create storage directory '{}': {}",
                config.storage.path, e
            ))
        })?;
    }

    run_startup_sequence::<fn() -> Result<(), &'static str>, &'static str>(config, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ChainConfig, GasConfig, GrpcConfig, ObservabilityConfig, OperationsConfig, RpcConfig,
        StorageConfig,
    };
    use tempfile::TempDir;

    fn test_config(path: &str) -> NodeConfig {
        NodeConfig {
            chain: ChainConfig {
                chain_id: 1,
                gas: GasConfig::default(),
            },
            storage: StorageConfig {
                path: path.to_string(),
                cache_size: 1024 * 1024 * 1024,
                write_buffer_size: 64 * 1024 * 1024,
                partition_prefix: "evolve-state".to_string(),
            },
            rpc: RpcConfig::default(),
            grpc: GrpcConfig::default(),
            operations: OperationsConfig::default(),
            observability: ObservabilityConfig::default(),
        }
    }

    #[test]
    fn test_startup_sequence_success() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_str().unwrap());

        let result =
            run_startup_sequence::<fn() -> Result<(), &'static str>, &'static str>(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_startup_sequence_disabled() {
        let mut config = test_config("/nonexistent/path");
        config.operations.startup_checks = false;

        // Should succeed even with invalid path because checks are disabled
        let result =
            run_startup_sequence::<fn() -> Result<(), &'static str>, &'static str>(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_startup_sequence_path_not_found() {
        let config = test_config("/nonexistent/path/that/does/not/exist");

        let result =
            run_startup_sequence::<fn() -> Result<(), &'static str>, &'static str>(&config, None);
        assert!(matches!(result, Err(StartupError::PathNotFound(_))));
    }

    #[test]
    fn test_startup_with_mkdir() {
        let temp_dir = TempDir::new().unwrap();
        let new_path = temp_dir.path().join("new_data_dir");
        let config = test_config(new_path.to_str().unwrap());

        let result = run_startup_with_mkdir(&config);
        assert!(result.is_ok());
        assert!(new_path.exists());
    }

    #[test]
    fn test_startup_with_integrity_check() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_str().unwrap());

        // Success case
        let result = run_startup_sequence(&config, Some(|| Ok::<(), &str>(())));
        assert!(result.is_ok());

        // Failure case
        let result = run_startup_sequence(&config, Some(|| Err("db corrupted")));
        assert!(matches!(result, Err(StartupError::IntegrityCheckFailed(_))));
    }
}
