//! P2P channel definitions for the Evolve protocol.
//!
//! Commonware P2P uses numbered channels with per-channel rate limits.
//! Each channel is independent — rate limiting and message sizing are
//! configured per channel so that e.g. large block transfers don't starve
//! consensus vote messages.

/// Channel ID constants for the Evolve P2P protocol.
///
/// Each channel has its own rate limit quota and message capacity.
/// Consensus channels (0–2) are used by simplex BFT.
/// Application channels (3–4) are Evolve-specific.
pub mod channel {
    /// Consensus vote messages (notarize, finalize, nullify).
    /// High rate, small messages (~200 bytes).
    pub const VOTES: u32 = 0;

    /// Consensus certificates (notarization, finalization, nullification).
    /// High rate, medium messages (~2 KB with aggregated sigs).
    pub const CERTIFICATES: u32 = 1;

    /// Block resolution requests/responses.
    /// Medium rate, large messages (full block data).
    pub const RESOLVER: u32 = 2;

    /// Block broadcast from proposer to validators.
    /// Low rate (1 per view), large messages.
    pub const BLOCK_BROADCAST: u32 = 3;

    /// Transaction gossip between validators.
    /// Medium rate, medium messages (encoded transactions).
    ///
    /// NOTE: This channel is Evolve-specific — Alto doesn't have it because
    /// Alto has no transaction concept.
    pub const TX_GOSSIP: u32 = 4;

    /// Total number of channels.
    pub const COUNT: u32 = 5;
}

/// Rate limiting configuration for a single P2P channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Sustained throughput: messages allowed per second.
    pub messages_per_second: u32,
    /// Burst capacity: maximum message spike above the sustained rate.
    pub burst: u32,
}

/// Configuration for a single P2P channel.
#[derive(Clone, Debug)]
pub struct ChannelConfig {
    /// Channel identifier. Must match one of the constants in [`channel`].
    pub id: u32,
    /// Maximum encoded size of a single message on this channel (bytes).
    pub max_message_size: usize,
    /// Rate limiting parameters for this channel.
    pub rate_limit: RateLimit,
}

/// Default channel configurations for all Evolve P2P channels.
///
/// Values are conservative starting points. Production deployments should
/// tune `max_message_size` and `rate_limit` based on observed traffic.
pub fn default_channel_configs() -> Vec<ChannelConfig> {
    vec![
        ChannelConfig {
            id: channel::VOTES,
            max_message_size: 512,
            rate_limit: RateLimit {
                messages_per_second: 100,
                burst: 200,
            },
        },
        ChannelConfig {
            id: channel::CERTIFICATES,
            max_message_size: 4_096,
            rate_limit: RateLimit {
                messages_per_second: 100,
                burst: 200,
            },
        },
        ChannelConfig {
            id: channel::RESOLVER,
            max_message_size: 4_194_304, // 4 MiB
            rate_limit: RateLimit {
                messages_per_second: 10,
                burst: 20,
            },
        },
        ChannelConfig {
            id: channel::BLOCK_BROADCAST,
            max_message_size: 4_194_304, // 4 MiB
            rate_limit: RateLimit {
                messages_per_second: 5,
                burst: 10,
            },
        },
        ChannelConfig {
            id: channel::TX_GOSSIP,
            max_message_size: 65_536, // 64 KiB
            rate_limit: RateLimit {
                messages_per_second: 50,
                burst: 100,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_ids_are_unique_and_sequential() {
        assert_eq!(channel::VOTES, 0);
        assert_eq!(channel::CERTIFICATES, 1);
        assert_eq!(channel::RESOLVER, 2);
        assert_eq!(channel::BLOCK_BROADCAST, 3);
        assert_eq!(channel::TX_GOSSIP, 4);
        assert_eq!(channel::COUNT, 5);
    }

    #[test]
    fn default_configs_cover_all_channels() {
        let configs = default_channel_configs();
        assert_eq!(configs.len(), channel::COUNT as usize);

        let mut ids: Vec<u32> = configs.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let expected: Vec<u32> = (0..channel::COUNT).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn default_configs_have_positive_limits() {
        for cfg in default_channel_configs() {
            assert!(
                cfg.max_message_size > 0,
                "channel {} has zero max_message_size",
                cfg.id
            );
            assert!(
                cfg.rate_limit.messages_per_second > 0,
                "channel {} has zero messages_per_second",
                cfg.id
            );
            assert!(
                cfg.rate_limit.burst >= cfg.rate_limit.messages_per_second,
                "channel {} burst is less than messages_per_second",
                cfg.id
            );
        }
    }
}
