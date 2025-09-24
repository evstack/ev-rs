//! Server builder for configuring and constructing an EvolveServer.

use crate::error::ServerError;
use crate::indexer::spawn_indexer;
use crate::server::EvolveServer;
use evolve_operations::config::{
    load_config, ChainConfig, GrpcConfig, NodeConfig, ObservabilityConfig, OperationsConfig,
    RpcConfig, StorageConfig,
};
use evolve_operations::init_logging_from_config;
use evolve_operations::shutdown::ShutdownCoordinator;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Default indexer buffer size.
const DEFAULT_INDEXER_BUFFER: usize = 256;

/// Builder for constructing an `EvolveServer`.
///
/// If no config is provided, sensible defaults are used.
///
/// # Example
///
/// ```ignore
/// // With config file
/// let server = EvolveServer::<MyStf, MyStorage, MyCodes>::builder()
///     .config_path("./config.yaml")
///     .build(stf, storage, codes)
///     .await?;
///
/// // With defaults (no config needed)
/// let server = EvolveServer::<MyStf, MyStorage, MyCodes>::builder()
///     .build(stf, storage, codes)
///     .await?;
/// ```
pub struct ServerBuilder<Stf, Storage, Codes> {
    config_path: Option<PathBuf>,
    config: Option<NodeConfig>,
    indexer_buffer_size: usize,
    _marker: PhantomData<(Stf, Storage, Codes)>,
}

impl<Stf, Storage, Codes> Default for ServerBuilder<Stf, Storage, Codes> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Stf, Storage, Codes> ServerBuilder<Stf, Storage, Codes> {
    /// Create a new server builder.
    pub fn new() -> Self {
        Self {
            config_path: None,
            config: None,
            indexer_buffer_size: DEFAULT_INDEXER_BUFFER,
            _marker: PhantomData,
        }
    }

    /// Set the config file path.
    pub fn config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the config directly.
    pub fn config(mut self, config: NodeConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the indexer buffer size.
    pub fn indexer_buffer(mut self, size: usize) -> Self {
        self.indexer_buffer_size = size;
        self
    }

    /// Create default config with sensible values.
    fn default_config() -> NodeConfig {
        NodeConfig {
            chain: ChainConfig {
                chain_id: 1,
                gas: evolve_operations::config::GasConfig::default(),
            },
            storage: StorageConfig {
                path: "./data".to_string(),
                cache_size: 1024 * 1024 * 1024,      // 1GB
                write_buffer_size: 64 * 1024 * 1024, // 64MB
                partition_prefix: "evolve-state".to_string(),
            },
            rpc: RpcConfig::default(),
            grpc: GrpcConfig::default(),
            operations: OperationsConfig::default(),
            observability: ObservabilityConfig::default(),
        }
    }

    fn resolve_config(&self) -> Result<NodeConfig, ServerError> {
        // Priority: explicit config > config file > defaults
        if let Some(ref config) = self.config {
            return Ok(config.clone());
        }

        if let Some(ref path) = self.config_path {
            return load_config(path).map_err(|e| ServerError::Config(e.to_string()));
        }

        // Use defaults
        Ok(Self::default_config())
    }

    /// Build the server.
    pub async fn build(
        self,
        stf: Stf,
        storage: Storage,
        account_codes: Codes,
    ) -> Result<EvolveServer<Stf, Storage, Codes>, ServerError>
    where
        Stf: Send + Sync + 'static,
        Storage: Send + Sync + 'static,
        Codes: Send + Sync + 'static,
    {
        let config = self.resolve_config()?;

        // Initialize logging
        let (_logger, _level_switch) = init_logging_from_config(
            &config.observability.log_level,
            &config.observability.log_format,
        );
        log::info!("Logging initialized");

        // Spawn indexer
        let indexer = spawn_indexer(self.indexer_buffer_size);
        log::info!("Indexer started");

        // Setup shutdown coordinator
        let shutdown_timeout = Duration::from_secs(config.operations.shutdown_timeout_secs);
        let shutdown = ShutdownCoordinator::new(shutdown_timeout);

        log::info!("EvolveServer ready");

        Ok(EvolveServer {
            config,
            storage: Arc::new(storage),
            account_codes: Arc::new(account_codes),
            stf: Arc::new(stf),
            indexer,
            pending_block: tokio::sync::RwLock::new(None),
            shutdown,
        })
    }
}
