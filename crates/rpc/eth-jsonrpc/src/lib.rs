//! Ethereum-compatible JSON-RPC server for the Evolve execution client.
//!
//! This crate provides a JSON-RPC server that implements the Ethereum JSON-RPC
//! specification, allowing standard wallets and tooling to interact with Evolve.
//!
//! # Example
//!
//! ```rust,no_run
//! use evolve_eth_jsonrpc::{RpcServerConfig, NoopStateProvider, start_server};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = RpcServerConfig {
//!         chain_id: 1,
//!         ..Default::default()
//!     };
//!     let handle = start_server(config, NoopStateProvider).await.unwrap();
//!     handle.stopped().await;
//! }
//! ```

pub mod api;
pub mod error;
pub mod health;
pub mod metrics_middleware;
pub mod server;
pub mod subscriptions;

// Re-export key types for convenience
pub use api::{EthApiServer, EthPubSubApiServer, NetApiServer, Web3ApiServer};
pub use error::{RpcError, RpcResult};
pub use health::HealthStatus;
pub use metrics_middleware::{encode_metrics, MetricsLayer, MetricsService};
pub use server::{
    start_server, start_server_with_subscriptions, EthRpcServer, NoopStateProvider,
    RpcServerConfig, StateProvider, CLIENT_VERSION,
};
pub use subscriptions::{SharedSubscriptionManager, SubscriptionKind, SubscriptionManager};
