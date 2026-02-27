//! gRPC server for the EVNode ExecutorService.

use std::net::SocketAddr;

use evolve_core::ReadonlyKV;
use evolve_mempool::{Mempool, SharedMempool};
use evolve_stf_traits::AccountsCodeStorage;
use evolve_tx_eth::TxContext;
use tonic::transport::Server;

use crate::proto::evnode::v1::executor_service_server::ExecutorServiceServer;
use crate::service::{
    EvnodeStfExecutor, ExecutorServiceConfig, ExecutorServiceImpl, OnBlockExecuted,
    StateChangeCallback,
};

/// Configuration for the EVNode gRPC server.
#[derive(Debug, Clone)]
pub struct EvnodeServerConfig {
    /// Address to bind the gRPC server to.
    pub addr: SocketAddr,
    /// Enable gzip compression.
    pub enable_gzip: bool,
    /// Maximum message size in bytes (default: 4MB).
    pub max_message_size: usize,
    /// Executor service configuration.
    pub executor_config: ExecutorServiceConfig,
}

impl Default for EvnodeServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 50051)),
            enable_gzip: true,
            max_message_size: 4 * 1024 * 1024, // 4MB
            executor_config: ExecutorServiceConfig::default(),
        }
    }
}

/// The EVNode gRPC server.
pub struct EvnodeServer<Stf, S, Codes>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    config: EvnodeServerConfig,
    stf: Stf,
    storage: S,
    codes: Codes,
    mempool: Option<SharedMempool<Mempool<TxContext>>>,
    on_state_change: Option<StateChangeCallback>,
    on_block_executed: Option<OnBlockExecuted>,
}

impl<Stf, S, Codes> EvnodeServer<Stf, S, Codes>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    /// Create a new EVNode server.
    pub fn new(config: EvnodeServerConfig, stf: Stf, storage: S, codes: Codes) -> Self {
        Self {
            config,
            stf,
            storage,
            codes,
            mempool: None,
            on_state_change: None,
            on_block_executed: None,
        }
    }

    /// Create a new EVNode server with a mempool.
    pub fn with_mempool(
        config: EvnodeServerConfig,
        stf: Stf,
        storage: S,
        codes: Codes,
        mempool: SharedMempool<Mempool<TxContext>>,
    ) -> Self {
        Self {
            config,
            stf,
            storage,
            codes,
            mempool: Some(mempool),
            on_state_change: None,
            on_block_executed: None,
        }
    }

    /// Set a callback for state changes.
    ///
    /// This callback is invoked whenever state changes are produced by execution
    /// (including genesis initialization). Use this to persist state changes to storage.
    pub fn with_state_change_callback(mut self, callback: StateChangeCallback) -> Self {
        self.on_state_change = Some(callback);
        self
    }

    /// Set a callback for block execution.
    ///
    /// When set, this is called after `execute_txs` with the full block data.
    /// The callback takes responsibility for persisting state changes and indexing.
    pub fn with_on_block_executed(mut self, callback: OnBlockExecuted) -> Self {
        self.on_block_executed = Some(callback);
        self
    }

    /// Start the gRPC server.
    ///
    /// This method runs the server until it is shut down.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = self.config.addr;

        let mut service = match self.mempool {
            Some(mempool) => ExecutorServiceImpl::with_mempool(
                self.stf,
                self.storage,
                self.codes,
                self.config.executor_config,
                mempool,
            ),
            None => ExecutorServiceImpl::new(
                self.stf,
                self.storage,
                self.codes,
                self.config.executor_config,
            ),
        };

        if let Some(callback) = self.on_state_change {
            service = service.with_state_change_callback(callback);
        }

        if let Some(callback) = self.on_block_executed {
            service = service.with_on_block_executed(callback);
        }

        let mut server = ExecutorServiceServer::new(service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        if self.config.enable_gzip {
            server = server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
        }

        tracing::info!("Starting EVNode gRPC server on {}", addr);

        Server::builder().add_service(server).serve(addr).await?;

        Ok(())
    }

    /// Start the gRPC server with graceful shutdown.
    ///
    /// The server will shut down when the provided signal is received.
    pub async fn serve_with_shutdown<F>(
        self,
        signal: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send,
    {
        let addr = self.config.addr;

        let mut service = match self.mempool {
            Some(mempool) => ExecutorServiceImpl::with_mempool(
                self.stf,
                self.storage,
                self.codes,
                self.config.executor_config,
                mempool,
            ),
            None => ExecutorServiceImpl::new(
                self.stf,
                self.storage,
                self.codes,
                self.config.executor_config,
            ),
        };

        if let Some(callback) = self.on_state_change {
            service = service.with_state_change_callback(callback);
        }

        if let Some(callback) = self.on_block_executed {
            service = service.with_on_block_executed(callback);
        }

        let mut server = ExecutorServiceServer::new(service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        if self.config.enable_gzip {
            server = server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
        }

        tracing::info!("Starting EVNode gRPC server on {}", addr);

        Server::builder()
            .add_service(server)
            .serve_with_shutdown(addr, signal)
            .await?;

        Ok(())
    }
}

/// Start an EVNode gRPC server with the given configuration.
pub async fn start_evnode_server<Stf, S, Codes>(
    config: EvnodeServerConfig,
    stf: Stf,
    storage: S,
    codes: Codes,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    EvnodeServer::new(config, stf, storage, codes).serve().await
}

/// Start an EVNode gRPC server with a mempool.
pub async fn start_evnode_server_with_mempool<Stf, S, Codes>(
    config: EvnodeServerConfig,
    stf: Stf,
    storage: S,
    codes: Codes,
    mempool: SharedMempool<Mempool<TxContext>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: ReadonlyKV + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: EvnodeStfExecutor<S, Codes> + 'static,
{
    EvnodeServer::with_mempool(config, stf, storage, codes, mempool)
        .serve()
        .await
}
