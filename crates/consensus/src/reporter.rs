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
            let mut pending = state.pending_blocks.write().unwrap_or_else(|poison| {
                tracing::warn!("reporter: recovered from poisoned pending_blocks lock");
                poison.into_inner()
            });
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
        state.height.fetch_max(
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

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::simplex::scheme::ed25519;
    use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
    use commonware_consensus::types::{Epoch, Round, View};
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_parallel::Sequential;
    use commonware_utils::ordered::Set;
    use evolve_core::{AccountId, FungibleAsset, InvokeRequest, Message};
    use evolve_server::{Block, BlockHeader};
    use std::sync::Arc;

    #[derive(Clone)]
    struct TestTx {
        id: [u8; 32],
        request: InvokeRequest,
        funds: Vec<FungibleAsset>,
    }

    impl TestTx {
        fn new(id: [u8; 32]) -> Self {
            Self {
                id,
                request: InvokeRequest::new_from_message("test", 1, Message::from_bytes(vec![])),
                funds: Vec::new(),
            }
        }
    }

    impl Transaction for TestTx {
        fn sender(&self) -> AccountId {
            AccountId::new(1)
        }

        fn recipient(&self) -> AccountId {
            AccountId::new(2)
        }

        fn request(&self) -> &InvokeRequest {
            &self.request
        }

        fn gas_limit(&self) -> u64 {
            21_000
        }

        fn funds(&self) -> &[FungibleAsset] {
            &self.funds
        }

        fn compute_identifier(&self) -> [u8; 32] {
            self.id
        }
    }

    fn test_scheme() -> ed25519::Scheme {
        let private_key = PrivateKey::from_seed(7);
        let public_key = private_key.public_key();
        let participants = Set::from_iter_dedup([public_key]);
        ed25519::Scheme::signer(b"reporter-test", participants, private_key)
            .expect("signer must exist in participants")
    }

    fn finalization_activity_for_digest(
        scheme: &ed25519::Scheme,
        digest: Sha256Digest,
    ) -> Activity<ed25519::Scheme, Sha256Digest> {
        let proposal = Proposal::new(
            Round::new(Epoch::zero(), View::new(1)),
            View::zero(),
            digest,
        );
        let finalize_vote = Finalize::sign(scheme, proposal).expect("finalize vote must sign");
        let finalization = Finalization::from_finalizes(scheme, [&finalize_vote], &Sequential)
            .expect("single-validator finalization must assemble");
        Activity::Finalization(finalization)
    }

    #[tokio::test]
    async fn finalization_updates_chain_state_and_evicts_pending_block() {
        let last_hash = Arc::new(TokioRwLock::new(B256::ZERO));
        let height = Arc::new(AtomicU64::new(1));
        let pending_blocks = Arc::new(RwLock::new(
            BTreeMap::<[u8; 32], ConsensusBlock<TestTx>>::new(),
        ));

        let block = Block::new(
            BlockHeader::new(1, 1_000, B256::ZERO),
            vec![TestTx::new([1u8; 32])],
        );
        let consensus_block = ConsensusBlock::new(block);
        let digest = consensus_block.digest_value();
        let block_hash = consensus_block.block_hash();
        pending_blocks
            .write()
            .unwrap()
            .insert(consensus_block.digest_value().0, consensus_block);

        let chain_state = ChainState {
            last_hash: last_hash.clone(),
            height: height.clone(),
            pending_blocks: pending_blocks.clone(),
        };
        let mut reporter =
            EvolveReporter::<Activity<ed25519::Scheme, Sha256Digest>, TestTx>::with_chain_state(
                chain_state,
            );

        let scheme = test_scheme();
        reporter
            .report(finalization_activity_for_digest(&scheme, digest))
            .await;

        assert_eq!(*last_hash.read().await, block_hash);
        assert_eq!(height.load(Ordering::SeqCst), 2);
        assert!(
            pending_blocks.read().unwrap().is_empty(),
            "finalized block should be evicted from pending cache"
        );
    }

    #[tokio::test]
    async fn non_finalization_activity_does_not_mutate_chain_state() {
        let last_hash = Arc::new(TokioRwLock::new(B256::repeat_byte(0xAA)));
        let height = Arc::new(AtomicU64::new(42));
        let pending_blocks = Arc::new(RwLock::new(
            BTreeMap::<[u8; 32], ConsensusBlock<TestTx>>::new(),
        ));

        let chain_state = ChainState {
            last_hash: last_hash.clone(),
            height: height.clone(),
            pending_blocks: pending_blocks.clone(),
        };
        let mut reporter =
            EvolveReporter::<Activity<ed25519::Scheme, Sha256Digest>, TestTx>::with_chain_state(
                chain_state,
            );

        let digest = Sha256Digest([9u8; 32]);
        let proposal = Proposal::new(
            Round::new(Epoch::zero(), View::new(2)),
            View::new(1),
            digest,
        );
        let scheme = test_scheme();
        let finalize_vote = Finalize::sign(&scheme, proposal).expect("finalize vote must sign");

        reporter.report(Activity::Finalize(finalize_vote)).await;

        assert_eq!(*last_hash.read().await, B256::repeat_byte(0xAA));
        assert_eq!(height.load(Ordering::SeqCst), 42);
        assert!(pending_blocks.read().unwrap().is_empty());
    }

    #[tokio::test]
    async fn finalization_with_unknown_digest_does_not_mutate_chain_state() {
        let last_hash = Arc::new(TokioRwLock::new(B256::repeat_byte(0xBB)));
        let height = Arc::new(AtomicU64::new(7));
        let pending_blocks = Arc::new(RwLock::new(
            BTreeMap::<[u8; 32], ConsensusBlock<TestTx>>::new(),
        ));

        let chain_state = ChainState {
            last_hash: last_hash.clone(),
            height: height.clone(),
            pending_blocks: pending_blocks.clone(),
        };
        let mut reporter =
            EvolveReporter::<Activity<ed25519::Scheme, Sha256Digest>, TestTx>::with_chain_state(
                chain_state,
            );

        let scheme = test_scheme();
        reporter
            .report(finalization_activity_for_digest(
                &scheme,
                Sha256Digest([0xCC; 32]),
            ))
            .await;

        assert_eq!(*last_hash.read().await, B256::repeat_byte(0xBB));
        assert_eq!(height.load(Ordering::SeqCst), 7);
        assert!(pending_blocks.read().unwrap().is_empty());
    }
}
