//! P2P networking configuration for Evolve's consensus layer.
//!
//! This crate configures Commonware's authenticated P2P networking
//! for use in the Evolve blockchain. It provides:
//!
//! - Channel constants and rate limit configuration
//! - `NetworkConfig` for bootstrapping the P2P layer
//! - `ValidatorSet` for managing epoch-based validator identities
//! - `EpochPeerProvider` implementing the `Provider` trait for peer management

pub mod channels;
pub mod config;
pub mod provider;
pub mod validator;

pub use channels::{default_channel_configs, ChannelConfig, RateLimit};
pub use config::NetworkConfig;
pub use provider::EpochPeerProvider;
pub use validator::{ValidatorIdentity, ValidatorSet};
