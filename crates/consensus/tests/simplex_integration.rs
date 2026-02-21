//! Integration tests for the simplex consensus engine wiring.
//!
//! Validates that [`SimplexSetup`] correctly configures and starts a simplex
//! engine using ed25519 signatures, RoundRobin leader election, and the
//! commonware simulated P2P network.

use commonware_consensus::simplex::elector::RoundRobin;
use commonware_consensus::simplex::mocks;
use commonware_consensus::simplex::scheme::ed25519;
use commonware_consensus::types::View;
use commonware_consensus::Monitor;
use commonware_cryptography::Sha256;
use commonware_p2p::simulated::{Config as NetConfig, Network};
use commonware_runtime::{deterministic, Metrics, Quota, Runner};
use evolve_consensus::engine::SimplexSetup;
use evolve_consensus::ConsensusConfig;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

/// Single-validator consensus: one validator starts consensus, proposes
/// blocks, and finalizes them via [`SimplexSetup`].
///
/// Verifies:
/// - The engine starts successfully via SimplexSetup::build
/// - The automaton's propose() is called (blocks are produced)
/// - Blocks are finalized (reporter receives finalization events)
/// - No faults are detected
#[test]
fn single_validator_consensus_produces_and_finalizes_blocks() {
    let executor = deterministic::Runner::timed(Duration::from_secs(120));
    executor.start(|mut context| async move {
        let namespace = b"evolve_test".to_vec();
        let n = 1u32;

        // Create simulated P2P network.
        let (network, oracle) = Network::new(
            context.with_label("network"),
            NetConfig {
                max_size: 1024 * 1024,
                disconnect_on_block: true,
                tracked_peer_sets: None,
            },
        );
        network.start();

        // Generate ed25519 fixture for 1 validator.
        let fixture = ed25519::fixture(&mut context, &namespace, n);
        let validator = fixture.participants[0].clone();

        // Register validator channels on simulated network.
        let control = oracle.control(validator.clone());
        let quota = Quota::per_second(NonZeroU32::MAX);
        let vote_network = control.register(0, quota).await.unwrap();
        let certificate_network = control.register(1, quota).await.unwrap();
        let resolver_network = control.register(2, quota).await.unwrap();

        // Create mock relay, application, and reporter.
        let relay = Arc::new(mocks::relay::Relay::new());
        let app_cfg = mocks::application::Config {
            hasher: Sha256::default(),
            relay: relay.clone(),
            me: validator.clone(),
            propose_latency: (5.0, 1.0),
            verify_latency: (5.0, 1.0),
            certify_latency: (5.0, 1.0),
            should_certify: mocks::application::Certifier::Always,
        };
        let (application, mailbox) =
            mocks::application::Application::new(context.with_label("application"), app_cfg);
        application.start();

        let elector = RoundRobin::<Sha256>::default();
        let reporter_cfg = mocks::reporter::Config {
            participants: fixture.participants.clone().try_into().expect("non-empty"),
            scheme: fixture.schemes[0].clone(),
            elector: elector.clone(),
        };
        let mut reporter =
            mocks::reporter::Reporter::new(context.with_label("reporter"), reporter_cfg);

        // Subscribe to finalization events BEFORE starting the engine.
        let (mut latest, mut monitor): (View, _) = reporter.subscribe().await;

        // Use SimplexSetup to build the engine -- this is the code under test.
        let config = ConsensusConfig {
            chain_id: 1,
            namespace: namespace.clone(),
            gas_limit: 30_000_000,
            leader_timeout: Duration::from_secs(1),
            notarization_timeout: Duration::from_secs(2),
            activity_timeout: Duration::from_secs(10),
            epoch_length: 100,
        };
        let setup = SimplexSetup::new(config, fixture.schemes[0].clone());
        let engine = setup.build(
            context.with_label("engine"),
            mailbox.clone(), // automaton
            mailbox,         // relay (Mailbox implements both)
            reporter.clone(),
            oracle.control(validator.clone()), // blocker
        );

        // Start the engine with P2P channels.
        let _handle = engine.start(vote_network, certificate_network, resolver_network);

        // Wait for at least 3 finalized views.
        let target = View::new(3);
        while latest < target {
            latest = monitor.recv().await.expect("monitor channel closed");
        }

        // Verify: we reached the target view through the monitor.
        assert!(
            latest >= target,
            "expected finalization to reach view {target:?}, got {latest:?}"
        );

        // Verify: no faults detected.
        let faults = reporter.faults.lock().unwrap();
        assert!(faults.is_empty(), "unexpected faults detected");

        // Verify: no invalid signatures.
        let invalid = *reporter.invalid.lock().unwrap();
        assert_eq!(invalid, 0, "unexpected invalid signatures: {invalid}");
    });
}

/// Verify that ConsensusRunner wires the engine correctly.
///
/// This test validates the higher-level ConsensusRunner API produces
/// a running consensus engine.
#[test]
fn consensus_runner_starts_engine() {
    let executor = deterministic::Runner::timed(Duration::from_secs(120));
    executor.start(|mut context| async move {
        let namespace = b"runner_test".to_vec();
        let n = 1u32;

        // Create simulated P2P network.
        let (network, oracle) = Network::new(
            context.with_label("network"),
            NetConfig {
                max_size: 1024 * 1024,
                disconnect_on_block: true,
                tracked_peer_sets: None,
            },
        );
        network.start();

        // Generate fixture.
        let fixture = ed25519::fixture(&mut context, &namespace, n);
        let validator = fixture.participants[0].clone();

        // Register channels.
        let control = oracle.control(validator.clone());
        let quota = Quota::per_second(NonZeroU32::MAX);
        let vote_network = control.register(0, quota).await.unwrap();
        let certificate_network = control.register(1, quota).await.unwrap();
        let resolver_network = control.register(2, quota).await.unwrap();

        // Create mock components.
        let relay = Arc::new(mocks::relay::Relay::new());
        let app_cfg = mocks::application::Config {
            hasher: Sha256::default(),
            relay: relay.clone(),
            me: validator.clone(),
            propose_latency: (5.0, 1.0),
            verify_latency: (5.0, 1.0),
            certify_latency: (5.0, 1.0),
            should_certify: mocks::application::Certifier::Always,
        };
        let (application, mailbox) =
            mocks::application::Application::new(context.with_label("application"), app_cfg);
        application.start();

        let elector = RoundRobin::<Sha256>::default();
        let reporter_cfg = mocks::reporter::Config {
            participants: fixture.participants.clone().try_into().expect("non-empty"),
            scheme: fixture.schemes[0].clone(),
            elector: elector.clone(),
        };
        let mut reporter =
            mocks::reporter::Reporter::new(context.with_label("reporter"), reporter_cfg);

        let (mut latest, mut monitor): (View, _) = reporter.subscribe().await;

        // Use ConsensusRunner -- the higher-level API under test.
        let consensus_config = ConsensusConfig::default();
        let network_config = evolve_p2p::NetworkConfig::default();
        let runner = evolve_consensus::ConsensusRunner::new(consensus_config, network_config);
        let _handle = runner.start(
            context.with_label("runner_engine"),
            mailbox.clone(),
            mailbox,
            reporter.clone(),
            fixture.schemes[0].clone(),
            oracle.control(validator.clone()),
            vote_network,
            certificate_network,
            resolver_network,
        );

        // Wait for finalization progress.
        let target = View::new(2);
        while latest < target {
            latest = monitor.recv().await.expect("monitor channel closed");
        }

        // Verify: we reached the target view through the monitor.
        assert!(
            latest >= target,
            "expected finalization to reach view {target:?}, got {latest:?}"
        );
    });
}
