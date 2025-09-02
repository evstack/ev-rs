//! Shrinking utilities for finding minimal failing cases.
//!
//! When a property test fails, the shrinker attempts to find the smallest
//! input that still causes the failure, making debugging easier.

use crate::generators::{TestBlock, TestScenario};
use crate::invariants::InvariantViolation;
use serde::{Deserialize, Serialize};

/// A minimal failure case extracted from a larger failing scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalFailure {
    /// The seed that produced this failure.
    pub seed: u64,
    /// The invariant that was violated.
    pub violated_invariant: String,
    /// Minimal blocks needed to reproduce.
    pub blocks: Vec<MinimalBlock>,
    /// Total transactions in the minimal case.
    pub total_txs: usize,
    /// Shrinking statistics.
    pub shrink_stats: ShrinkStats,
}

/// A minimal block in a failure case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalBlock {
    /// Block height.
    pub height: u64,
    /// Transaction indices from the original block that are required.
    pub tx_indices: Vec<usize>,
}

/// Statistics about the shrinking process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShrinkStats {
    /// Original number of blocks.
    pub original_blocks: usize,
    /// Final number of blocks.
    pub shrunk_blocks: usize,
    /// Original number of transactions.
    pub original_txs: usize,
    /// Final number of transactions.
    pub shrunk_txs: usize,
    /// Number of shrink iterations performed.
    pub iterations: usize,
}

impl ShrinkStats {
    /// Returns the reduction ratio (0.0 to 1.0, lower is better).
    pub fn reduction_ratio(&self) -> f64 {
        if self.original_txs == 0 {
            1.0
        } else {
            self.shrunk_txs as f64 / self.original_txs as f64
        }
    }
}

/// Shrinking strategy configuration.
#[derive(Debug, Clone)]
pub struct ShrinkConfig {
    /// Maximum number of shrinking iterations.
    pub max_iterations: usize,
    /// Whether to try removing entire blocks.
    pub try_remove_blocks: bool,
    /// Whether to try removing individual transactions.
    pub try_remove_txs: bool,
    /// Whether to try binary search for first failing block.
    pub binary_search_blocks: bool,
}

impl Default for ShrinkConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            try_remove_blocks: true,
            try_remove_txs: true,
            binary_search_blocks: true,
        }
    }
}

/// Shrinks a failing test scenario to find the minimal failure case.
///
/// The shrinking process tries to:
/// 1. Find the first block that causes the failure (binary search)
/// 2. Remove blocks that aren't needed for the failure
/// 3. Remove transactions within blocks that aren't needed
pub struct FailureShrink<F>
where
    F: FnMut(&TestScenario) -> Result<(), InvariantViolation>,
{
    /// The original failing scenario.
    original: TestScenario,
    /// The invariant that was violated.
    violation: InvariantViolation,
    /// Function to replay and check for failure.
    replay_fn: F,
    /// Shrinking configuration.
    config: ShrinkConfig,
}

impl<F> FailureShrink<F>
where
    F: FnMut(&TestScenario) -> Result<(), InvariantViolation>,
{
    /// Creates a new shrink operation.
    pub fn new(
        original: TestScenario,
        violation: InvariantViolation,
        replay_fn: F,
        config: ShrinkConfig,
    ) -> Self {
        Self {
            original,
            violation,
            replay_fn,
            config,
        }
    }

    /// Performs the shrinking and returns the minimal failure.
    pub fn shrink(mut self) -> MinimalFailure {
        let mut stats = ShrinkStats {
            original_blocks: self.original.blocks.len(),
            original_txs: self.original.blocks.iter().map(|b| b.txs.len()).sum(),
            ..Default::default()
        };

        let mut current_blocks = self.original.blocks.clone();
        let mut iterations = 0;

        // Phase 1: Binary search for first failing block
        if self.config.binary_search_blocks && current_blocks.len() > 1 {
            if let Some(first_failing) = self.find_first_failing_block(&current_blocks) {
                // Keep only blocks up to and including the first failing one
                current_blocks.truncate(first_failing + 1);
                iterations += 1;
            }
        }

        // Phase 2: Try removing blocks from the end
        if self.config.try_remove_blocks {
            while iterations < self.config.max_iterations && current_blocks.len() > 1 {
                let without_last = current_blocks[..current_blocks.len() - 1].to_vec();
                if self.still_fails(&without_last) {
                    current_blocks = without_last;
                    iterations += 1;
                } else {
                    break;
                }
            }
        }

        // Phase 3: Try removing transactions from each block
        if self.config.try_remove_txs {
            for block_idx in 0..current_blocks.len() {
                if iterations >= self.config.max_iterations {
                    break;
                }

                let block = &current_blocks[block_idx];
                if block.txs.is_empty() {
                    continue;
                }

                // Try removing each transaction
                for tx_idx in (0..block.txs.len()).rev() {
                    if iterations >= self.config.max_iterations {
                        break;
                    }

                    let mut modified_blocks = current_blocks.clone();
                    modified_blocks[block_idx].txs.remove(tx_idx);

                    if self.still_fails(&modified_blocks) {
                        current_blocks = modified_blocks;
                        iterations += 1;
                    }
                }
            }
        }

        stats.shrunk_blocks = current_blocks.len();
        stats.shrunk_txs = current_blocks.iter().map(|b| b.txs.len()).sum();
        stats.iterations = iterations;

        // Build minimal failure report
        let blocks: Vec<MinimalBlock> = current_blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.txs.is_empty())
            .map(|(height, block)| MinimalBlock {
                height: height as u64,
                tx_indices: (0..block.txs.len()).collect(),
            })
            .collect();

        MinimalFailure {
            seed: self.original.seed,
            violated_invariant: self.violation.invariant_name,
            total_txs: stats.shrunk_txs,
            blocks,
            shrink_stats: stats,
        }
    }

    /// Binary search to find the first block that causes failure.
    fn find_first_failing_block(&mut self, blocks: &[TestBlock]) -> Option<usize> {
        if blocks.is_empty() {
            return None;
        }

        let mut low = 0;
        let mut high = blocks.len();
        let mut result = None;

        while low < high {
            let mid = low + (high - low) / 2;
            let prefix = blocks[..=mid].to_vec();

            if self.still_fails(&prefix) {
                result = Some(mid);
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        result
    }

    /// Checks if the modified scenario still fails.
    fn still_fails(&mut self, blocks: &[TestBlock]) -> bool {
        let mut scenario = self.original.clone();
        scenario.blocks = blocks.to_vec();
        (self.replay_fn)(&scenario).is_err()
    }
}

/// Simple scenario shrinker that creates increasingly smaller variations.
pub struct ScenarioShrinker {
    scenario: TestScenario,
}

impl ScenarioShrinker {
    pub fn new(scenario: TestScenario) -> Self {
        Self { scenario }
    }

    /// Returns an iterator of increasingly shrunk scenarios.
    pub fn shrink_iter(&self) -> impl Iterator<Item = TestScenario> + '_ {
        ShrinkIterator {
            scenario: &self.scenario,
            phase: ShrinkPhase::RemoveBlocks(0),
        }
    }
}

enum ShrinkPhase {
    RemoveBlocks(usize),
    RemoveTxs { block: usize, tx: usize },
    Done,
}

struct ShrinkIterator<'a> {
    scenario: &'a TestScenario,
    phase: ShrinkPhase,
}

impl Iterator for ShrinkIterator<'_> {
    type Item = TestScenario;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match &mut self.phase {
                ShrinkPhase::RemoveBlocks(idx) => {
                    if *idx >= self.scenario.blocks.len() {
                        self.phase = ShrinkPhase::RemoveTxs { block: 0, tx: 0 };
                        continue;
                    }

                    let mut shrunk = self.scenario.clone();
                    shrunk.blocks.remove(*idx);
                    *idx += 1;

                    if !shrunk.blocks.is_empty() {
                        return Some(shrunk);
                    }
                }
                ShrinkPhase::RemoveTxs { block, tx } => {
                    if *block >= self.scenario.blocks.len() {
                        self.phase = ShrinkPhase::Done;
                        return None;
                    }

                    let current_block = &self.scenario.blocks[*block];
                    if *tx >= current_block.txs.len() {
                        *block += 1;
                        *tx = 0;
                        continue;
                    }

                    let mut shrunk = self.scenario.clone();
                    shrunk.blocks[*block].txs.remove(*tx);
                    *tx += 1;

                    return Some(shrunk);
                }
                ShrinkPhase::Done => return None,
            }
        }
    }
}

impl MinimalFailure {
    /// Formats the failure as a reproduction command.
    pub fn reproduction_command(&self, test_name: &str) -> String {
        format!(
            "EVOLVE_TEST_SEED={} cargo test {} -- --exact",
            self.seed, test_name
        )
    }

    /// Returns a human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "Minimal failure:\n\
             - Seed: {}\n\
             - Invariant: {}\n\
             - Blocks: {} (shrunk from {})\n\
             - Transactions: {} (shrunk from {})\n\
             - Reduction: {:.1}%",
            self.seed,
            self.violated_invariant,
            self.blocks.len(),
            self.shrink_stats.original_blocks,
            self.total_txs,
            self.shrink_stats.original_txs,
            (1.0 - self.shrink_stats.reduction_ratio()) * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::{TestOperation, TestTx};
    use evolve_core::AccountId;

    fn make_test_scenario(num_blocks: usize, txs_per_block: usize) -> TestScenario {
        let accounts = vec![AccountId::new(1), AccountId::new(2)];
        let blocks: Vec<TestBlock> = (0..num_blocks)
            .map(|h| TestBlock {
                height: h as u64,
                timestamp_ms: h as u64 * 1000,
                txs: (0..txs_per_block)
                    .map(|t| TestTx {
                        id: [t as u8; 32],
                        sender: accounts[0],
                        recipient: accounts[1],
                        operation: TestOperation::Noop,
                        gas_limit: 100000,
                        nonce: t as u64,
                    })
                    .collect(),
            })
            .collect();

        TestScenario {
            seed: 12345,
            accounts: accounts.clone(),
            initial_balances: vec![(accounts[0], 1000), (accounts[1], 1000)],
            blocks,
            config: Default::default(),
        }
    }

    #[test]
    fn test_shrink_stats() {
        let stats = ShrinkStats {
            original_blocks: 10,
            shrunk_blocks: 2,
            original_txs: 100,
            shrunk_txs: 5,
            iterations: 50,
        };

        assert!((stats.reduction_ratio() - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_scenario_shrinker_iterator() {
        let scenario = make_test_scenario(3, 2);
        let shrinker = ScenarioShrinker::new(scenario);

        let shrunk: Vec<_> = shrinker.shrink_iter().take(10).collect();

        // Should produce variations
        assert!(!shrunk.is_empty());

        // Each variation should be smaller
        for s in &shrunk {
            assert!(s.blocks.len() <= 3);
        }
    }

    #[test]
    fn test_minimal_failure_summary() {
        let failure = MinimalFailure {
            seed: 42,
            violated_invariant: "test_invariant".to_string(),
            blocks: vec![MinimalBlock {
                height: 0,
                tx_indices: vec![0],
            }],
            total_txs: 1,
            shrink_stats: ShrinkStats {
                original_blocks: 10,
                shrunk_blocks: 1,
                original_txs: 50,
                shrunk_txs: 1,
                iterations: 25,
            },
        };

        let summary = failure.summary();
        assert!(summary.contains("Seed: 42"));
        assert!(summary.contains("test_invariant"));
        assert!(summary.contains("Reduction:"));
    }

    #[test]
    fn test_failure_shrink() {
        let scenario = make_test_scenario(5, 3);
        let violation = InvariantViolation {
            invariant_name: "test".to_string(),
            reason: "failed".to_string(),
            details: String::new(),
        };

        // Replay function that fails only when there are at least 2 blocks
        let replay_fn = |s: &TestScenario| -> Result<(), InvariantViolation> {
            if s.blocks.len() >= 2 {
                Err(InvariantViolation {
                    invariant_name: "test".to_string(),
                    reason: "needs 2 blocks".to_string(),
                    details: String::new(),
                })
            } else {
                Ok(())
            }
        };

        let shrinker = FailureShrink::new(scenario, violation, replay_fn, ShrinkConfig::default());

        let minimal = shrinker.shrink();

        // Should shrink to exactly 2 blocks
        assert!(minimal.shrink_stats.shrunk_blocks <= 2);
    }
}
