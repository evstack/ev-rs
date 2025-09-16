//! Configuration file loading.

use crate::config::types::NodeConfig;
use crate::config::validation::validate_config;
use crate::errors::ConfigError;
use std::path::Path;

/// Load and validate configuration from a YAML file.
///
/// This function:
/// 1. Reads the file from disk
/// 2. Parses the YAML content
/// 3. Validates all configuration values
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The YAML is invalid
/// - Any configuration value fails validation
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<NodeConfig, ConfigError> {
    let path = path.as_ref();
    let path_str = path.display().to_string();

    let content = std::fs::read_to_string(path).map_err(|e| ConfigError::FileRead {
        path: path_str.clone(),
        source: e,
    })?;

    load_config_from_str(&content, &path_str)
}

/// Load and validate configuration from a YAML string.
///
/// Useful for testing or when config is provided via other means.
pub fn load_config_from_str(content: &str, source_name: &str) -> Result<NodeConfig, ConfigError> {
    let config: NodeConfig = serde_yaml::from_str(content).map_err(|e| ConfigError::Parse {
        path: source_name.to_string(),
        source: e,
    })?;

    validate_config(&config)?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CONFIG: &str = r#"
chain:
  chain_id: 1
  gas_service_account: "18446744073709551615"

storage:
  path: "./data"
  cache_size: 1073741824
  write_buffer_size: 67108864

rpc:
  http_addr: "127.0.0.1:8545"
  enabled: true

operations:
  shutdown_timeout_secs: 30
  startup_checks: true
"#;

    #[test]
    fn test_load_valid_config() {
        let config = load_config_from_str(VALID_CONFIG, "config.yaml").unwrap();
        assert_eq!(config.chain.chain_id, 1);
        assert_eq!(config.storage.path, "./data");
        assert!(config.rpc.enabled);
    }

    #[test]
    fn test_unknown_field_rejected() {
        let config_with_unknown = r#"
chain:
  chain_id: 1
  gas_service_account: "100"
  unknown_field: "bad"

storage:
  path: "./data"
"#;
        let result = load_config_from_str(config_with_unknown, "config.yaml");
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::Parse { .. } => {}
            e => panic!("Expected Parse error, got {:?}", e),
        }
    }

    #[test]
    fn test_defaults_applied() {
        let minimal_config = r#"
chain:
  chain_id: 1
  gas_service_account: "100"

storage:
  path: "./data"
"#;
        let config = load_config_from_str(minimal_config, "config.yaml").unwrap();

        // RPC defaults
        assert!(config.rpc.enabled);
        assert_eq!(config.rpc.http_addr, "127.0.0.1:8545");

        // Operations defaults
        assert_eq!(config.operations.shutdown_timeout_secs, 30);
        assert!(config.operations.startup_checks);

        // Storage defaults
        assert_eq!(config.storage.cache_size, 1024 * 1024 * 1024);
    }

    #[test]
    fn test_invalid_yaml_syntax() {
        let bad_yaml = "chain:\n  chain_id: [invalid";
        let result = load_config_from_str(bad_yaml, "config.yaml");
        assert!(matches!(result, Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn test_validation_runs_after_parse() {
        let config_with_invalid_values = r#"
chain:
  chain_id: 0
  gas_service_account: "100"

storage:
  path: ""
"#;
        let result = load_config_from_str(config_with_invalid_values, "config.yaml");
        assert!(matches!(result, Err(ConfigError::ValidationFailed(_))));
    }
}
