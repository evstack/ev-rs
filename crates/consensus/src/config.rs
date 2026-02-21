use std::time::Duration;

/// Configuration for the consensus application layer.
#[derive(Debug, Clone)]
pub struct ConsensusConfig {
    /// Chain ID for the evolve network.
    pub chain_id: u64,
    /// Namespace bytes for signing domain separation (e.g., b"_EVOLVE").
    pub namespace: Vec<u8>,
    /// Gas limit per block.
    pub gas_limit: u64,
    /// Timeout before leader rotation.
    pub leader_timeout: Duration,
    /// Timeout for notarization.
    pub notarization_timeout: Duration,
    /// Timeout for activity before penalization.
    pub activity_timeout: Duration,
    /// Number of blocks per epoch.
    pub epoch_length: u64,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            chain_id: 1,
            namespace: b"_EVOLVE".to_vec(),
            gas_limit: 30_000_000,
            leader_timeout: Duration::from_secs(2),
            notarization_timeout: Duration::from_secs(2),
            activity_timeout: Duration::from_secs(10),
            epoch_length: 100,
        }
    }
}
