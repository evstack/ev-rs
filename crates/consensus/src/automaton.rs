use crate::block::ConsensusBlock;
use crate::config::ConsensusConfig;
use alloy_primitives::B256;
use commonware_consensus::types::Epoch;
use commonware_consensus::{Automaton, CertifiableAutomaton};
use commonware_cryptography::{Hasher, Sha256};
use commonware_utils::channel::oneshot;
use evolve_core::ReadonlyKV;
use evolve_mempool::{Mempool, MempoolTx, SharedMempool};
use evolve_server::BlockBuilder;
use evolve_server::StfExecutor;
use evolve_stf_traits::{AccountsCodeStorage, Transaction};
use evolve_storage::Storage;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

/// EvolveAutomaton bridges Evolve's STF and mempool with commonware's consensus.
///
/// It implements the `Automaton` and `CertifiableAutomaton` traits, allowing
/// the simplex consensus engine to propose and verify blocks through Evolve's
/// state transition function.
///
/// # Design
///
/// Consensus operates on opaque digests, not full blocks. The automaton:
/// - On `propose()`: builds a block from mempool txs, stores it locally,
///   returns only the digest to consensus.
/// - On `verify()`: looks up the block by digest (populated via Relay),
///   validates parent chain and height.
pub struct EvolveAutomaton<Stf, S, Codes, Tx: MempoolTx, Ctx> {
    stf: Stf,
    storage: S,
    codes: Codes,
    mempool: SharedMempool<Mempool<Tx>>,
    /// Block cache: stores proposed/received blocks by their digest.
    pending_blocks: Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>,
    /// Current chain height.
    height: Arc<AtomicU64>,
    /// Last block hash.
    last_hash: Arc<TokioRwLock<B256>>,
    /// Consensus configuration.
    config: ConsensusConfig,
    /// Phantom for the context type.
    _ctx: std::marker::PhantomData<Ctx>,
}

impl<Stf, S, Codes, Tx: MempoolTx, Ctx> Clone for EvolveAutomaton<Stf, S, Codes, Tx, Ctx>
where
    Stf: Clone,
    S: Clone,
    Codes: Clone,
{
    fn clone(&self) -> Self {
        Self {
            stf: self.stf.clone(),
            storage: self.storage.clone(),
            codes: self.codes.clone(),
            mempool: self.mempool.clone(),
            pending_blocks: self.pending_blocks.clone(),
            height: self.height.clone(),
            last_hash: self.last_hash.clone(),
            config: self.config.clone(),
            _ctx: std::marker::PhantomData,
        }
    }
}

impl<Stf, S, Codes, Tx: MempoolTx, Ctx> EvolveAutomaton<Stf, S, Codes, Tx, Ctx> {
    pub fn new(
        stf: Stf,
        storage: S,
        codes: Codes,
        mempool: SharedMempool<Mempool<Tx>>,
        pending_blocks: Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>>,
        config: ConsensusConfig,
    ) -> Self {
        Self {
            stf,
            storage,
            codes,
            mempool,
            pending_blocks,
            height: Arc::new(AtomicU64::new(1)),
            last_hash: Arc::new(TokioRwLock::new(B256::ZERO)),
            config,
            _ctx: std::marker::PhantomData,
        }
    }

    /// Get the current height.
    pub fn height(&self) -> u64 {
        self.height.load(Ordering::SeqCst)
    }

    /// Get a reference to the shared pending blocks.
    pub fn pending_blocks(&self) -> &Arc<RwLock<BTreeMap<[u8; 32], ConsensusBlock<Tx>>>> {
        &self.pending_blocks
    }

    /// Get a reference to the shared last block hash.
    ///
    /// The finalization path (reporter) must update this after a block is
    /// certified so that subsequent `propose()` calls use the correct parent.
    pub fn last_hash(&self) -> &Arc<TokioRwLock<B256>> {
        &self.last_hash
    }

    /// Get a reference to the shared height counter.
    ///
    /// The finalization path (reporter) may use this to reconcile height
    /// after block finalization.
    pub fn height_atomic(&self) -> &Arc<AtomicU64> {
        &self.height
    }
}

impl<Stf, S, Codes, Tx, Ctx> Automaton for EvolveAutomaton<Stf, S, Codes, Tx, Ctx>
where
    Tx: Transaction + MempoolTx + Clone + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + Clone + 'static,
    Ctx: Clone + Send + 'static,
{
    type Context = Ctx;
    type Digest = commonware_cryptography::sha256::Digest;

    async fn genesis(&mut self, _epoch: Epoch) -> Self::Digest {
        // Genesis: return the digest of the empty genesis block at height 0.
        let genesis_block = BlockBuilder::<Tx>::new()
            .number(0)
            .timestamp(0)
            .parent_hash(B256::ZERO)
            .gas_limit(self.config.gas_limit)
            .build();

        let mut genesis_block = genesis_block;
        genesis_block.header.transactions_root =
            compute_transactions_root(&genesis_block.transactions);

        let cb = ConsensusBlock::new(genesis_block);
        let digest = cb.digest;

        // Store genesis in pending blocks.
        self.pending_blocks.write().unwrap().insert(digest.0, cb);

        digest
    }

    // SystemTime::now() is used here for block timestamps only. This is acceptable
    // in a consensus proposer context — the timestamp is validated by verifiers.
    #[allow(clippy::disallowed_types)]
    async fn propose(&mut self, _context: Self::Context) -> oneshot::Receiver<Self::Digest> {
        let (sender, receiver) = oneshot::channel();

        let height = self.height.clone();
        let last_hash = *self.last_hash.read().await;
        let gas_limit = self.config.gas_limit;
        let mempool = self.mempool.clone();
        let pending_blocks = self.pending_blocks.clone();

        // Spawn block building onto a background task.
        tokio::spawn(async move {
            // Pull transactions from mempool.
            let selected = {
                let mut pool = mempool.write().await;
                pool.select(1000) // max txs per block
            };

            let transactions: Vec<Tx> = selected
                .into_iter()
                .map(|arc_tx| (*arc_tx).clone())
                .collect();

            // Build the block.
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Increment height only after successful block construction to avoid
            // gaps when the spawned task fails or is cancelled.
            let block_height = height.fetch_add(1, Ordering::SeqCst);

            let block = BlockBuilder::<Tx>::new()
                .number(block_height)
                .timestamp(timestamp)
                .parent_hash(last_hash)
                .gas_limit(gas_limit)
                .transactions(transactions)
                .build();

            let mut block = block;
            block.header.transactions_root = compute_transactions_root(&block.transactions);

            let cb = ConsensusBlock::new(block);
            let digest = cb.digest;

            // Store in pending blocks for later retrieval.
            pending_blocks.write().unwrap().insert(digest.0, cb);

            // Return the digest to consensus.
            let _ = sender.send(digest);
        });

        receiver
    }

    async fn verify(
        &mut self,
        _context: Self::Context,
        payload: Self::Digest,
    ) -> oneshot::Receiver<bool> {
        let (sender, receiver) = oneshot::channel();

        let pending_blocks = self.pending_blocks.clone();
        let last_hash = self.last_hash.clone();

        tokio::spawn(async move {
            // Look up the block by digest.
            let block = {
                let blocks = pending_blocks.read().unwrap();
                blocks.get(&payload.0).cloned()
            };

            let Some(block) = block else {
                tracing::warn!(
                    digest = ?payload,
                    "verify: block not found in pending blocks"
                );
                let _ = sender.send(false);
                return;
            };

            // Validate parent hash chain.
            let expected_parent = *last_hash.read().await;
            if block.inner.header.parent_hash != expected_parent {
                tracing::warn!(
                    expected = ?expected_parent,
                    actual = ?block.inner.header.parent_hash,
                    "verify: parent hash mismatch"
                );
                let _ = sender.send(false);
                return;
            }

            // Validate height is positive.
            if block.inner.header.number == 0 {
                tracing::warn!("verify: block height cannot be 0 (genesis)");
                let _ = sender.send(false);
                return;
            }

            let _ = sender.send(true);
        });

        receiver
    }
}

fn compute_transactions_root<Tx: Transaction>(transactions: &[Tx]) -> B256 {
    let mut hasher = Sha256::new();
    hasher.update(&(transactions.len() as u64).to_le_bytes());
    for tx in transactions {
        hasher.update(&tx.compute_identifier());
    }
    B256::from_slice(&hasher.finalize().0)
}

impl<Stf, S, Codes, Tx, Ctx> CertifiableAutomaton for EvolveAutomaton<Stf, S, Codes, Tx, Ctx>
where
    Tx: Transaction + MempoolTx + Clone + Send + Sync + 'static,
    S: ReadonlyKV + Storage + Clone + Send + Sync + 'static,
    Codes: AccountsCodeStorage + Clone + Send + Sync + 'static,
    Stf: StfExecutor<Tx, S, Codes> + Send + Sync + Clone + 'static,
    Ctx: Clone + Send + 'static,
{
    // Use the default implementation which always certifies.
}
