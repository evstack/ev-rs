//! Configuration validation.
//!
//! Validates configuration and collects all errors before returning,
//! enabling users to fix multiple issues in a single iteration.

use crate::config::types::{
    NodeConfig, ObservabilityConfig, OperationsConfig, RpcConfig, StorageConfig,
};
use crate::errors::ConfigError;

/// Minimum allowed cache size: 16MB.
const MIN_CACHE_SIZE: u64 = 16 * 1024 * 1024;
/// Maximum allowed cache size: 64GB.
const MAX_CACHE_SIZE: u64 = 64 * 1024 * 1024 * 1024;

/// Minimum allowed write buffer size: 4MB.
const MIN_WRITE_BUFFER_SIZE: u64 = 4 * 1024 * 1024;
/// Maximum allowed write buffer size: 1GB.
const MAX_WRITE_BUFFER_SIZE: u64 = 1024 * 1024 * 1024;

/// Minimum shutdown timeout: 1 second.
const MIN_SHUTDOWN_TIMEOUT: u64 = 1;
/// Maximum shutdown timeout: 300 seconds (5 minutes).
const MAX_SHUTDOWN_TIMEOUT: u64 = 300;

/// Validate the entire node configuration.
///
/// Collects all validation errors and returns them together, allowing users
/// to fix multiple issues at once.
pub fn validate_config(config: &NodeConfig) -> Result<(), ConfigError> {
    let mut errors = Vec::new();

    validate_chain_config(config, &mut errors);
    validate_storage_config(&config.storage, &mut errors);
    validate_rpc_config(&config.rpc, &mut errors);
    validate_operations_config(&config.operations, &mut errors);
    validate_observability_config(&config.observability, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::ValidationFailed(errors))
    }
}

fn validate_chain_config(config: &NodeConfig, errors: &mut Vec<String>) {
    if config.chain.chain_id == 0 {
        errors.push("chain.chain_id must be greater than 0".to_string());
    }
}

fn validate_storage_config(config: &StorageConfig, errors: &mut Vec<String>) {
    if config.path.is_empty() {
        errors.push("storage.path cannot be empty".to_string());
    }

    if config.cache_size < MIN_CACHE_SIZE {
        errors.push(format!(
            "storage.cache_size must be at least {} bytes ({} MB)",
            MIN_CACHE_SIZE,
            MIN_CACHE_SIZE / (1024 * 1024)
        ));
    }

    if config.cache_size > MAX_CACHE_SIZE {
        errors.push(format!(
            "storage.cache_size must be at most {} bytes ({} GB)",
            MAX_CACHE_SIZE,
            MAX_CACHE_SIZE / (1024 * 1024 * 1024)
        ));
    }

    if config.write_buffer_size < MIN_WRITE_BUFFER_SIZE {
        errors.push(format!(
            "storage.write_buffer_size must be at least {} bytes ({} MB)",
            MIN_WRITE_BUFFER_SIZE,
            MIN_WRITE_BUFFER_SIZE / (1024 * 1024)
        ));
    }

    if config.write_buffer_size > MAX_WRITE_BUFFER_SIZE {
        errors.push(format!(
            "storage.write_buffer_size must be at most {} bytes ({} GB)",
            MAX_WRITE_BUFFER_SIZE,
            MAX_WRITE_BUFFER_SIZE / (1024 * 1024 * 1024)
        ));
    }

    if config.partition_prefix.is_empty() {
        errors.push("storage.partition_prefix cannot be empty".to_string());
    }

    // Validate partition_prefix contains only valid characters (alphanumeric, hyphen, underscore)
    if !config
        .partition_prefix
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        errors.push(format!(
            "storage.partition_prefix '{}' contains invalid characters. Only alphanumeric, hyphen, and underscore are allowed.",
            config.partition_prefix
        ));
    }
}

fn validate_rpc_config(config: &RpcConfig, errors: &mut Vec<String>) {
    if config.enabled && config.http_addr.is_empty() {
        errors.push("rpc.http_addr cannot be empty when RPC is enabled".to_string());
    }

    // Basic address format validation
    if config.enabled && !config.http_addr.is_empty() {
        if !config.http_addr.contains(':') {
            errors.push(format!(
                "rpc.http_addr '{}' must be in host:port format",
                config.http_addr
            ));
        } else {
            let parts: Vec<&str> = config.http_addr.rsplitn(2, ':').collect();
            if parts.len() == 2 && parts[0].parse::<u16>().is_err() {
                errors.push(format!(
                    "rpc.http_addr '{}' has invalid port",
                    config.http_addr
                ));
            }
        }
    }
}

fn validate_operations_config(config: &OperationsConfig, errors: &mut Vec<String>) {
    if config.shutdown_timeout_secs < MIN_SHUTDOWN_TIMEOUT {
        errors.push(format!(
            "operations.shutdown_timeout_secs must be at least {} second(s)",
            MIN_SHUTDOWN_TIMEOUT
        ));
    }

    if config.shutdown_timeout_secs > MAX_SHUTDOWN_TIMEOUT {
        errors.push(format!(
            "operations.shutdown_timeout_secs must be at most {} seconds",
            MAX_SHUTDOWN_TIMEOUT
        ));
    }
}

fn validate_observability_config(config: &ObservabilityConfig, errors: &mut Vec<String>) {
    let valid_levels = [
        "trace", "debug", "info", "warn", "warning", "error", "critical", "crit",
    ];
    if !valid_levels.contains(&config.log_level.to_lowercase().as_str()) {
        errors.push(format!(
            "observability.log_level '{}' is invalid. Valid levels: trace, debug, info, warn, error, critical",
            config.log_level
        ));
    }

    let valid_formats = ["json", "pretty", "text", "human"];
    if !valid_formats.contains(&config.log_format.to_lowercase().as_str()) {
        errors.push(format!(
            "observability.log_format '{}' is invalid. Valid formats: json, pretty",
            config.log_format
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::ChainConfig;

    fn valid_config() -> NodeConfig {
        NodeConfig {
            chain: ChainConfig {
                chain_id: 1,
                gas_service_account: u128::MAX,
            },
            storage: StorageConfig {
                path: "./data".to_string(),
                cache_size: 1024 * 1024 * 1024,
                write_buffer_size: 64 * 1024 * 1024,
                partition_prefix: "evolve-state".to_string(),
            },
            rpc: RpcConfig::default(),
            operations: OperationsConfig::default(),
            observability: ObservabilityConfig::default(),
        }
    }

    #[test]
    fn test_valid_config_passes() {
        let config = valid_config();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_zero_chain_id_fails() {
        let mut config = valid_config();
        config.chain.chain_id = 0;

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains("chain_id"));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_empty_storage_path_fails() {
        let mut config = valid_config();
        config.storage.path = String::new();

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("storage.path")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_cache_size_bounds() {
        let mut config = valid_config();

        // Too small
        config.storage.cache_size = MIN_CACHE_SIZE - 1;
        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("cache_size")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }

        // Too large
        config.storage.cache_size = MAX_CACHE_SIZE + 1;
        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("cache_size")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_multiple_errors_collected() {
        let mut config = valid_config();
        config.chain.chain_id = 0;
        config.storage.path = String::new();
        config.operations.shutdown_timeout_secs = 0;

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(
                    errors.len() >= 3,
                    "Expected at least 3 errors, got {}",
                    errors.len()
                );
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_invalid_rpc_addr_format() {
        let mut config = valid_config();
        config.rpc.http_addr = "invalid".to_string();

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("host:port")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_empty_partition_prefix_fails() {
        let mut config = valid_config();
        config.storage.partition_prefix = String::new();

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("partition_prefix")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_invalid_partition_prefix_chars() {
        let mut config = valid_config();
        config.storage.partition_prefix = "invalid/prefix".to_string();

        let err = validate_config(&config).unwrap_err();
        match err {
            ConfigError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("partition_prefix")));
                assert!(errors.iter().any(|e| e.contains("invalid characters")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_valid_partition_prefix() {
        let mut config = valid_config();

        // These should all pass
        for prefix in ["evolve-state", "mychain_data", "test123", "a-b_c"] {
            config.storage.partition_prefix = prefix.to_string();
            assert!(
                validate_config(&config).is_ok(),
                "Expected '{}' to be valid",
                prefix
            );
        }
    }
}
