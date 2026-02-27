//! gRPC server startup and configuration.

use std::net::SocketAddr;
use std::sync::Arc;

use evolve_eth_jsonrpc::{SharedSubscriptionManager, StateProvider, SubscriptionManager};
use tonic::transport::Server;

use crate::proto::evolve::v1::{
    execution_service_server::ExecutionServiceServer,
    streaming_service_server::StreamingServiceServer,
};
use crate::services::{ExecutionServiceImpl, StreamingServiceImpl};

/// Configuration for the gRPC server.
#[derive(Debug, Clone)]
pub struct GrpcServerConfig {
    /// Address to bind the gRPC server to.
    pub addr: SocketAddr,
    /// Chain ID to return for GetChainId.
    pub chain_id: u64,
    /// Enable gzip compression.
    pub enable_gzip: bool,
    /// Maximum message size in bytes (default: 4MB).
    pub max_message_size: usize,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 9545)),
            chain_id: 1,
            enable_gzip: true,
            max_message_size: 4 * 1024 * 1024, // 4MB
        }
    }
}

/// gRPC server handle that can be used to manage the server lifecycle.
pub struct GrpcServerHandle {
    /// The address the server is listening on.
    pub addr: SocketAddr,
}

/// The gRPC server for the Evolve execution client.
pub struct GrpcServer<S: StateProvider> {
    config: GrpcServerConfig,
    state: Arc<S>,
    subscriptions: SharedSubscriptionManager,
}

impl<S: StateProvider> GrpcServer<S> {
    /// Create a new gRPC server with the given configuration and state provider.
    pub fn new(config: GrpcServerConfig, state: S) -> Self {
        Self {
            config,
            state: Arc::new(state),
            subscriptions: Arc::new(SubscriptionManager::new()),
        }
    }

    /// Create a new gRPC server with a shared subscription manager.
    ///
    /// Use this when you need to share event publishing with the JSON-RPC server.
    pub fn with_subscription_manager(
        config: GrpcServerConfig,
        state: S,
        subscriptions: SharedSubscriptionManager,
    ) -> Self {
        Self {
            config,
            state: Arc::new(state),
            subscriptions,
        }
    }

    /// Get a reference to the subscription manager.
    ///
    /// Use this to publish events (new blocks, logs, etc.) to subscribers.
    pub fn subscription_manager(&self) -> &SharedSubscriptionManager {
        &self.subscriptions
    }

    /// Start the gRPC server.
    ///
    /// This method runs the server until it is shut down.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = self.config.addr;

        let execution_service =
            ExecutionServiceImpl::new(self.config.chain_id, Arc::clone(&self.state));
        let streaming_service = StreamingServiceImpl::new(Arc::clone(&self.subscriptions));

        let mut execution_server = ExecutionServiceServer::new(execution_service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        let mut streaming_server = StreamingServiceServer::new(streaming_service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        if self.config.enable_gzip {
            execution_server = execution_server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
            streaming_server = streaming_server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
        }

        tracing::info!("Starting gRPC server on {}", addr);

        Server::builder()
            .add_service(execution_server)
            .add_service(streaming_server)
            .serve(addr)
            .await?;

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

        let execution_service =
            ExecutionServiceImpl::new(self.config.chain_id, Arc::clone(&self.state));
        let streaming_service = StreamingServiceImpl::new(Arc::clone(&self.subscriptions));

        let mut execution_server = ExecutionServiceServer::new(execution_service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        let mut streaming_server = StreamingServiceServer::new(streaming_service)
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        if self.config.enable_gzip {
            execution_server = execution_server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
            streaming_server = streaming_server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
        }

        tracing::info!("Starting gRPC server on {}", addr);

        Server::builder()
            .add_service(execution_server)
            .add_service(streaming_server)
            .serve_with_shutdown(addr, signal)
            .await?;

        Ok(())
    }
}

/// Start a gRPC server with the given configuration and state provider.
///
/// This is a convenience function for simple use cases.
pub async fn start_server<S: StateProvider>(
    config: GrpcServerConfig,
    state: S,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    GrpcServer::new(config, state).serve().await
}

/// Start a gRPC server with a shared subscription manager.
///
/// Use this when you need to share event publishing with the JSON-RPC server.
pub async fn start_server_with_subscriptions<S: StateProvider>(
    config: GrpcServerConfig,
    state: S,
    subscriptions: SharedSubscriptionManager,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    GrpcServer::with_subscription_manager(config, state, subscriptions)
        .serve()
        .await
}
