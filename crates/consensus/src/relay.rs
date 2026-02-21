use crate::block::ConsensusBlock;
use commonware_consensus::Relay;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// A local in-memory relay for block broadcast.
///
/// Stores proposed blocks so that other participants (or the local verifier)
/// can look them up by digest. In production, this would broadcast full blocks
/// over the P2P network. For now, it serves as a shared cache between the
/// automaton (proposer) and verifier.
#[derive(Clone)]
pub struct EvolveRelay<Tx> {
    /// Shared storage of blocks indexed by their digest.
    blocks: Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>,
}

impl<Tx> EvolveRelay<Tx> {
    pub fn new(blocks: Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>) -> Self {
        Self { blocks }
    }

    /// Retrieve a block by its digest.
    pub fn get_block(&self, digest: &[u8; 32]) -> Option<ConsensusBlock<Tx>>
    where
        Tx: Clone,
    {
        self.blocks.read().unwrap().get(digest).cloned()
    }

    /// Insert a block into the relay's store.
    pub fn insert_block(&self, block: ConsensusBlock<Tx>)
    where
        Tx: Clone,
    {
        let digest = block.digest.0;
        self.blocks.write().unwrap().insert(digest, block);
    }
}

impl<Tx> Relay for EvolveRelay<Tx>
where
    Tx: Clone + Send + Sync + 'static,
{
    type Digest = commonware_cryptography::sha256::Digest;

    async fn broadcast(&mut self, payload: Self::Digest) {
        // In production, this would broadcast the full block to all peers.
        // For now, the block is already stored in the shared pending_blocks map
        // by the automaton's propose() method. We just log the broadcast.
        tracing::debug!(
            digest = ?payload,
            "relay: broadcasting block digest (local relay, no-op)"
        );
    }
}
