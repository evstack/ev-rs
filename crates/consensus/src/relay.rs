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
        self.blocks
            .read()
            .unwrap_or_else(|poison| {
                tracing::warn!("relay: recovered from poisoned read lock");
                poison.into_inner()
            })
            .get(digest)
            .cloned()
    }

    /// Insert a block into the relay's store.
    pub fn insert_block(&self, block: ConsensusBlock<Tx>)
    where
        Tx: Clone,
    {
        let digest = block.digest_value().0;
        self.blocks
            .write()
            .unwrap_or_else(|poison| {
                tracing::warn!("relay: recovered from poisoned write lock");
                poison.into_inner()
            })
            .insert(digest, block);
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use evolve_core::{AccountId, FungibleAsset, InvokeRequest, Message};
    use evolve_stf_traits::Transaction;
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};

    #[derive(Debug, Clone)]
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

    #[test]
    fn insert_and_get_block_by_digest() {
        let store = Arc::new(RwLock::new(BTreeMap::new()));
        let relay = EvolveRelay::new(store);
        let block = evolve_server::Block::new(
            evolve_server::BlockHeader::new(1, 100, B256::ZERO),
            vec![TestTx::new([7u8; 32])],
        );
        let consensus_block = ConsensusBlock::new(block);
        let digest = consensus_block.digest_value().0;

        relay.insert_block(consensus_block.clone());

        let fetched = relay.get_block(&digest).expect("block should exist");
        assert_eq!(fetched.digest_value(), consensus_block.digest_value());
        assert_eq!(fetched.inner.header.number, 1);
    }
}
