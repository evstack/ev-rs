//! Marshal wiring for ordered finalized block delivery.
//!
//! The marshal sits between the simplex consensus engine and the application
//! reporter. It receives out-of-order finalization events from consensus,
//! reconstructs total order, and delivers blocks sequentially to the
//! application.
//!
//! # Architecture
//!
//! ```text
//! Simplex Engine
//!     │ (Activity<S, D>)
//!     ▼
//! Marshal Mailbox (implements Reporter)
//!     │
//!     ▼
//! Marshal Actor (archives + reordering)
//!     │ (Update<B>)
//!     ▼
//! Application Reporter
//! ```
//!
//! # Usage
//!
//! 1. Create archive stores for certificates and blocks
//! 2. Call [`MarshalActor::init`] with the stores and a [`MarshalConfig`]
//! 3. Pass the returned [`MarshalMailbox`] as the `reporter` to
//!    [`SimplexSetup::build`](crate::engine::SimplexSetup::build)
//! 4. Call [`MarshalActor::start`] with your application reporter, broadcast
//!    buffer, and resolver
//!
//! See [`init_marshal_config`] for creating a config with sensible defaults.

use commonware_consensus::types::{Epoch, ViewDelta};
use commonware_cryptography::certificate::{ConstantProvider, Scheme};
use commonware_parallel::Sequential;
use commonware_runtime::buffer::paged::CacheRef;
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};

/// Re-export marshal types needed for wiring.
pub use commonware_consensus::marshal::{
    resolver, Actor as MarshalActor, Config as MarshalActorConfig, Mailbox as MarshalMailbox,
    Update,
};
pub use commonware_consensus::types::FixedEpocher;
pub use commonware_storage::archive::immutable;

/// Configuration for initializing the marshal subsystem.
///
/// Provides sensible defaults for storage partitioning, buffer sizes,
/// and epoch boundaries. Uses `NonZero` types to make invalid states
/// unrepresentable.
pub struct MarshalConfig {
    /// Prefix for storage partition names (e.g., "evolve-validator-0").
    pub partition_prefix: String,
    /// Number of blocks per epoch for the fixed epocher. Must be non-zero.
    pub epoch_length: NonZeroU64,
    /// Size of the marshal's internal mailbox.
    pub mailbox_size: usize,
    /// How many views to retain before pruning.
    pub view_retention: u64,
    /// Maximum concurrent repair requests. Must be non-zero.
    pub max_repair: NonZeroUsize,
}

impl Default for MarshalConfig {
    fn default() -> Self {
        Self {
            partition_prefix: "evolve".to_string(),
            epoch_length: NonZeroU64::new(100).unwrap(),
            mailbox_size: 1024,
            view_retention: 10,
            max_repair: NonZeroUsize::new(10).unwrap(),
        }
    }
}

/// Create a marshal [`MarshalActorConfig`] from a [`MarshalConfig`] and signing scheme.
///
/// Uses [`ConstantProvider`] (same scheme for all epochs) and [`FixedEpocher`]
/// (fixed-length epochs). The block codec config defaults to `()`.
///
/// # Type Parameters
///
/// * `S` - Certificate scheme (e.g., `ed25519::Scheme`, `bls12381::Scheme`)
/// * `B` - Block type (e.g., [`ConsensusBlock<Tx>`](crate::ConsensusBlock))
pub fn init_marshal_config<S, B>(
    config: &MarshalConfig,
    scheme: S,
    block_codec_config: <B as commonware_codec::Read>::Cfg,
) -> MarshalActorConfig<B, ConstantProvider<S, Epoch>, FixedEpocher, Sequential>
where
    S: Scheme,
    B: commonware_consensus::Block,
{
    MarshalActorConfig {
        provider: ConstantProvider::<S, Epoch>::new(scheme),
        epocher: FixedEpocher::new(config.epoch_length),
        partition_prefix: config.partition_prefix.clone(),
        mailbox_size: config.mailbox_size,
        view_retention_timeout: ViewDelta::new(config.view_retention),
        prunable_items_per_section: NonZeroU64::new(10).unwrap(),
        page_cache: CacheRef::new(
            NonZeroU16::new(4096).unwrap(),
            NonZeroUsize::new(1024).unwrap(),
        ),
        replay_buffer: NonZeroUsize::new(1024).unwrap(),
        key_write_buffer: NonZeroUsize::new(1024).unwrap(),
        value_write_buffer: NonZeroUsize::new(1024).unwrap(),
        block_codec_config,
        max_repair: config.max_repair,
        strategy: Sequential,
    }
}

/// Create an [`immutable::Config`] for a marshal archive store.
///
/// Builds the 17-field config required by [`immutable::Archive::init`]
/// with standard settings for the given partition prefix and codec config.
pub fn archive_config<C: Clone>(
    partition_prefix: &str,
    store_name: &str,
    codec_config: C,
) -> immutable::Config<C> {
    let prefix = format!("{}-{}", partition_prefix, store_name);
    let page_cache = CacheRef::new(
        NonZeroU16::new(4096).unwrap(),
        NonZeroUsize::new(1024).unwrap(),
    );

    immutable::Config {
        metadata_partition: format!("{prefix}-metadata"),
        freezer_table_partition: format!("{prefix}-freezer-table"),
        freezer_table_initial_size: 64,
        freezer_table_resize_frequency: 10,
        freezer_table_resize_chunk_size: 10,
        freezer_key_partition: format!("{prefix}-freezer-key"),
        freezer_key_page_cache: page_cache,
        freezer_value_partition: format!("{prefix}-freezer-value"),
        freezer_value_target_size: 4096,
        freezer_value_compression: None,
        ordinal_partition: format!("{prefix}-ordinal"),
        items_per_section: NonZeroU64::new(10).unwrap(),
        codec_config,
        replay_buffer: NonZeroUsize::new(1024).unwrap(),
        freezer_key_write_buffer: NonZeroUsize::new(1024).unwrap(),
        freezer_value_write_buffer: NonZeroUsize::new(1024).unwrap(),
        ordinal_write_buffer: NonZeroUsize::new(1024).unwrap(),
    }
}

/// Create a resolver [`Config`](resolver::p2p::Config) for the marshal's P2P block fetcher.
pub fn resolver_config<P, C, B>(
    public_key: P,
    provider: C,
    blocker: B,
    mailbox_size: usize,
) -> resolver::p2p::Config<P, C, B>
where
    P: commonware_cryptography::PublicKey,
    C: commonware_p2p::Provider<PublicKey = P>,
    B: commonware_p2p::Blocker<PublicKey = P>,
{
    resolver::p2p::Config {
        public_key,
        provider,
        blocker,
        mailbox_size,
        initial: std::time::Duration::from_secs(1),
        timeout: std::time::Duration::from_secs(2),
        fetch_retry_timeout: std::time::Duration::from_millis(100),
        priority_requests: false,
        priority_responses: false,
    }
}
