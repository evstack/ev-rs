//! Dev mode consensus engine for testing and development.
//!
//! This module provides a mock consensus engine that can produce blocks
//! without an external consensus layer. It's useful for:
//!
//! - Local development and testing
//! - Running the simulator with block production
//! - Integration testing without full consensus setup
//!
//! # Example
//!
//! ```ignore
//! use evolve_server::{DevConsensus, DevConfig};
//! use std::sync::Arc;
//!
//! // Create storage (must implement ReadonlyKV + Storage)
//! let storage = MyAsyncStorage::new().await;
//!
//! // Create dev consensus
//! let dev = Arc::new(DevConsensus::new(
//!     stf,
//!     storage,
//!     codes,
//!     DevConfig::default(),
//! ));
//!
//! // Produce blocks manually
//! dev.produce_block().await?;
//!
//! // Or run automatic block production
//! dev.run_block_production().await;
//! ```

use crate::block::{Block, BlockBuilder};
use crate::error::ServerError;
use alloy_primitives::B256;
use evolve_core::ReadonlyKV;
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{AccountsCodeStorage, PostTxExecution, Transaction, TxValidator};
use evolve_storage::{Operation, Storage};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Configuration for dev mode block production.
#[derive(Debug, Clone)]
pub struct DevConfig {
    /// Interval between automatic block production.
    /// Set to None to disable auto-production.
    pub block_interval: Option<Duration>,

    /// Gas limit per block.
    pub gas_limit: u64,

    /// Initial block height (default: 1, since 0 is typically genesis).
    pub initial_height: u64,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            block_interval: Some(Duration::from_secs(1)),
            gas_limit: 30_000_000,
            initial_height: 1,
        }
    }
}

impl DevConfig {
    /// Create config with no automatic block production.
    pub fn manual() -> Self {
        Self {
            block_interval: None,
            ..Default::default()
        }
    }

    /// Create config with specified block interval.
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            block_interval: Some(interval),
            ..Default::default()
        }
    }
}

/// State tracked by the dev consensus engine.
struct DevState {
    /// Current block height.
    height: AtomicU64,
    /// Last block hash.
    last_hash: RwLock<B256>,
    /// Last block timestamp.
    last_timestamp: AtomicU64,
}

impl DevState {
    fn new(initial_height: u64) -> Self {
        Self {
            height: AtomicU64::new(initial_height),
            last_hash: RwLock::new(B256::ZERO),
            last_timestamp: AtomicU64::new(0),
        }
    }
}

/// Result of producing a block.
#[derive(Debug)]
pub struct ProducedBlock {
    /// Block height.
    pub height: u64,
    /// Block hash.
    pub hash: B256,
    /// Number of transactions in the block.
    pub tx_count: usize,
    /// Total gas used.
    pub gas_used: u64,
    /// Number of successful transactions.
    pub successful_txs: usize,
    /// Number of failed transactions.
    pub failed_txs: usize,
}

/// A mock consensus engine for development and testing.
///
/// This type provides block production without an external consensus layer.
/// It uses a shared storage with interior mutability for both reads and writes.
///
/// # Type Parameters
///
/// * `Stf` - The state transition function type
/// * `S` - The storage backend (must be ReadonlyKV + Storage for async operations)
/// * `Codes` - Account codes storage
/// * `Tx` - Transaction type
pub struct DevConsensus<Stf, S, Codes, Tx> {
    /// The STF for block execution.
    stf: Stf,
    /// Storage with async batch/commit operations.
    storage: S,
    /// Account codes.
    codes: Codes,
    /// Configuration.
    config: DevConfig,
    /// Internal state.
    state: DevState,
    /// Whether auto-production is running.
    running: AtomicBool,
    /// Phantom for Tx type.
    _tx: std::marker::PhantomData<Tx>,
}

impl<Stf, S, Codes, Tx> DevConsensus<Stf, S, Codes, Tx> {
    /// Create a new dev consensus engine.
    ///
    /// # Arguments
    ///
    /// * `stf` - The state transition function
    /// * `storage` - Storage backend with async batch/commit
    /// * `codes` - Account codes storage
    /// * `config` - Dev mode configuration
    pub fn new(stf: Stf, storage: S, codes: Codes, config: DevConfig) -> Self {
        let initial_height = config.initial_height;
        Self {
            stf,
            storage,
            codes,
            config,
            state: DevState::new(initial_height),
            running: AtomicBool::new(false),
            _tx: std::marker::PhantomData,
        }
    }

    /// Get a reference to the STF.
    pub fn stf(&self) -> &Stf {
        &self.stf
    }

    /// Get a reference to the storage.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Get a reference to the codes.
    pub fn codes(&self) -> &Codes {
        &self.codes
    }

    /// Get the current block height.
    pub fn height(&self) -> u64 {
        self.state.height.load(Ordering::SeqCst)
    }

    /// Get the last block hash.
    pub async fn last_hash(&self) -> B256 {
        *self.state.last_hash.read().await
    }

    /// Check if auto-production is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Stop auto-production.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl<Stf, S, Codes, Tx> DevConsensus<Stf, S, Codes, Tx>
where
    Tx: Transaction + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + 'static,
{
    /// Produce a single empty block.
    ///
    /// This creates a block with no transactions, executes it through the STF,
    /// and commits the state changes.
    pub async fn produce_block(&self) -> Result<ProducedBlock, ServerError> {
        self.produce_block_with_txs(vec![]).await
    }

    /// Produce a block with the given transactions.
    pub async fn produce_block_with_txs(
        &self,
        transactions: Vec<Tx>,
    ) -> Result<ProducedBlock, ServerError> {
        let height = self.state.height.fetch_add(1, Ordering::SeqCst);
        let parent_hash = *self.state.last_hash.read().await;
        let timestamp = current_timestamp();
        let tx_count = transactions.len();

        // Build the block
        let block = BlockBuilder::<Tx>::new()
            .number(height)
            .timestamp(timestamp)
            .parent_hash(parent_hash)
            .gas_limit(self.config.gas_limit)
            .transactions(transactions)
            .build();

        // Execute through STF
        let (result, exec_state) = self.stf.execute_block(&self.storage, &self.codes, &block);

        let changes = exec_state
            .into_changes()
            .map_err(|e| ServerError::Execution(format!("failed to get state changes: {:?}", e)))?;

        // Compute block hash
        let block_hash = compute_block_hash(height, timestamp, parent_hash);

        // Calculate gas used and success/failure counts
        let gas_used: u64 = result.tx_results.iter().map(|r| r.gas_used).sum();
        let successful_txs = result
            .tx_results
            .iter()
            .filter(|r| r.response.is_ok())
            .count();
        let failed_txs = tx_count - successful_txs;

        // Convert StateChange to Operation and commit via async storage
        let operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();
        self.storage
            .batch(operations)
            .await
            .map_err(|e| ServerError::Storage(format!("batch failed: {:?}", e)))?;
        self.storage
            .commit()
            .await
            .map_err(|e| ServerError::Storage(format!("commit failed: {:?}", e)))?;

        // Update state
        *self.state.last_hash.write().await = block_hash;
        self.state.last_timestamp.store(timestamp, Ordering::SeqCst);

        log::info!(
            "Produced block {} with {} txs, {} gas used",
            height,
            tx_count,
            gas_used
        );

        Ok(ProducedBlock {
            height,
            hash: block_hash,
            tx_count,
            gas_used,
            successful_txs,
            failed_txs,
        })
    }

    /// Start automatic block production.
    ///
    /// Returns a future that runs the block production loop.
    /// The future is not `Send` due to storage constraints, so it must be
    /// run on a local executor (e.g., `tokio::task::LocalSet`).
    ///
    /// Use `stop()` to stop production.
    pub async fn run_block_production(self: &Arc<Self>) {
        let interval = match self.config.block_interval {
            Some(i) => i,
            None => return,
        };

        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return; // Already running
        }

        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // First tick is immediate

        while self.running.load(Ordering::SeqCst) {
            ticker.tick().await;

            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            match self.produce_block().await {
                Ok(block) => {
                    log::debug!("Auto-produced block {}", block.height);
                }
                Err(e) => {
                    log::error!("Failed to produce block: {}", e);
                    // Continue trying - dev mode should be resilient
                }
            }
        }

        log::info!("Dev consensus stopped");
    }
}

/// Trait for STF execution.
///
/// This abstracts the STF's `apply_block` method so that `DevConsensus`
/// can work with any compatible STF implementation.
pub trait StfExecutor<Tx, S, Codes>
where
    S: ReadonlyKV,
    Codes: AccountsCodeStorage,
{
    /// Execute a block and return the result with execution state.
    fn execute_block<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        block: &Block<Tx>,
    ) -> (BlockResult, ExecutionState<'a, S>);
}

/// Implement StfExecutor for the evolve_stf::Stf type.
impl<Tx, BeginBlocker, TxValidatorT, EndBlocker, PostTx, S, Codes> StfExecutor<Tx, S, Codes>
    for evolve_stf::Stf<Tx, Block<Tx>, BeginBlocker, TxValidatorT, EndBlocker, PostTx>
where
    Tx: Transaction,
    S: ReadonlyKV,
    Codes: AccountsCodeStorage,
    BeginBlocker: evolve_stf_traits::BeginBlocker<Block<Tx>>,
    TxValidatorT: TxValidator<Tx>,
    EndBlocker: evolve_stf_traits::EndBlocker,
    PostTx: PostTxExecution<Tx>,
{
    fn execute_block<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        block: &Block<Tx>,
    ) -> (BlockResult, ExecutionState<'a, S>) {
        self.apply_block(storage, codes, block)
    }
}

/// Get current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Compute a simple block hash.
///
/// In production, this would be a proper Merkle root or similar.
/// For dev mode, we use a simple hash of height + timestamp + parent.
fn compute_block_hash(height: u64, timestamp: u64, parent_hash: B256) -> B256 {
    use alloy_primitives::keccak256;

    let mut data = Vec::with_capacity(48);
    data.extend_from_slice(&height.to_le_bytes());
    data.extend_from_slice(&timestamp.to_le_bytes());
    data.extend_from_slice(parent_hash.as_slice());

    keccak256(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dev_config_defaults() {
        let config = DevConfig::default();
        assert!(config.block_interval.is_some());
        assert_eq!(config.gas_limit, 30_000_000);
        assert_eq!(config.initial_height, 1);
    }

    #[test]
    fn test_dev_config_manual() {
        let config = DevConfig::manual();
        assert!(config.block_interval.is_none());
    }

    #[test]
    fn test_compute_block_hash_deterministic() {
        let h1 = compute_block_hash(1, 1000, B256::ZERO);
        let h2 = compute_block_hash(1, 1000, B256::ZERO);
        assert_eq!(h1, h2);

        // Different height -> different hash
        let h3 = compute_block_hash(2, 1000, B256::ZERO);
        assert_ne!(h1, h3);
    }
}
