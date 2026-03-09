use crate::block::ConsensusBlock;
use commonware_consensus::Relay;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock, Weak};

type BlockStore<Tx> = Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>;
type BlockStoreWeak<Tx> = Weak<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>;

/// Shared in-memory fanout used by [`EvolveRelay`] instances in the same test/process.
///
/// Each validator should be constructed with the same network handle so `broadcast()`
/// can copy the full block into every peer's pending-block store.
#[derive(Debug)]
pub struct InMemoryRelayNetwork<Tx> {
    peers: Mutex<Vec<BlockStoreWeak<Tx>>>,
}

impl<Tx> InMemoryRelayNetwork<Tx> {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(&self, blocks: &BlockStore<Tx>) {
        let mut peers = self
            .peers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        peers.retain(|peer| peer.strong_count() > 0);

        let already_registered = peers.iter().any(|peer| {
            peer.upgrade()
                .is_some_and(|registered| Arc::ptr_eq(&registered, blocks))
        });
        if !already_registered {
            peers.push(Arc::downgrade(blocks));
        }
    }

    fn fanout(&self, source: &BlockStore<Tx>, block: &ConsensusBlock<Tx>) -> usize
    where
        Tx: Clone,
    {
        let digest = block.digest_value().0;
        let mut delivered = 0;
        let mut peers = self
            .peers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        peers.retain(|peer| {
            let Some(store) = peer.upgrade() else {
                return false;
            };
            if Arc::ptr_eq(&store, source) {
                return true;
            }
            store
                .write()
                .unwrap_or_else(|poison| {
                    tracing::warn!("relay: recovered from poisoned write lock");
                    poison.into_inner()
                })
                .insert(digest, block.clone());
            delivered += 1;
            true
        });
        delivered
    }
}

impl<Tx> Default for InMemoryRelayNetwork<Tx> {
    fn default() -> Self {
        Self {
            peers: Mutex::new(Vec::new()),
        }
    }
}

/// A local in-memory relay for block broadcast.
///
/// Stores proposed blocks so that other participants (or the local verifier)
/// can look them up by digest. Validators that should exchange proposals must
/// share the same [`InMemoryRelayNetwork`].
#[derive(Clone)]
pub struct EvolveRelay<Tx> {
    /// Shared storage of blocks indexed by their digest.
    blocks: BlockStore<Tx>,
    network: Arc<InMemoryRelayNetwork<Tx>>,
}

impl<Tx> EvolveRelay<Tx> {
    pub fn new(network: Arc<InMemoryRelayNetwork<Tx>>, blocks: BlockStore<Tx>) -> Self {
        network.register(&blocks);
        Self { blocks, network }
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
        let block = self.get_block(&payload.0);
        let Some(block) = block else {
            tracing::warn!(
                digest = ?payload,
                "relay: cannot broadcast missing block"
            );
            return;
        };

        let recipients = self.network.fanout(&self.blocks, &block);
        tracing::debug!(
            digest = ?payload,
            recipients,
            "relay: broadcast block to registered peers"
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
            AccountId::from_u64(1)
        }

        fn recipient(&self) -> AccountId {
            AccountId::from_u64(2)
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
        let network = Arc::new(InMemoryRelayNetwork::new());
        let store = Arc::new(RwLock::new(BTreeMap::new()));
        let relay = EvolveRelay::new(network, store);
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

    #[tokio::test]
    async fn broadcast_copies_block_to_other_registered_relays() {
        let network = Arc::new(InMemoryRelayNetwork::new());
        let store_a = Arc::new(RwLock::new(BTreeMap::new()));
        let store_b = Arc::new(RwLock::new(BTreeMap::new()));
        let mut relay_a = EvolveRelay::new(network.clone(), store_a);
        let relay_b = EvolveRelay::new(network, store_b);

        let block = evolve_server::Block::new(
            evolve_server::BlockHeader::new(1, 100, B256::ZERO),
            vec![TestTx::new([9u8; 32])],
        );
        let consensus_block = ConsensusBlock::new(block);
        let digest = consensus_block.digest_value();
        relay_a.insert_block(consensus_block.clone());

        relay_a.broadcast(digest).await;

        let fetched = relay_b
            .get_block(&digest.0)
            .expect("peer relay should receive the broadcast block");
        assert_eq!(fetched.digest_value(), consensus_block.digest_value());
    }
}
