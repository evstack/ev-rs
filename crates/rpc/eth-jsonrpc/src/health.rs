//! Health status reporting for node monitoring.

use serde::{Deserialize, Serialize};

/// Health status of the node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Whether the node is healthy and ready to serve requests.
    pub healthy: bool,
    /// Whether the node is synced with the network.
    pub synced: bool,
    /// Current block height.
    pub block_height: u64,
    /// Number of connected peers.
    pub peers: u64,
    /// Node version string.
    pub version: String,
    /// Chain ID.
    pub chain_id: u64,
}

impl HealthStatus {
    /// Create a healthy status with the given parameters.
    pub fn healthy(block_height: u64, peers: u64, version: String, chain_id: u64) -> Self {
        Self {
            healthy: true,
            synced: true,
            block_height,
            peers,
            version,
            chain_id,
        }
    }

    /// Create an unhealthy status indicating the node is not ready.
    pub fn unhealthy(version: String, chain_id: u64, reason: &str) -> Self {
        Self {
            healthy: false,
            synced: false,
            block_height: 0,
            peers: 0,
            version: format!("{} ({})", version, reason),
            chain_id,
        }
    }
}

/// Trait for providing health information.
///
/// Implement this to provide custom health checks.
pub trait HealthProvider: Send + Sync {
    /// Get the current health status.
    fn health_status(&self) -> HealthStatus;
}
