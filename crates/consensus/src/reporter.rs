use alloy_primitives::B256;
use commonware_consensus::Reporter;
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

impl<A: Clone + Send + 'static, Tx: Send + Sync + 'static> Reporter for EvolveReporter<A, Tx> {
    type Activity = A;

    async fn report(&mut self, _activity: Self::Activity) {
        // TODO: Extract finalization events from activity, look up the finalized
        // block by digest in pending_blocks, and update chain state:
        //   1. Set last_hash to the finalized block's hash
        //   2. Remove the block from pending_blocks
        //   3. Execute through STF and commit state changes
        //
        // This requires matching on Activity<S, D>::Finalization which depends on
        // concrete scheme types. Will be implemented when the full finalization
        // pipeline is wired.
        if let Some(ref state) = self.chain_state {
            tracing::debug!(
                height = state.height.load(Ordering::SeqCst),
                "reporter: received consensus activity (chain state wired)"
            );
        } else {
            tracing::debug!("reporter: received consensus activity");
        }
    }
}
