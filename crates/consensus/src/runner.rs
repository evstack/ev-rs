//! Consensus lifecycle orchestrator.
//!
//! [`ConsensusRunner`] manages the full lifecycle of consensus + P2P subsystems:
//! network initialization, engine startup, and graceful shutdown.
//!
//! # With Marshal (Production)
//!
//! For ordered finalized block delivery, initialize the marshal via
//! [`crate::marshal`] and pass the [`MarshalMailbox`](crate::MarshalMailbox)
//! as the `reporter` parameter to [`start`](ConsensusRunner::start). The
//! marshal actor then delivers blocks in sequential height order to the
//! application reporter. See [`crate::marshal`] for the full wiring pattern.
//!
//! # Without Marshal (Testing)
//!
//! For testing, pass any `Reporter` implementation directly — the simplex
//! engine will report raw activity events without ordering guarantees.

use crate::config::ConsensusConfig;
use crate::engine::{SimplexScheme, SimplexSetup};
use commonware_consensus::simplex::elector::{Config as ElectorConfig, RoundRobin};
use commonware_consensus::simplex::types::{Activity, Context};
use commonware_consensus::{CertifiableAutomaton, Relay, Reporter};
use commonware_cryptography::Digest;
use commonware_p2p::{Blocker, Receiver, Sender};
use commonware_runtime::{Clock, Handle, Metrics, Spawner, Storage};
use evolve_p2p::NetworkConfig;
use rand_core::CryptoRngCore;

/// Orchestrates the full consensus lifecycle.
///
/// Wires together P2P networking and the simplex consensus engine,
/// managing startup and graceful shutdown of all subsystems.
///
/// # Lifecycle
///
/// 1. Create the runner with consensus and network configuration.
/// 2. Call [`start`](Self::start) with the runtime context, automaton, scheme,
///    blocker, and network channels.
/// 3. The returned [`Handle`] completes when the runtime signals shutdown.
///
/// # Marshal Integration
///
/// For production use with ordered block delivery:
///
/// ```rust,ignore
/// use evolve_consensus::marshal::*;
///
/// // 1. Create archive stores
/// let certs = immutable::Archive::init(ctx, archive_config(&prefix, "certs", codec)).await?;
/// let blocks = immutable::Archive::init(ctx, archive_config(&prefix, "blocks", ())).await?;
///
/// // 2. Initialize marshal
/// let marshal_cfg = init_marshal_config::<S, B>(&MarshalConfig::default(), scheme, ());
/// let (actor, mailbox, height) = MarshalActor::init(ctx, certs, blocks, marshal_cfg).await;
///
/// // 3. Pass marshal mailbox as reporter to ConsensusRunner::start()
/// let engine_handle = runner.start(ctx, automaton, relay, mailbox, scheme, blocker, ...);
///
/// // 4. Start marshal actor (delivers ordered blocks to app reporter)
/// let marshal_handle = actor.start(app_reporter, broadcast_buffer, resolver);
/// ```
pub struct ConsensusRunner {
    consensus_config: ConsensusConfig,
    _network_config: NetworkConfig,
}

impl ConsensusRunner {
    /// Create a new consensus runner.
    pub fn new(consensus_config: ConsensusConfig, network_config: NetworkConfig) -> Self {
        Self {
            consensus_config,
            _network_config: network_config,
        }
    }

    /// Start the consensus engine with the given runtime context and components.
    ///
    /// The `reporter` parameter accepts any [`Reporter`] implementation.
    /// For production, pass a [`MarshalMailbox`](crate::MarshalMailbox) to
    /// enable ordered block delivery through the marshal actor.
    /// For testing, pass a mock reporter directly.
    ///
    /// # Returns
    ///
    /// A [`Handle`] that completes when the engine shuts down.
    #[allow(clippy::too_many_arguments)]
    pub fn start<E, S, D, B, A, R, F, VS, VR, CS, CR, RS, RR>(
        &self,
        context: E,
        automaton: A,
        relay: R,
        reporter: F,
        scheme: S,
        blocker: B,
        vote_network: (VS, VR),
        certificate_network: (CS, CR),
        resolver_network: (RS, RR),
    ) -> Handle<()>
    where
        E: Clock + CryptoRngCore + Spawner + Storage + Metrics,
        S: SimplexScheme<D> + Clone,
        D: Digest,
        B: Blocker<PublicKey = S::PublicKey>,
        A: CertifiableAutomaton<Context = Context<D, S::PublicKey>, Digest = D>,
        R: Relay<Digest = D>,
        F: Reporter<Activity = Activity<S, D>>,
        RoundRobin: ElectorConfig<S>,
        VS: Sender<PublicKey = S::PublicKey>,
        VR: Receiver<PublicKey = S::PublicKey>,
        CS: Sender<PublicKey = S::PublicKey>,
        CR: Receiver<PublicKey = S::PublicKey>,
        RS: Sender<PublicKey = S::PublicKey>,
        RR: Receiver<PublicKey = S::PublicKey>,
    {
        let setup = SimplexSetup::new(self.consensus_config.clone(), scheme);
        let engine = setup.build(context, automaton, relay, reporter, blocker);
        engine.start(vote_network, certificate_network, resolver_network)
    }
}
