use alloy_primitives::B256;
use borsh::{BorshDeserialize, BorshSerialize};
use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Error as CodecError, Read, ReadExt, Write};
use commonware_consensus::types::Height;
use commonware_consensus::Heightable;
use commonware_cryptography::{Committable, Digestible, Hasher, Sha256};
use evolve_server::{Block, BlockHeader};

/// A consensus-aware wrapper around Evolve's Block.
///
/// Implements commonware traits (Heightable, Digestible, Committable, Codec)
/// so it can participate in simplex consensus.
#[derive(Debug, Clone)]
pub struct ConsensusBlock<Tx> {
    /// The inner evolve block.
    pub inner: Block<Tx>,
    /// Cached digest (SHA-256 of the header fields).
    pub digest: commonware_cryptography::sha256::Digest,
    /// Cached parent digest.
    pub parent_digest: commonware_cryptography::sha256::Digest,
}

impl<Tx: Clone> ConsensusBlock<Tx> {
    /// Create a new ConsensusBlock from an evolve Block.
    ///
    /// Computes and caches the block digest and parent digest.
    pub fn new(inner: Block<Tx>) -> Self {
        let digest = compute_block_digest(&inner);
        let parent_digest = hash_to_sha256_digest(inner.header.parent_hash);
        Self {
            inner,
            digest,
            parent_digest,
        }
    }

    /// Return the inner block hash as a B256.
    pub fn block_hash(&self) -> B256 {
        B256::from_slice(&self.digest.0)
    }
}

/// Compute a deterministic SHA-256 digest from block header fields.
fn compute_block_digest<Tx>(block: &Block<Tx>) -> commonware_cryptography::sha256::Digest {
    let mut hasher = Sha256::new();
    hasher.update(&block.header.number.to_le_bytes());
    hasher.update(&block.header.timestamp.to_le_bytes());
    hasher.update(block.header.parent_hash.as_slice());
    hasher.update(&block.header.gas_limit.to_le_bytes());
    hasher.update(block.header.beneficiary.as_slice());
    hasher.finalize()
}

/// Convert a B256 into a SHA-256 Digest (both are 32 bytes).
fn hash_to_sha256_digest(hash: B256) -> commonware_cryptography::sha256::Digest {
    commonware_cryptography::sha256::Digest(hash.0)
}

// -- Commonware trait implementations --

impl<Tx: Clone + Send + Sync + 'static> Heightable for ConsensusBlock<Tx> {
    fn height(&self) -> Height {
        Height::new(self.inner.header.number)
    }
}

impl<Tx: Clone + Send + Sync + 'static> Digestible for ConsensusBlock<Tx> {
    type Digest = commonware_cryptography::sha256::Digest;

    fn digest(&self) -> Self::Digest {
        self.digest
    }
}

impl<Tx: Clone + Send + Sync + 'static> Committable for ConsensusBlock<Tx> {
    type Commitment = commonware_cryptography::sha256::Digest;

    fn commitment(&self) -> Self::Commitment {
        self.digest
    }
}

/// Borsh-based wire format for consensus block serialization.
#[derive(BorshSerialize, BorshDeserialize)]
struct WireBlock {
    number: u64,
    timestamp: u64,
    parent_hash: [u8; 32],
    gas_limit: u64,
    gas_used: u64,
    beneficiary: [u8; 20],
    transactions_encoded: Vec<Vec<u8>>,
}

impl<Tx: Clone + Send + Sync + 'static + BorshSerialize> Write for ConsensusBlock<Tx> {
    fn write(&self, buf: &mut impl BufMut) {
        let wire = to_wire(&self.inner);
        let bytes = borsh::to_vec(&wire).expect("wire block serialization should not fail");
        // Write length-prefixed bytes.
        (bytes.len() as u32).write(buf);
        buf.put_slice(&bytes);
    }
}

impl<Tx: Clone + Send + Sync + 'static + BorshDeserialize> Read for ConsensusBlock<Tx> {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _cfg: &Self::Cfg) -> Result<Self, CodecError> {
        let len = u32::read(buf)? as usize;
        if buf.remaining() < len {
            return Err(CodecError::EndOfBuffer);
        }
        let bytes = buf.copy_to_bytes(len);
        let wire: WireBlock = borsh::from_slice(&bytes)
            .map_err(|_| CodecError::Invalid("ConsensusBlock", "borsh deserialization failed"))?;

        let transactions: Vec<Tx> = wire
            .transactions_encoded
            .iter()
            .map(|encoded| {
                borsh::from_slice(encoded).map_err(|_| {
                    CodecError::Invalid("ConsensusBlock", "tx borsh deserialization failed")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let header = BlockHeader {
            number: wire.number,
            timestamp: wire.timestamp,
            parent_hash: B256::from_slice(&wire.parent_hash),
            gas_limit: wire.gas_limit,
            gas_used: wire.gas_used,
            beneficiary: alloy_primitives::Address::from_slice(&wire.beneficiary),
            ..Default::default()
        };

        let block = Block::new(header, transactions);
        Ok(ConsensusBlock::new(block))
    }
}

impl<Tx: Clone + Send + Sync + 'static + BorshSerialize> EncodeSize for ConsensusBlock<Tx> {
    fn encode_size(&self) -> usize {
        let wire = to_wire(&self.inner);
        let bytes = borsh::to_vec(&wire).expect("wire block serialization should not fail");
        // u32 length prefix + payload
        4 + bytes.len()
    }
}

fn to_wire<Tx: BorshSerialize>(inner: &Block<Tx>) -> WireBlock {
    WireBlock {
        number: inner.header.number,
        timestamp: inner.header.timestamp,
        parent_hash: inner.header.parent_hash.0,
        gas_limit: inner.header.gas_limit,
        gas_used: inner.header.gas_used,
        beneficiary: inner.header.beneficiary.0 .0,
        transactions_encoded: inner
            .transactions
            .iter()
            .map(|tx| borsh::to_vec(tx).expect("tx serialization should not fail"))
            .collect(),
    }
}

impl<Tx: Clone + Send + Sync + 'static + BorshSerialize + BorshDeserialize>
    commonware_consensus::Block for ConsensusBlock<Tx>
{
    fn parent(&self) -> Self::Commitment {
        self.parent_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::{DecodeExt, Encode};

    #[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
    struct TestTx {
        data: Vec<u8>,
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let header = BlockHeader::new(42, 1000, B256::repeat_byte(0xAA));
        let txs = vec![
            TestTx {
                data: vec![1, 2, 3],
            },
            TestTx {
                data: vec![4, 5, 6],
            },
        ];
        let block = Block::new(header, txs);
        let consensus_block = ConsensusBlock::new(block);

        let encoded = consensus_block.encode();
        let decoded = ConsensusBlock::<TestTx>::decode(encoded).unwrap();

        assert_eq!(
            decoded.inner.header.number,
            consensus_block.inner.header.number
        );
        assert_eq!(
            decoded.inner.header.timestamp,
            consensus_block.inner.header.timestamp
        );
        assert_eq!(
            decoded.inner.header.parent_hash,
            consensus_block.inner.header.parent_hash
        );
        assert_eq!(
            decoded.inner.transactions,
            consensus_block.inner.transactions
        );
        assert_eq!(decoded.digest, consensus_block.digest);
    }

    #[test]
    fn test_digest_determinism() {
        let header = BlockHeader::new(10, 500, B256::repeat_byte(0xBB));
        let txs = vec![TestTx {
            data: vec![7, 8, 9],
        }];

        let block1 = Block::new(header.clone(), txs.clone());
        let block2 = Block::new(header, txs);

        let cb1 = ConsensusBlock::new(block1);
        let cb2 = ConsensusBlock::new(block2);

        assert_eq!(cb1.digest, cb2.digest);
    }

    #[test]
    fn test_heightable() {
        let header = BlockHeader::new(99, 0, B256::ZERO);
        let block = Block::<TestTx>::new(header, vec![]);
        let cb = ConsensusBlock::new(block);

        assert_eq!(cb.height(), Height::new(99));
    }

    #[test]
    fn test_parent_returns_parent_hash() {
        let parent = B256::repeat_byte(0xCC);
        let header = BlockHeader::new(5, 0, parent);
        let block = Block::<TestTx>::new(header, vec![]);
        let cb = ConsensusBlock::new(block);

        let parent_digest = <ConsensusBlock<TestTx> as commonware_consensus::Block>::parent(&cb);
        assert_eq!(parent_digest.0, parent.0);
    }

    #[test]
    fn test_different_blocks_different_digests() {
        let block1 = Block::<TestTx>::new(BlockHeader::new(1, 100, B256::ZERO), vec![]);
        let block2 = Block::<TestTx>::new(BlockHeader::new(2, 100, B256::ZERO), vec![]);

        let cb1 = ConsensusBlock::new(block1);
        let cb2 = ConsensusBlock::new(block2);

        assert_ne!(cb1.digest, cb2.digest);
    }
}
