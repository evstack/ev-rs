//! EvolveServer - the main server type that composes infrastructure.

use crate::error::ServerError;
use crate::indexer::{BlockData, IndexerHandle};
use alloy_primitives::B256;
use evolve_operations::config::NodeConfig;
use evolve_operations::shutdown::ShutdownCoordinator;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Pending block metadata, tracked between register_block and commit.
#[derive(Debug, Clone)]
pub(crate) struct PendingBlock {
    height: u64,
    hash: B256,
    #[allow(dead_code)]
    block_data: BlockData,
}

/// The main Evolve server that composes all infrastructure.
///
/// # Type Parameters
///
/// * `Stf` - The state transition function type
/// * `Storage` - The storage backend type
/// * `Codes` - The account codes storage type
pub struct EvolveServer<Stf, Storage, Codes> {
    /// Node configuration.
    pub(crate) config: NodeConfig,

    /// Storage backend.
    pub(crate) storage: Arc<Storage>,

    /// Account codes storage.
    pub(crate) account_codes: Arc<Codes>,

    /// State transition function.
    pub(crate) stf: Arc<Stf>,

    /// Indexer handle for sending events.
    pub(crate) indexer: IndexerHandle,

    /// Pending block (between register_block and commit).
    pub(crate) pending_block: RwLock<Option<PendingBlock>>,

    /// Shutdown coordinator.
    pub(crate) shutdown: ShutdownCoordinator,
}

impl<Stf, Storage, Codes> EvolveServer<Stf, Storage, Codes> {
    /// Create a new server builder.
    pub fn builder() -> crate::builder::ServerBuilder<Stf, Storage, Codes> {
        crate::builder::ServerBuilder::new()
    }

    /// Get the node configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// Get a reference to the storage.
    pub fn storage(&self) -> &Arc<Storage> {
        &self.storage
    }

    /// Get a reference to the account codes storage.
    pub fn account_codes(&self) -> &Arc<Codes> {
        &self.account_codes
    }

    /// Get a reference to the STF.
    pub fn stf(&self) -> &Arc<Stf> {
        &self.stf
    }

    /// Get a reference to the shutdown coordinator.
    pub fn shutdown_coordinator(&self) -> &ShutdownCoordinator {
        &self.shutdown
    }
}

impl<Stf, Storage, Codes> EvolveServer<Stf, Storage, Codes>
where
    Storage: Send + Sync + 'static,
    Codes: Send + Sync + 'static,
    Stf: Send + Sync + 'static,
{
    /// Register a block for indexing after STF execution.
    ///
    /// Call this after executing a block through the STF. It sends chain data
    /// to the indexer for Phase 1 indexing (blocks, txs, receipts, logs).
    ///
    /// After calling this, you must call `commit()` to persist state changes
    /// and trigger Phase 2 indexing (state snapshots).
    pub async fn register_block(
        &self,
        height: u64,
        hash: B256,
        block_data: BlockData,
    ) -> Result<(), ServerError> {
        {
            let pending = self.pending_block.read().await;
            if pending.is_some() {
                return Err(ServerError::Execution(
                    "previous block not committed".to_string(),
                ));
            }
        }

        // Phase 1: Send chain data to indexer (non-blocking)
        self.indexer
            .send_chain_data(height, hash, block_data.clone());

        // Track pending block
        *self.pending_block.write().await = Some(PendingBlock {
            height,
            hash,
            block_data,
        });

        Ok(())
    }

    /// Commit storage changes and trigger state snapshot indexing.
    ///
    /// This must be called after `register_block()`.
    pub async fn commit<F, E>(&self, commit_fn: F) -> Result<(), ServerError>
    where
        F: FnOnce() -> Result<(), E>,
        E: std::fmt::Display,
    {
        let pending = self
            .pending_block
            .write()
            .await
            .take()
            .ok_or(ServerError::NoPendingBlock)?;

        // Commit storage
        commit_fn().map_err(|e| ServerError::Storage(e.to_string()))?;

        // Phase 2: Send state snapshot to indexer (non-blocking)
        self.indexer
            .send_state_snapshot(pending.height, pending.hash);

        Ok(())
    }

    /// Graceful shutdown.
    pub async fn shutdown(self) -> Result<(), ServerError> {
        log::info!("Server shutting down...");
        self.indexer.shutdown().await;
        self.shutdown.shutdown().await;
        log::info!("Server shutdown complete");
        Ok(())
    }
}
