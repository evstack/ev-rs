//! # Evolve Simulator
//!
//! A deterministic simulation engine for testing the Evolve SDK.
//!
//! ## Features
//!
//! - **Seed-based reproducibility**: Every simulation can be exactly reproduced
//!   by providing the same seed.
//! - **Time acceleration**: Simulate hours of blockchain operation in seconds.
//! - **Fault injection**: Test error handling with configurable storage failures.
//! - **Performance tracking**: Collect detailed metrics about block/tx execution.
//!
//! ## Example
//!
//! ```ignore
//! use evolve_simulator::{Simulator, SimConfig};
//!
//! let config = SimConfig::default();
//! let mut sim = Simulator::new(12345, config);
//!
//! // Run simulation
//! sim.run_blocks(100);
//!
//! // Get performance report
//! let report = sim.generate_report();
//! println!("{}", report.to_string_pretty());
//!
//! // If test fails, print reproduction command
//! println!("Reproduce with: {}", sim.seed_info().reproduction_command());
//! ```

pub mod metrics;
pub mod seed;
pub mod storage;
pub mod time;

pub use metrics::{BlockMetrics, MetricsConfig, PerformanceReport, SimMetrics, TxMetrics};
pub use seed::{SeedInfo, SeededRng};
pub use storage::{SimulatedStorage, StorageConfig, StorageSnapshot, StorageStats};
pub use time::{SimulatedTime, TimeConfig, TimeSnapshot};

use evolve_core::{define_error, ErrorCode, ReadonlyKV};
use evolve_server_core::{StateChange, WritableKV};
use metrics::BlockMetricsBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

define_error!(ERR_SIMULATION_ABORTED, 0x10, "simulation aborted");

/// Configuration for the simulator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimConfig {
    /// Storage configuration (fault injection).
    pub storage: StorageConfig,
    /// Time configuration.
    pub time: TimeConfig,
    /// Metrics configuration.
    pub metrics: MetricsConfig,
    /// Maximum blocks to simulate (0 = unlimited).
    pub max_blocks: u64,
    /// Maximum transactions per block.
    pub max_txs_per_block: usize,
    /// Whether to stop on first error.
    pub stop_on_error: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            storage: StorageConfig::default(),
            time: TimeConfig::default(),
            metrics: MetricsConfig::default(),
            max_blocks: 0,
            max_txs_per_block: 1000,
            stop_on_error: false,
        }
    }
}

impl SimConfig {
    /// Creates a configuration for stress testing with high fault rates.
    pub fn stress_test() -> Self {
        Self {
            storage: StorageConfig::with_faults(0.01, 0.01).with_logging(),
            max_blocks: 10000,
            ..Default::default()
        }
    }

    /// Creates a configuration for deterministic replay without faults.
    pub fn replay() -> Self {
        Self {
            storage: StorageConfig::no_faults(),
            ..Default::default()
        }
    }
}

/// The main deterministic simulator.
///
/// Orchestrates simulation execution with controlled time, storage, and randomness.
pub struct Simulator {
    /// The seed used to initialize this simulation.
    seed: u64,
    /// Random number generator.
    rng: SeededRng,
    /// Simulated time.
    time: SimulatedTime,
    /// Simulated storage.
    storage: SimulatedStorage,
    /// Configuration.
    config: SimConfig,
    /// Collected metrics.
    metrics: SimMetrics,
    /// Whether simulation is running.
    running: bool,
    /// Abort reason if stopped.
    abort_reason: Option<String>,
}

impl Simulator {
    /// Creates a new simulator with the given seed and configuration.
    pub fn new(seed: u64, config: SimConfig) -> Self {
        let mut rng = SeededRng::new(seed);
        let storage_seed = rng.gen();
        let storage = SimulatedStorage::new(storage_seed, config.storage.clone());
        let time = SimulatedTime::new(config.time.clone());
        let metrics = SimMetrics::with_config(config.metrics.clone());

        Self {
            seed,
            rng,
            time,
            storage,
            config,
            metrics,
            running: false,
            abort_reason: None,
        }
    }

    /// Creates a new simulator with a random seed.
    ///
    /// Returns both the simulator and the seed for reproduction.
    pub fn with_random_seed(config: SimConfig) -> (Self, u64) {
        let (rng, seed) = SeededRng::from_entropy();
        let storage_seed = rng.seed().wrapping_add(1);
        let storage = SimulatedStorage::new(storage_seed, config.storage.clone());
        let time = SimulatedTime::new(config.time.clone());
        let metrics = SimMetrics::with_config(config.metrics.clone());

        let sim = Self {
            seed,
            rng,
            time,
            storage,
            config,
            metrics,
            running: false,
            abort_reason: None,
        };

        (sim, seed)
    }

    /// Returns the seed used to initialize this simulation.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns seed information for reproduction.
    pub fn seed_info(&self) -> SeedInfo {
        SeedInfo::new(self.seed)
    }

    /// Returns a reference to the random number generator.
    pub fn rng(&mut self) -> &mut SeededRng {
        &mut self.rng
    }

    /// Returns a reference to the simulated time.
    pub fn time(&self) -> &SimulatedTime {
        &self.time
    }

    /// Returns a mutable reference to the simulated time.
    pub fn time_mut(&mut self) -> &mut SimulatedTime {
        &mut self.time
    }

    /// Returns a reference to the storage.
    pub fn storage(&self) -> &SimulatedStorage {
        &self.storage
    }

    /// Returns a mutable reference to the storage.
    pub fn storage_mut(&mut self) -> &mut SimulatedStorage {
        &mut self.storage
    }

    /// Returns a reference to the metrics.
    pub fn metrics(&self) -> &SimMetrics {
        &self.metrics
    }

    /// Returns the configuration.
    pub fn config(&self) -> &SimConfig {
        &self.config
    }

    /// Returns whether the simulation is currently running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Returns the abort reason if the simulation was stopped.
    pub fn abort_reason(&self) -> Option<&str> {
        self.abort_reason.as_deref()
    }

    /// Advances simulation by one tick.
    pub fn tick(&mut self) {
        self.time.advance_tick();
    }

    /// Advances simulation by multiple ticks.
    pub fn advance_ticks(&mut self, n: u64) {
        self.time.advance_ticks(n);
    }

    /// Advances to the next block.
    pub fn advance_block(&mut self) {
        self.time.advance_block();
    }

    /// Sets the block height directly.
    pub fn set_block_height(&mut self, height: u64) {
        self.time.set_block_height(height);
    }

    /// Records block metrics.
    pub fn record_block_metrics(&mut self, metrics: BlockMetrics) {
        self.metrics.record_block(metrics);
        self.metrics.set_simulated_time_ms(self.time.now_ms());
    }

    /// Creates a new block metrics builder for the current block.
    pub fn start_block(&mut self) -> BlockMetricsBuilder {
        BlockMetricsBuilder::new(self.time.block_height())
    }

    /// Applies state changes to storage.
    pub fn apply_state_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        self.storage.apply_changes(changes)
    }

    /// Generates a performance report.
    pub fn generate_report(&self) -> PerformanceReport {
        self.metrics.generate_report()
    }

    /// Creates a snapshot of the current simulation state.
    pub fn snapshot(&self) -> SimSnapshot {
        SimSnapshot {
            seed: self.seed,
            time: self.time.snapshot(),
            storage: self.storage.snapshot(),
            metrics: self.metrics.snapshot(),
        }
    }

    /// Restores from a snapshot.
    ///
    /// Note: This only restores time and storage state. The RNG state
    /// is not restored, so subsequent random values will differ.
    pub fn restore(&mut self, snapshot: SimSnapshot) {
        self.time.restore(&snapshot.time);
        self.storage.restore(snapshot.storage);
    }

    /// Aborts the simulation with a reason.
    pub fn abort(&mut self, reason: impl Into<String>) {
        self.running = false;
        self.abort_reason = Some(reason.into());
    }

    /// Resets the simulator to initial state (same seed).
    pub fn reset(&mut self) {
        let config = self.config.clone();
        *self = Self::new(self.seed, config);
    }

    /// Runs simulation until a predicate returns true or max_blocks is reached.
    pub fn run_until<F>(&mut self, mut predicate: F) -> Result<(), ErrorCode>
    where
        F: FnMut(&Self) -> bool,
    {
        self.running = true;
        let max_blocks = if self.config.max_blocks > 0 {
            self.config.max_blocks
        } else {
            u64::MAX
        };

        while self.running {
            if self.time.block_height() >= max_blocks {
                self.running = false;
                break;
            }

            if predicate(self) {
                self.running = false;
                break;
            }

            self.advance_block();
        }

        if self.abort_reason.is_some() {
            return Err(ERR_SIMULATION_ABORTED);
        }

        Ok(())
    }

    /// Runs simulation for a specified number of blocks.
    pub fn run_blocks(&mut self, n: u64) -> Result<(), ErrorCode> {
        let target = self.time.block_height().saturating_add(n);
        self.run_until(|sim| sim.time.block_height() >= target)
    }
}

/// A snapshot of simulation state for checkpointing/restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    /// The original seed.
    pub seed: u64,
    /// Time state.
    pub time: TimeSnapshot,
    /// Storage state.
    pub storage: StorageSnapshot,
    /// Metrics snapshot.
    pub metrics: metrics::SimMetricsSnapshot,
}

/// A wrapper around SimulatedStorage that implements ReadonlyKV.
///
/// This is useful for integrating with the STF which expects ReadonlyKV.
pub struct SimStorageAdapter<'a> {
    storage: &'a SimulatedStorage,
}

impl<'a> SimStorageAdapter<'a> {
    pub fn new(storage: &'a SimulatedStorage) -> Self {
        Self { storage }
    }
}

impl ReadonlyKV for SimStorageAdapter<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.storage.get(key)
    }
}

/// Builder for constructing a Simulator with custom configuration.
#[derive(Default)]
pub struct SimulatorBuilder {
    seed: Option<u64>,
    config: SimConfig,
    initial_state: HashMap<Vec<u8>, Vec<u8>>,
}

impl SimulatorBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the seed for the simulation.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Sets the storage configuration.
    pub fn storage_config(mut self, config: StorageConfig) -> Self {
        self.config.storage = config;
        self
    }

    /// Sets the time configuration.
    pub fn time_config(mut self, config: TimeConfig) -> Self {
        self.config.time = config;
        self
    }

    /// Sets the metrics configuration.
    pub fn metrics_config(mut self, config: MetricsConfig) -> Self {
        self.config.metrics = config;
        self
    }

    /// Sets the maximum blocks to simulate.
    pub fn max_blocks(mut self, max: u64) -> Self {
        self.config.max_blocks = max;
        self
    }

    /// Sets the maximum transactions per block.
    pub fn max_txs_per_block(mut self, max: usize) -> Self {
        self.config.max_txs_per_block = max;
        self
    }

    /// Enables stopping on first error.
    pub fn stop_on_error(mut self) -> Self {
        self.config.stop_on_error = true;
        self
    }

    /// Adds initial state to the storage.
    pub fn with_state(mut self, key: Vec<u8>, value: Vec<u8>) -> Self {
        self.initial_state.insert(key, value);
        self
    }

    /// Builds the simulator.
    pub fn build(self) -> Simulator {
        let seed = self.seed.unwrap_or_else(rand::random);
        let mut sim = Simulator::new(seed, self.config);

        // Apply initial state
        if !self.initial_state.is_empty() {
            let changes: Vec<StateChange> = self
                .initial_state
                .into_iter()
                .map(|(key, value)| StateChange::Set { key, value })
                .collect();
            sim.storage.apply_changes(changes).expect("initial state");
        }

        sim
    }

    /// Builds the simulator and returns the seed.
    pub fn build_with_seed(self) -> (Simulator, u64) {
        let seed = self.seed.unwrap_or_else(rand::random);
        let sim = SimulatorBuilder {
            seed: Some(seed),
            ..self
        }
        .build();
        (sim, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulator_determinism() {
        let seed = 42u64;

        let mut sim1 = Simulator::new(seed, SimConfig::default());
        let mut sim2 = Simulator::new(seed, SimConfig::default());

        // Same seed should produce identical random sequences
        for _ in 0..100 {
            let v1: u64 = sim1.rng().gen();
            let v2: u64 = sim2.rng().gen();
            assert_eq!(v1, v2);
        }
    }

    #[test]
    fn test_simulator_time_progression() {
        let mut sim = Simulator::new(42, SimConfig::default());

        assert_eq!(sim.time().block_height(), 0);

        sim.advance_block();
        assert_eq!(sim.time().block_height(), 1);

        sim.set_block_height(10);
        assert_eq!(sim.time().block_height(), 10);
    }

    #[test]
    fn test_simulator_storage() {
        let mut sim = Simulator::new(42, SimConfig::default());

        let changes = vec![StateChange::Set {
            key: b"key".to_vec(),
            value: b"value".to_vec(),
        }];

        sim.apply_state_changes(changes).unwrap();

        let value = sim.storage().get(b"key").unwrap();
        assert_eq!(value, Some(b"value".to_vec()));
    }

    #[test]
    fn test_simulator_snapshot_restore() {
        let mut sim = Simulator::new(42, SimConfig::default());

        sim.advance_block();
        sim.apply_state_changes(vec![StateChange::Set {
            key: b"k1".to_vec(),
            value: b"v1".to_vec(),
        }])
        .unwrap();

        let snapshot = sim.snapshot();

        sim.advance_block();
        sim.apply_state_changes(vec![StateChange::Set {
            key: b"k2".to_vec(),
            value: b"v2".to_vec(),
        }])
        .unwrap();

        sim.restore(snapshot);

        assert_eq!(sim.time().block_height(), 1);
        assert!(sim.storage().get(b"k2").unwrap().is_none());
    }

    #[test]
    fn test_simulator_builder() {
        let (sim, seed) = SimulatorBuilder::new()
            .max_blocks(100)
            .stop_on_error()
            .with_state(b"initial_key".to_vec(), b"initial_value".to_vec())
            .build_with_seed();

        assert_eq!(sim.config().max_blocks, 100);
        assert!(sim.config().stop_on_error);
        assert_eq!(
            sim.storage().get(b"initial_key").unwrap(),
            Some(b"initial_value".to_vec())
        );
        assert!(seed > 0);
    }

    #[test]
    fn test_run_blocks() {
        let mut sim = Simulator::new(42, SimConfig::default());

        sim.run_blocks(10).unwrap();

        assert!(sim.time().block_height() >= 10);
    }

    #[test]
    fn test_run_until() {
        let mut sim = Simulator::new(42, SimConfig::default());

        sim.run_until(|s| s.time().block_height() >= 5).unwrap();

        assert!(sim.time().block_height() >= 5);
    }

    #[test]
    fn test_abort_simulation() {
        let mut sim = Simulator::new(42, SimConfig::default());

        // Abort after 3 blocks
        let result = sim.run_until(|s| {
            if s.time().block_height() >= 3 {
                return true;
            }
            false
        });

        assert!(result.is_ok());
        assert!(sim.time().block_height() >= 3);
    }

    #[test]
    fn test_seed_info() {
        let sim = Simulator::new(12345, SimConfig::default());
        let info = sim.seed_info();

        assert_eq!(info.seed, 12345);
        assert!(info.reproduction_command().contains("12345"));
    }
}
