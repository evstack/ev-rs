//! Consensus lifecycle orchestrator.
//!
//! [`ConsensusRunner`] manages the full lifecycle of consensus + P2P subsystems:
//! network initialization, engine startup, and graceful shutdown.

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
    /// Builds a simplex engine from the provided components, then starts it
    /// with the given P2P channel pairs.
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
