//! gRPC server for the EVNode ExecutorService.

use std::net::SocketAddr;

use evolve_core::ReadonlyKV;
use evolve_mempool::{Mempool, SharedMempool};
use evolve_stf_traits::AccountsCodeStorage;
use evolve_tx_eth::TxContext;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Server;
use tonic::{Request, Status};

use crate::proto::evnode::v1::executor_service_server::ExecutorServiceServer;
use crate::service::{
    EvnodeStfExecutor, ExecutorServiceConfig, ExecutorServiceImpl, OnBlockExecuted,
    StateChangeCallback,
};

/// Metadata key used to carry the shared-secret auth token.
pub const AUTH_TOKEN_METADATA_KEY: &str = "x-evnode-token";

/// Build a tonic interceptor that enforces a shared-secret bearer token.
///
/// The interceptor reads the `x-evnode-token` metadata key from every
/// incoming request and rejects the call with `UNAUTHENTICATED` if the
/// value is absent or does not match `expected_token`.
///
/// # Security note
///
/// This provides defense-in-depth for the privileged evnode execution
/// interface. The server should **also** be bound to a loopback or VPN
/// interface — token auth is not a substitute for network-level isolation.
// `tonic::Status` is large by design; the Interceptor trait requires
// `Result<Request<()>, Status>` so we cannot box the error.
#[allow(clippy::result_large_err)]
pub fn make_auth_interceptor(
    expected_token: String,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone {
    move |req: Request<()>| {
        let provided = req
            .metadata()
            .get(AUTH_TOKEN_METADATA_KEY)
            .and_then(|v| v.to_str().ok());

        match provided {
            Some(token) if token == expected_token => Ok(req),
            _ => Err(Status::unauthenticated(
                "missing or invalid evnode auth token",
            )),
        }
    }
}

/// Configuration for the EVNode gRPC server.
#[derive(Clone)]
pub struct EvnodeServerConfig {
    /// Address to bind the gRPC server to.
    pub addr: SocketAddr,
    /// Enable gzip compression.
    pub enable_gzip: bool,
    /// Maximum message size in bytes (default: 4MB).
    pub max_message_size: usize,
    /// Executor service configuration.
    pub executor_config: ExecutorServiceConfig,
    /// Optional shared-secret auth token.
    ///
    /// When set, every inbound gRPC call must carry this value in the
    /// `x-evnode-token` metadata header. Calls without a valid token
    /// are rejected with `UNAUTHENTICATED`.
    ///
    /// The `EVOLVE_EVNODE_AUTH_TOKEN` environment variable is a convenient
    /// source for this value. When `require_auth` is `true` (the default),
    /// the server refuses to start if no token is provided.
    pub auth_token: Option<String>,
    /// Require a non-empty `auth_token` before the server will start.
    ///
    /// Defaults to `true`. When `true` and `auth_token` is `None` the
    /// `serve` / `serve_with_shutdown` methods return an error immediately
    /// rather than starting an unauthenticated server. Set to `false` only
    /// for local development or test environments where network-level
    /// isolation makes token auth unnecessary.
    pub require_auth: bool,
}

impl std::fmt::Debug for EvnodeServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvnodeServerConfig")
            .field("addr", &self.addr)
            .field("enable_gzip", &self.enable_gzip)
            .field("max_message_size", &self.max_message_size)
            .field("executor_config", &self.executor_config)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field("require_auth", &self.require_auth)
            .finish()
    }
}

impl Default for EvnodeServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:50051".parse().unwrap(),
            enable_gzip: true,
            max_message_size: 4 * 1024 * 1024, // 4MB
            executor_config: ExecutorServiceConfig::default(),
            auth_token: std::env::var("EVOLVE_EVNODE_AUTH_TOKEN")
                .ok()
                .filter(|t| !t.trim().is_empty()),
            require_auth: true,
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

    /// Build the [`ExecutorServiceImpl`] from the current server state, consuming `self`.
    fn build_service(self) -> (EvnodeServerConfig, ExecutorServiceImpl<Stf, S, Codes>) {
        let config = self.config;
        let mut service = match self.mempool {
            Some(mempool) => ExecutorServiceImpl::with_mempool(
                self.stf,
                self.storage,
                self.codes,
                config.executor_config.clone(),
                mempool,
            ),
            None => ExecutorServiceImpl::new(
                self.stf,
                self.storage,
                self.codes,
                config.executor_config.clone(),
            ),
        };

        if let Some(callback) = self.on_state_change {
            service = service.with_state_change_callback(callback);
        }

        if let Some(callback) = self.on_block_executed {
            service = service.with_on_block_executed(callback);
        }

        (config, service)
    }

    /// Apply message size and compression settings to a raw service, returning
    /// a fully-configured [`ExecutorServiceServer`].
    fn apply_settings(
        service: ExecutorServiceImpl<Stf, S, Codes>,
        max_message_size: usize,
        enable_gzip: bool,
    ) -> ExecutorServiceServer<ExecutorServiceImpl<Stf, S, Codes>> {
        let mut server = ExecutorServiceServer::new(service)
            .max_decoding_message_size(max_message_size)
            .max_encoding_message_size(max_message_size);
        if enable_gzip {
            server = server
                .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                .send_compressed(tonic::codec::CompressionEncoding::Gzip);
        }
        server
    }

    /// Start the gRPC server.
    ///
    /// Returns an error if `require_auth` is `true` and no `auth_token` is
    /// configured. This is a fail-fast guard against accidentally exposing the
    /// privileged execution interface without authentication.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (config, service) = self.build_service();

        if config.require_auth && config.auth_token.is_none() {
            return Err(
                "EVNode server refuses to start: require_auth is true but no auth_token is set. \
                 Provide a token via EVOLVE_EVNODE_AUTH_TOKEN or EvnodeServerConfig::auth_token, \
                 or set require_auth = false for dev/test environments."
                    .into(),
            );
        }

        let addr = config.addr;
        tracing::info!("Starting EVNode gRPC server on {}", addr);
        let server = Self::apply_settings(service, config.max_message_size, config.enable_gzip);

        if let Some(token) = config.auth_token {
            tracing::info!(
                "EVNode auth token is configured; unauthenticated calls will be rejected"
            );
            let svc = InterceptedService::new(server, make_auth_interceptor(token));
            Server::builder().add_service(svc).serve(addr).await?;
        } else {
            tracing::warn!(
                "EVNode server starting WITHOUT auth token — \
                 set EVOLVE_EVNODE_AUTH_TOKEN or config.auth_token for production use"
            );
            Server::builder().add_service(server).serve(addr).await?;
        }

        Ok(())
    }

    /// Start the gRPC server with graceful shutdown.
    ///
    /// The server will shut down when the provided signal is received.
    ///
    /// Returns an error if `require_auth` is `true` and no `auth_token` is
    /// configured. See [`serve`](Self::serve) for details.
    pub async fn serve_with_shutdown<F>(
        self,
        signal: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send,
    {
        let (config, service) = self.build_service();

        if config.require_auth && config.auth_token.is_none() {
            return Err(
                "EVNode server refuses to start: require_auth is true but no auth_token is set. \
                 Provide a token via EVOLVE_EVNODE_AUTH_TOKEN or EvnodeServerConfig::auth_token, \
                 or set require_auth = false for dev/test environments."
                    .into(),
            );
        }

        let addr = config.addr;
        tracing::info!("Starting EVNode gRPC server on {}", addr);
        let server = Self::apply_settings(service, config.max_message_size, config.enable_gzip);

        if let Some(token) = config.auth_token {
            tracing::info!(
                "EVNode auth token is configured; unauthenticated calls will be rejected"
            );
            let svc = InterceptedService::new(server, make_auth_interceptor(token));
            Server::builder()
                .add_service(svc)
                .serve_with_shutdown(addr, signal)
                .await?;
        } else {
            tracing::warn!(
                "EVNode server starting WITHOUT auth token — \
                 set EVOLVE_EVNODE_AUTH_TOKEN or config.auth_token for production use"
            );
            Server::builder()
                .add_service(server)
                .serve_with_shutdown(addr, signal)
                .await?;
        }

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
