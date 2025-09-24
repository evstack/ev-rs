//! Two-phase indexing for chain data and state snapshots.
//!
//! The indexer receives events from the server and processes them asynchronously:
//!
//! - **Phase 1 (ChainData)**: Immediately after `apply_block()`, indexes blocks,
//!   transactions, receipts, and logs. No committed state needed.
//!
//! - **Phase 2 (StateSnapshot)**: After `commit()`, indexes state snapshots.
//!   State is now readable from storage.
//!
//! This design allows indexing to run concurrently with block execution.

use alloy_primitives::B256;
use tokio::sync::mpsc;

/// Events sent to the indexer for processing.
#[derive(Debug, Clone)]
pub enum IndexEvent {
    /// Phase 1: Chain data available immediately after apply_block.
    /// No state access needed - can be indexed right away.
    ChainData {
        /// Block height.
        height: u64,
        /// Block hash.
        hash: B256,
        /// Raw block data for indexing.
        block_data: BlockData,
    },

    /// Phase 2: State snapshot available after commit.
    /// Indexer can now read committed state from storage.
    StateSnapshot {
        /// Block height.
        height: u64,
        /// Block hash.
        hash: B256,
    },

    /// Shutdown signal for graceful termination.
    Shutdown,
}

/// Block data for chain indexing (phase 1).
#[derive(Debug, Clone, Default)]
pub struct BlockData {
    /// Encoded transactions.
    pub transactions: Vec<Vec<u8>>,
    /// Transaction receipts.
    pub receipts: Vec<Receipt>,
    /// Event logs.
    pub logs: Vec<Log>,
}

/// Transaction receipt for indexing.
#[derive(Debug, Clone)]
pub struct Receipt {
    /// Transaction hash.
    pub tx_hash: B256,
    /// Transaction index in block.
    pub tx_index: u64,
    /// Whether execution succeeded.
    pub success: bool,
    /// Gas used.
    pub gas_used: u64,
    /// Logs emitted.
    pub logs: Vec<Log>,
}

/// Event log for indexing.
#[derive(Debug, Clone)]
pub struct Log {
    /// Contract address that emitted the log.
    pub address: [u8; 20],
    /// Indexed topics.
    pub topics: Vec<B256>,
    /// Non-indexed data.
    pub data: Vec<u8>,
}

/// Handle for sending events to the indexer.
#[derive(Clone)]
pub struct IndexerHandle {
    tx: mpsc::Sender<IndexEvent>,
}

impl IndexerHandle {
    /// Create a new indexer handle.
    pub fn new(tx: mpsc::Sender<IndexEvent>) -> Self {
        Self { tx }
    }

    /// Send chain data for indexing (non-blocking).
    /// Returns immediately even if the indexer is busy.
    pub fn send_chain_data(&self, height: u64, hash: B256, block_data: BlockData) {
        let _ = self.tx.try_send(IndexEvent::ChainData {
            height,
            hash,
            block_data,
        });
    }

    /// Send state snapshot notification (non-blocking).
    pub fn send_state_snapshot(&self, height: u64, hash: B256) {
        let _ = self.tx.try_send(IndexEvent::StateSnapshot { height, hash });
    }

    /// Signal shutdown to the indexer.
    pub async fn shutdown(&self) {
        let _ = self.tx.send(IndexEvent::Shutdown).await;
    }
}

/// Spawns the indexer background task.
///
/// Returns a handle for sending events to the indexer.
pub fn spawn_indexer(buffer_size: usize) -> IndexerHandle {
    let (tx, rx) = mpsc::channel(buffer_size);

    tokio::spawn(run_indexer(rx));

    IndexerHandle::new(tx)
}

/// Main indexer loop.
async fn run_indexer(mut rx: mpsc::Receiver<IndexEvent>) {
    log::info!("Indexer started");

    while let Some(event) = rx.recv().await {
        match event {
            IndexEvent::ChainData {
                height,
                hash,
                block_data,
            } => {
                log::debug!("Indexing chain data for block {} ({})", height, hash);
                index_chain_data(height, hash, block_data).await;
            }
            IndexEvent::StateSnapshot { height, hash } => {
                log::debug!("Indexing state snapshot for block {} ({})", height, hash);
                index_state_snapshot(height, hash).await;
            }
            IndexEvent::Shutdown => {
                log::info!("Indexer shutting down");
                break;
            }
        }
    }

    log::info!("Indexer stopped");
}

/// Index chain data (blocks, transactions, receipts, logs).
/// This is Phase 1 - no state access needed.
async fn index_chain_data(height: u64, _hash: B256, block_data: BlockData) {
    // TODO: Implement actual chain data indexing
    // - Store block header
    // - Index transactions by hash
    // - Index receipts
    // - Index logs by address and topics

    log::trace!(
        "Indexed block {} with {} txs, {} receipts, {} logs",
        height,
        block_data.transactions.len(),
        block_data.receipts.len(),
        block_data.logs.len()
    );
}

/// Index state snapshot.
/// This is Phase 2 - state is committed and readable.
async fn index_state_snapshot(height: u64, _hash: B256) {
    // TODO: Implement actual state snapshot indexing
    // - Snapshot account balances
    // - Index contract storage changes
    // - Update state root index

    log::trace!("Indexed state snapshot for block {}", height);
}
