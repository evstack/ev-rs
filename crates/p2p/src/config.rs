//! Network configuration for Evolve's P2P layer.

use std::{net::SocketAddr, time::Duration};

use crate::channels::{default_channel_configs, ChannelConfig};

/// Network configuration for Evolve's authenticated P2P layer.
///
/// Passed to the Commonware P2P bootstrapper at startup. All fields have
/// sensible defaults via [`Default`]; override only what your deployment
/// requires.
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// Address to listen on for incoming P2P connections.
    pub listen_addr: SocketAddr,

    /// Bootstrapper/seed node addresses for initial peer discovery.
    ///
    /// Leave empty for single-node dev setups. Production nodes should list
    /// at least one stable seed address.
    pub bootstrappers: Vec<SocketAddr>,

    /// Per-channel rate limiting and message size configuration.
    ///
    /// Defaults to [`default_channel_configs`] covering all five Evolve channels.
    pub channel_configs: Vec<ChannelConfig>,

    /// Maximum number of concurrent peer connections to maintain.
    pub max_peers: usize,

    /// Timeout for establishing a new peer connection.
    pub connection_timeout: Duration,

    /// Signing namespace for domain separation.
    ///
    /// Prevents cross-chain message replay. Must be unique per network.
    pub namespace: Vec<u8>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9000".parse().expect("static addr is valid"),
            bootstrappers: vec![],
            channel_configs: default_channel_configs(),
            max_peers: 50,
            connection_timeout: Duration::from_secs(10),
            namespace: b"_EVOLVE".to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::channel;
    use std::time::Duration;

    #[test]
    fn default_config_invariants() {
        let cfg = NetworkConfig::default();

        assert_eq!(cfg.listen_addr.port(), 9000);
        assert!(cfg.bootstrappers.is_empty());
        assert_eq!(cfg.channel_configs.len(), channel::COUNT as usize);
        assert_eq!(cfg.namespace, b"_EVOLVE");
        assert!(cfg.max_peers > 0);
        assert!(cfg.connection_timeout > Duration::ZERO);
    }

    #[test]
    fn custom_config_roundtrip() {
        let addr: SocketAddr = "127.0.0.1:19000".parse().unwrap();
        let seed: SocketAddr = "10.0.0.1:9000".parse().unwrap();
        let cfg = NetworkConfig {
            listen_addr: addr,
            bootstrappers: vec![seed],
            max_peers: 10,
            namespace: b"_TEST".to_vec(),
            ..NetworkConfig::default()
        };
        assert_eq!(cfg.listen_addr, addr);
        assert_eq!(cfg.bootstrappers.len(), 1);
        assert_eq!(cfg.max_peers, 10);
        assert_eq!(cfg.namespace, b"_TEST");
    }
}
