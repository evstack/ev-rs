//! Configuration types for the node.

use serde::Deserialize;

/// Root configuration for a node.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// Chain-specific configuration.
    pub chain: ChainConfig,

    /// Storage configuration.
    pub storage: StorageConfig,

    /// JSON-RPC server configuration.
    #[serde(default)]
    pub rpc: RpcConfig,

    /// gRPC server configuration.
    #[serde(default)]
    pub grpc: GrpcConfig,

    /// Operations configuration.
    #[serde(default)]
    pub operations: OperationsConfig,

    /// Observability configuration.
    #[serde(default)]
    pub observability: ObservabilityConfig,
}

/// Chain-specific configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainConfig {
    /// Unique chain identifier. Must be > 0.
    pub chain_id: u64,

    /// Gas configuration for storage operations.
    #[serde(default)]
    pub gas: GasConfig,
}

/// Gas configuration for storage operations.
///
/// This configuration defines the gas costs for different storage operations.
/// All charges are per-byte costs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasConfig {
    /// Gas charged per byte for storage read operations.
    #[serde(default = "GasConfig::default_storage_get_charge")]
    pub storage_get_charge: u64,
    /// Gas charged per byte for storage write operations.
    #[serde(default = "GasConfig::default_storage_set_charge")]
    pub storage_set_charge: u64,
    /// Gas charged per byte for storage delete operations.
    #[serde(default = "GasConfig::default_storage_remove_charge")]
    pub storage_remove_charge: u64,
}

impl Default for GasConfig {
    fn default() -> Self {
        Self {
            storage_get_charge: Self::default_storage_get_charge(),
            storage_set_charge: Self::default_storage_set_charge(),
            storage_remove_charge: Self::default_storage_remove_charge(),
        }
    }
}

impl GasConfig {
    const fn default_storage_get_charge() -> u64 {
        10
    }

    const fn default_storage_set_charge() -> u64 {
        10
    }

    const fn default_storage_remove_charge() -> u64 {
        10
    }
}

/// Storage configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Path to the storage directory.
    pub path: String,

    /// Cache size in bytes. Default: 1GB.
    #[serde(default = "StorageConfig::default_cache_size")]
    pub cache_size: u64,

    /// Write buffer size in bytes. Default: 64MB.
    #[serde(default = "StorageConfig::default_write_buffer_size")]
    pub write_buffer_size: u64,

    /// Partition prefix for evolve state. Default: "evolve-state".
    #[serde(default = "StorageConfig::default_partition_prefix")]
    pub partition_prefix: String,
}

impl StorageConfig {
    /// Default cache size: 1GB.
    const fn default_cache_size() -> u64 {
        1024 * 1024 * 1024
    }

    /// Default write buffer size: 64MB.
    const fn default_write_buffer_size() -> u64 {
        64 * 1024 * 1024
    }

    /// Default partition prefix.
    fn default_partition_prefix() -> String {
        "evolve-state".to_string()
    }

    /// Convert to the storage crate's config type.
    pub fn to_storage_config(&self) -> evolve_storage::StorageConfig {
        evolve_storage::StorageConfig {
            path: std::path::PathBuf::from(&self.path),
            cache_size: self.cache_size,
            write_buffer_size: self.write_buffer_size,
            partition_prefix: self.partition_prefix.clone(),
        }
    }
}

/// RPC server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcConfig {
    /// HTTP address to bind to.
    #[serde(default = "RpcConfig::default_http_addr")]
    pub http_addr: String,

    /// Whether the RPC server is enabled.
    #[serde(default = "RpcConfig::default_enabled")]
    pub enabled: bool,

    /// Client version string.
    #[serde(default = "RpcConfig::default_client_version")]
    pub client_version: String,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            http_addr: Self::default_http_addr(),
            enabled: Self::default_enabled(),
            client_version: Self::default_client_version(),
        }
    }
}

impl RpcConfig {
    fn default_http_addr() -> String {
        "127.0.0.1:8545".to_string()
    }

    const fn default_enabled() -> bool {
        true
    }

    fn default_client_version() -> String {
        "evolve/0.1.0".to_string()
    }
}

/// gRPC server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrpcConfig {
    /// gRPC address to bind to.
    #[serde(default = "GrpcConfig::default_addr")]
    pub addr: String,

    /// Whether the gRPC server is enabled.
    #[serde(default = "GrpcConfig::default_enabled")]
    pub enabled: bool,

    /// Enable gzip compression.
    #[serde(default = "GrpcConfig::default_enable_gzip")]
    pub enable_gzip: bool,

    /// Maximum message size in bytes. Default: 4MB.
    #[serde(default = "GrpcConfig::default_max_message_size")]
    pub max_message_size: usize,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            addr: Self::default_addr(),
            enabled: Self::default_enabled(),
            enable_gzip: Self::default_enable_gzip(),
            max_message_size: Self::default_max_message_size(),
        }
    }
}

impl GrpcConfig {
    fn default_addr() -> String {
        "127.0.0.1:9545".to_string()
    }

    const fn default_enabled() -> bool {
        false // Disabled by default, opt-in
    }

    const fn default_enable_gzip() -> bool {
        true
    }

    const fn default_max_message_size() -> usize {
        4 * 1024 * 1024 // 4MB
    }

    /// Convert to the grpc crate's config type.
    pub fn to_grpc_server_config(&self, chain_id: u64) -> evolve_grpc::GrpcServerConfig {
        evolve_grpc::GrpcServerConfig {
            addr: self.addr.parse().expect("invalid gRPC address"),
            chain_id,
            enable_gzip: self.enable_gzip,
            max_message_size: self.max_message_size,
        }
    }
}

/// Operations configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationsConfig {
    /// Shutdown timeout in seconds. Default: 30.
    #[serde(default = "OperationsConfig::default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,

    /// Whether to run startup checks. Default: true.
    #[serde(default = "OperationsConfig::default_startup_checks")]
    pub startup_checks: bool,

    /// Minimum required disk space in MB. Default: 1024 (1GB).
    #[serde(default = "OperationsConfig::default_min_disk_space_mb")]
    pub min_disk_space_mb: u64,
}

impl Default for OperationsConfig {
    fn default() -> Self {
        Self {
            shutdown_timeout_secs: Self::default_shutdown_timeout_secs(),
            startup_checks: Self::default_startup_checks(),
            min_disk_space_mb: Self::default_min_disk_space_mb(),
        }
    }
}

impl OperationsConfig {
    const fn default_shutdown_timeout_secs() -> u64 {
        30
    }

    const fn default_startup_checks() -> bool {
        true
    }

    const fn default_min_disk_space_mb() -> u64 {
        1024
    }
}

/// Observability configuration for logging and metrics.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Log level: trace, debug, info, warn, error. Default: info.
    #[serde(default = "ObservabilityConfig::default_log_level")]
    pub log_level: String,

    /// Log format: json or pretty. Default: json.
    #[serde(default = "ObservabilityConfig::default_log_format")]
    pub log_format: String,

    /// Whether Prometheus metrics are enabled. Default: true.
    #[serde(default = "ObservabilityConfig::default_metrics_enabled")]
    pub metrics_enabled: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: Self::default_log_level(),
            log_format: Self::default_log_format(),
            metrics_enabled: Self::default_metrics_enabled(),
        }
    }
}

impl ObservabilityConfig {
    fn default_log_level() -> String {
        "info".to_string()
    }

    fn default_log_format() -> String {
        "json".to_string()
    }

    const fn default_metrics_enabled() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_rpc_config() {
        let config = RpcConfig::default();
        assert_eq!(config.http_addr, "127.0.0.1:8545");
        assert!(config.enabled);
        assert_eq!(config.client_version, "evolve/0.1.0");
    }

    #[test]
    fn test_default_grpc_config() {
        let config = GrpcConfig::default();
        assert_eq!(config.addr, "127.0.0.1:9545");
        assert!(!config.enabled); // Disabled by default
        assert!(config.enable_gzip);
        assert_eq!(config.max_message_size, 4 * 1024 * 1024);
    }

    #[test]
    fn test_default_operations_config() {
        let config = OperationsConfig::default();
        assert_eq!(config.shutdown_timeout_secs, 30);
        assert!(config.startup_checks);
        assert_eq!(config.min_disk_space_mb, 1024);
    }

    #[test]
    fn test_default_storage_sizes() {
        assert_eq!(StorageConfig::default_cache_size(), 1024 * 1024 * 1024);
        assert_eq!(StorageConfig::default_write_buffer_size(), 64 * 1024 * 1024);
    }

    #[test]
    fn test_default_partition_prefix() {
        assert_eq!(StorageConfig::default_partition_prefix(), "evolve-state");
    }

    #[test]
    fn test_storage_config_conversion() {
        let ops_config = StorageConfig {
            path: "/var/data".to_string(),
            cache_size: 512 * 1024 * 1024,
            write_buffer_size: 32 * 1024 * 1024,
            partition_prefix: "custom-prefix".to_string(),
        };

        let storage_config = ops_config.to_storage_config();

        assert_eq!(storage_config.path, std::path::PathBuf::from("/var/data"));
        assert_eq!(storage_config.cache_size, 512 * 1024 * 1024);
        assert_eq!(storage_config.write_buffer_size, 32 * 1024 * 1024);
        assert_eq!(storage_config.partition_prefix, "custom-prefix");
    }
}
