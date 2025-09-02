//! # Evolve Property Testing
//!
//! A property-based testing framework for the Evolve SDK.
//!
//! ## Features
//!
//! - **Generators**: Create random but valid test inputs (transactions, blocks, scenarios)
//! - **Invariants**: Define and check system properties that must always hold
//! - **Shrinking**: Automatically find minimal failing cases when tests fail
//!
//! ## Example
//!
//! ```ignore
//! use evolve_proptest::{generators, invariants};
//! use proptest::prelude::*;
//!
//! proptest! {
//!     #[test]
//!     fn test_balance_conservation(
//!         scenario in generators::arb_scenario(Default::default())
//!     ) {
//!         let checker = invariants::StandardInvariants::token_invariants(
//!             AccountId::new(1),
//!             1_000_000,
//!         );
//!
//!         // Execute scenario and check invariants...
//!     }
//! }
//! ```

pub mod generators;
pub mod invariants;
pub mod shrinker;

pub use generators::{
    arb_account_id, arb_amount, arb_block, arb_fungible_asset, arb_scenario, arb_tx,
    ScenarioConfig, TestBlock, TestOperation, TestScenario, TestTransfer, TestTx,
};
pub use invariants::{
    BalanceConservation, CustomInvariant, Invariant, InvariantChecker, InvariantResult,
    InvariantViolation, NoNegativeBalances, NonceMonotonicity, StandardInvariants, StateSizeBounds,
};
pub use shrinker::{FailureShrink, MinimalFailure, ScenarioShrinker, ShrinkConfig, ShrinkStats};

use evolve_core::AccountId;
use evolve_simulator::{SimConfig, Simulator};

/// Result type for property tests.
pub type PropTestResult<T = ()> = Result<T, PropTestError>;

/// Error type for property test failures.
#[derive(Debug)]
pub enum PropTestError {
    /// An invariant was violated.
    InvariantViolation(InvariantViolation),
    /// Simulation error.
    SimulationError(String),
    /// Maximum iterations reached.
    MaxIterations { attempted: usize, limit: usize },
}

impl std::fmt::Display for PropTestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropTestError::InvariantViolation(v) => write!(f, "{v}"),
            PropTestError::SimulationError(msg) => write!(f, "Simulation error: {msg}"),
            PropTestError::MaxIterations { attempted, limit } => {
                write!(f, "Max iterations reached: {attempted}/{limit}")
            }
        }
    }
}

impl std::error::Error for PropTestError {}

impl From<InvariantViolation> for PropTestError {
    fn from(v: InvariantViolation) -> Self {
        PropTestError::InvariantViolation(v)
    }
}

/// Configuration for property test runs.
#[derive(Debug, Clone)]
pub struct PropTestConfig {
    /// Simulator configuration.
    pub sim_config: SimConfig,
    /// Scenario generation configuration.
    pub scenario_config: ScenarioConfig,
    /// Maximum test iterations.
    pub max_iterations: usize,
    /// Whether to stop on first failure.
    pub stop_on_failure: bool,
    /// Whether to shrink failures.
    pub shrink_failures: bool,
}

impl Default for PropTestConfig {
    fn default() -> Self {
        Self {
            sim_config: SimConfig::default(),
            scenario_config: ScenarioConfig::default(),
            max_iterations: 100,
            stop_on_failure: true,
            shrink_failures: true,
        }
    }
}

/// A property test runner that integrates simulation with invariant checking.
pub struct PropertyTestRunner {
    config: PropTestConfig,
    invariants: InvariantChecker,
}

impl PropertyTestRunner {
    /// Creates a new property test runner.
    pub fn new(config: PropTestConfig) -> Self {
        Self {
            config,
            invariants: InvariantChecker::new(),
        }
    }

    /// Adds an invariant to check.
    pub fn with_invariant(mut self, invariant: impl Invariant + 'static) -> Self {
        self.invariants.add(invariant);
        self
    }

    /// Adds all standard token invariants.
    pub fn with_token_invariants(self, asset_id: AccountId, supply: u128) -> Self {
        self.with_invariant(BalanceConservation::with_defaults(asset_id, supply))
            .with_invariant(NoNegativeBalances::new(b"balance:".to_vec()))
    }

    /// Runs the property test with the given scenario.
    pub fn run(&self, scenario: &TestScenario) -> PropTestResult<()> {
        let mut sim = Simulator::new(scenario.seed, self.config.sim_config.clone());

        // Execute each block
        for block in &scenario.blocks {
            sim.set_block_height(block.height);

            // Execute transactions (simplified - in real impl would go through STF)
            for _tx in &block.txs {
                // Transaction execution would happen here
            }

            // Check invariants after each block
            self.invariants.check_all_or_fail(sim.storage())?;
        }

        Ok(())
    }

    /// Runs with shrinking on failure.
    pub fn run_with_shrinking(&self, scenario: &TestScenario) -> PropTestResult<()> {
        match self.run(scenario) {
            Ok(()) => Ok(()),
            Err(PropTestError::InvariantViolation(violation)) if self.config.shrink_failures => {
                // Create a copy of self for the closure
                let config = self.config.clone();

                let shrinker = FailureShrink::new(
                    scenario.clone(),
                    violation.clone(),
                    |s| {
                        let runner = PropertyTestRunner::new(config.clone());
                        runner.run(s).map_err(|e| match e {
                            PropTestError::InvariantViolation(v) => v,
                            _ => InvariantViolation {
                                invariant_name: "unknown".to_string(),
                                reason: e.to_string(),
                                details: String::new(),
                            },
                        })
                    },
                    ShrinkConfig::default(),
                );

                let minimal = shrinker.shrink();
                eprintln!("{}", minimal.summary());
                eprintln!("Reproduce: {}", minimal.reproduction_command("test"));

                Err(PropTestError::InvariantViolation(violation))
            }
            Err(e) => Err(e),
        }
    }
}

/// Helper macro for defining property tests with the Evolve framework.
///
/// # Example
///
/// ```ignore
/// evolve_proptest! {
///     #[test]
///     fn test_token_transfers(config = Default::default()) {
///         |scenario| {
///             let runner = PropertyTestRunner::new(config)
///                 .with_token_invariants(AccountId::new(1), 1_000_000);
///             runner.run(&scenario)
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! evolve_proptest {
    (
        #[test]
        fn $name:ident(config = $config:expr) $body:block
    ) => {
        #[test]
        fn $name() {
            use proptest::prelude::*;
            use $crate::generators::arb_scenario;

            let config: $crate::PropTestConfig = $config;
            let strategy = arb_scenario(config.scenario_config.clone());

            proptest!(|(scenario in strategy)| {
                let test_fn = $body;
                test_fn(scenario).map_err(|e| {
                    proptest::test_runner::TestCaseError::fail(format!("{}", e))
                })?;
            });
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prop_test_config_default() {
        let config = PropTestConfig::default();
        assert_eq!(config.max_iterations, 100);
        assert!(config.stop_on_failure);
        assert!(config.shrink_failures);
    }

    #[test]
    fn test_property_runner_creation() {
        let runner = PropertyTestRunner::new(PropTestConfig::default())
            .with_token_invariants(AccountId::new(1), 1_000_000);

        assert_eq!(runner.invariants.len(), 2);
    }

    #[test]
    fn test_empty_scenario_passes() {
        let runner = PropertyTestRunner::new(PropTestConfig::default());

        let scenario = TestScenario {
            seed: 42,
            accounts: vec![AccountId::new(1), AccountId::new(2)],
            initial_balances: vec![(AccountId::new(1), 1000)],
            blocks: vec![],
            config: ScenarioConfig::default(),
        };

        assert!(runner.run(&scenario).is_ok());
    }

    #[test]
    fn test_prop_test_error_display() {
        let err = PropTestError::InvariantViolation(InvariantViolation {
            invariant_name: "test".to_string(),
            reason: "failed".to_string(),
            details: String::new(),
        });

        let display = format!("{err}");
        assert!(display.contains("test"));
        assert!(display.contains("failed"));
    }
}
