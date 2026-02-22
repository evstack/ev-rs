use alloy_primitives::B256;
use commonware_consensus::simplex::types::Activity;
use commonware_consensus::Reporter;
use commonware_cryptography::certificate::Scheme;
use commonware_cryptography::sha256::Digest as Sha256Digest;
use evolve_stf_traits::Transaction;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

use crate::block::ConsensusBlock;

/// Shared chain state that the reporter updates on finalization.
///
/// The automaton exposes these via `last_hash()` and `height_atomic()`.
/// Pass them to the reporter so finalization events update chain linkage.
pub struct ChainState<Tx: Send + 'static> {
    /// The hash of the most recently finalized block.
    pub last_hash: Arc<TokioRwLock<B256>>,
    /// The current chain height.
    pub height: Arc<AtomicU64>,
    /// Pending blocks cache (shared with automaton).
    pub pending_blocks: Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>,
}

impl<Tx: Send + 'static> Clone for ChainState<Tx> {
    fn clone(&self) -> Self {
        Self {
            last_hash: self.last_hash.clone(),
            height: self.height.clone(),
            pending_blocks: self.pending_blocks.clone(),
        }
    }
}

/// A reporter that logs consensus activity and updates chain state on finalization.
///
/// Generic over the activity type so it works with any consensus scheme.
/// Holds an optional [`ChainState`] reference — when present, finalization
/// events update `last_hash` so subsequent proposals use the correct parent.
///
/// In production, the marshal sits between simplex and this reporter,
/// delivering ordered `Update<B>` events. For direct (non-marshal) usage,
/// this reporter receives raw `Activity<S, D>` events.
pub struct EvolveReporter<A, Tx: Send + 'static = Vec<u8>> {
    chain_state: Option<ChainState<Tx>>,
    _phantom: std::marker::PhantomData<fn() -> A>,
}

impl<A, Tx: Send + 'static> Clone for EvolveReporter<A, Tx> {
    fn clone(&self) -> Self {
        Self {
            chain_state: self.chain_state.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<A, Tx: Send + 'static> EvolveReporter<A, Tx> {
    /// Create a reporter without chain state (logging only).
    pub fn new() -> Self {
        Self {
            chain_state: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reporter with chain state for finalization updates.
    pub fn with_chain_state(chain_state: ChainState<Tx>) -> Self {
        Self {
            chain_state: Some(chain_state),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<A, Tx: Send + 'static> Default for EvolveReporter<A, Tx> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, Tx> Reporter for EvolveReporter<Activity<S, Sha256Digest>, Tx>
where
    S: Scheme + Clone + Send + 'static,
    Tx: Clone + Transaction + Send + Sync + 'static,
{
    type Activity = Activity<S, Sha256Digest>;

    async fn report(&mut self, activity: Self::Activity) {
        let Some(state) = self.chain_state.as_ref() else {
            tracing::debug!("reporter: received consensus activity");
            return;
        };

        let finalized_digest = match activity {
            Activity::Finalization(finalization) => finalization.proposal.payload.0,
            _ => {
                tracing::debug!(
                    height = state.height.load(Ordering::SeqCst),
                    "reporter: received non-finalization activity"
                );
                return;
            }
        };

        let finalized_block = {
            let mut pending = state.pending_blocks.write().unwrap();
            pending.remove(&finalized_digest)
        };

        let Some(block) = finalized_block else {
            tracing::warn!(
                digest = ?finalized_digest,
                "reporter: finalization digest not found in pending blocks"
            );
            return;
        };

        let finalized_hash = block.block_hash();
        *state.last_hash.write().await = finalized_hash;
        state.height.store(
            block.inner.header.number.saturating_add(1),
            Ordering::SeqCst,
        );

        tracing::debug!(
            digest = ?finalized_digest,
            block_number = block.inner.header.number,
            block_hash = ?finalized_hash,
            "reporter: advanced chain state from finalized activity"
        );
    }
}
