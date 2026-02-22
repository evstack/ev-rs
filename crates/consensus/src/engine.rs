//! Simplex consensus engine configuration and setup.
//!
//! Configures `commonware_consensus::simplex::Engine` to work with Evolve's
//! automaton, relay, and reporter types. This module provides [`SimplexSetup`]
//! which builds the engine from a signing scheme, leader elector, and Evolve
//! application components.

use crate::config::ConsensusConfig;
use commonware_consensus::simplex;
use commonware_consensus::simplex::elector::{Config as ElectorConfig, RoundRobin};
use commonware_consensus::simplex::types::{Activity, Context};
use commonware_consensus::{CertifiableAutomaton, Relay, Reporter};
use commonware_cryptography::certificate::Scheme;
use commonware_cryptography::Digest;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_runtime::buffer::paged::CacheRef;
use commonware_runtime::{Clock, Handle, Metrics, Spawner, Storage};
use rand_core::CryptoRngCore;
use std::num::{NonZeroU16, NonZeroUsize};
use std::time::Duration;

/// Convenience re-export: the simplex `scheme::Scheme<D>` super-trait that binds
/// a `certificate::Scheme` to simplex's `Subject` type.
pub use commonware_consensus::simplex::scheme::Scheme as SimplexScheme;

/// Configuration and factory for the simplex consensus engine.
///
/// Captures the configuration needed to build a simplex engine instance.
/// Call [`build`](Self::build) to construct the engine, then
/// [`Engine::start`](simplex::Engine::start) to begin consensus.
///
/// # Type Parameters
///
/// * `S` - Signing scheme (e.g., `ed25519::Scheme`, `bls12381_threshold::standard::Scheme`)
/// * `L` - Leader elector config (e.g., `RoundRobin`)
pub struct SimplexSetup<S, L = RoundRobin> {
    config: ConsensusConfig,
    scheme: S,
    elector: L,
}

impl<S: Scheme> SimplexSetup<S, RoundRobin> {
    /// Create a new simplex setup with round-robin leader election.
    pub fn new(config: ConsensusConfig, scheme: S) -> Self {
        Self {
            config,
            scheme,
            elector: RoundRobin::default(),
        }
    }
}

impl<S: Scheme, L: Clone> SimplexSetup<S, L> {
    /// Set a custom leader elector configuration.
    pub fn with_elector<L2>(self, elector: L2) -> SimplexSetup<S, L2> {
        SimplexSetup {
            config: self.config,
            scheme: self.scheme,
            elector,
        }
    }

    /// Build the simplex engine with the given runtime context and consensus components.
    ///
    /// Returns the engine ready to be started with
    /// [`Engine::start`](simplex::Engine::start).
    #[allow(clippy::too_many_arguments)]
    pub fn build<E, D, B, A, R, F>(
        self,
        context: E,
        automaton: A,
        relay: R,
        reporter: F,
        blocker: B,
    ) -> simplex::Engine<E, S, L, B, D, A, R, F, Sequential>
    where
        E: Clock + CryptoRngCore + Spawner + Storage + Metrics,
        D: Digest,
        S: SimplexScheme<D>,
        L: ElectorConfig<S>,
        B: Blocker<PublicKey = S::PublicKey>,
        A: CertifiableAutomaton<Context = Context<D, S::PublicKey>, Digest = D>,
        R: Relay<Digest = D>,
        F: Reporter<Activity = Activity<S, D>>,
    {
        // Convert activity_timeout from Duration to ViewDelta (seconds).
        let activity_timeout_secs = self.config.activity_timeout.as_secs();
        let activity_timeout =
            commonware_consensus::types::ViewDelta::new(activity_timeout_secs.max(1));

        let partition = String::from_utf8(self.config.namespace.clone())
            .unwrap_or_else(|_| "evolve".to_string());

        let cfg = simplex::Config {
            scheme: self.scheme,
            elector: self.elector,
            blocker,
            automaton,
            relay,
            reporter,
            strategy: Sequential,
            partition,
            mailbox_size: 1024,
            epoch: commonware_consensus::types::Epoch::new(0),
            replay_buffer: NonZeroUsize::new(65536).unwrap(),
            write_buffer: NonZeroUsize::new(4096).unwrap(),
            page_cache: CacheRef::new(
                NonZeroU16::new(4096).unwrap(),   // page_size
                NonZeroUsize::new(1024).unwrap(), // cache_size
            ),
            leader_timeout: self.config.leader_timeout,
            notarization_timeout: self.config.notarization_timeout,
            nullify_retry: Duration::from_secs(1),
            activity_timeout,
            skip_timeout: commonware_consensus::types::ViewDelta::new(5),
            fetch_timeout: Duration::from_secs(5),
            fetch_concurrent: 3,
        };

        simplex::Engine::new(context, cfg)
    }
}

/// Start a simplex engine with the given P2P channel pairs.
///
/// Convenience function that calls `engine.start()` with the three required
/// network channel pairs (votes, certificates, resolver).
pub fn start_engine<E, S, L, B, D, A, R, F>(
    engine: simplex::Engine<E, S, L, B, D, A, R, F, Sequential>,
    vote_network: (
        impl Sender<PublicKey = S::PublicKey>,
        impl Receiver<PublicKey = S::PublicKey>,
    ),
    certificate_network: (
        impl Sender<PublicKey = S::PublicKey>,
        impl Receiver<PublicKey = S::PublicKey>,
    ),
    resolver_network: (
        impl Sender<PublicKey = S::PublicKey>,
        impl Receiver<PublicKey = S::PublicKey>,
    ),
) -> Handle<()>
where
    E: Clock + CryptoRngCore + Spawner + Storage + Metrics,
    S: SimplexScheme<D>,
    L: ElectorConfig<S>,
    B: Blocker<PublicKey = S::PublicKey>,
    D: Digest,
    A: CertifiableAutomaton<Context = Context<D, S::PublicKey>, Digest = D>,
    R: Relay<Digest = D>,
    F: Reporter<Activity = Activity<S, D>>,
{
    engine.start(vote_network, certificate_network, resolver_network)
}
