//! Ethereum-compatible block types.
//!
//! These types are designed for Ethereum JSON-RPC compatibility while
//! working with the Evolve STF.

use alloy_primitives::{Address, Bloom, Bytes, B256, B64, U256};
use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::encoding::Encodable;
use evolve_core::BlockContext;
use evolve_stf_traits::Block as BlockTrait;

/// Ethereum-compatible block header.
#[derive(Debug, Clone, Default)]
pub struct BlockHeader {
    /// Parent block hash.
    pub parent_hash: B256,
    /// Ommers hash (uncles) - always empty hash for non-PoW.
    pub ommers_hash: B256,
    /// Beneficiary (miner/validator/proposer).
    pub beneficiary: Address,
    /// State root after execution.
    pub state_root: B256,
    /// Transactions trie root.
    pub transactions_root: B256,
    /// Receipts trie root.
    pub receipts_root: B256,
    /// Bloom filter for logs.
    pub logs_bloom: Bloom,
    /// Difficulty - always 0 for non-PoW.
    pub difficulty: U256,
    /// Block number (height).
    pub number: u64,
    /// Gas limit for the block.
    pub gas_limit: u64,
    /// Total gas used by all transactions.
    pub gas_used: u64,
    /// Block timestamp (Unix seconds).
    pub timestamp: u64,
    /// Extra data (max 32 bytes).
    pub extra_data: Bytes,
    /// Mix hash - always zero for non-PoW.
    pub mix_hash: B256,
    /// Nonce - always zero for non-PoW.
    pub nonce: B64,
    /// Base fee per gas (EIP-1559).
    pub base_fee_per_gas: Option<u64>,
    /// Withdrawals root (EIP-4895) - optional.
    pub withdrawals_root: Option<B256>,
    /// Blob gas used (EIP-4844) - optional.
    pub blob_gas_used: Option<u64>,
    /// Excess blob gas (EIP-4844) - optional.
    pub excess_blob_gas: Option<u64>,
    /// Parent beacon block root (EIP-4788) - optional.
    pub parent_beacon_block_root: Option<B256>,
}

impl BlockHeader {
    /// Create a new block header with minimal required fields.
    pub fn new(number: u64, timestamp: u64, parent_hash: B256) -> Self {
        Self {
            number,
            timestamp,
            parent_hash,
            // Empty ommers hash (keccak256 of empty list)
            ommers_hash: B256::from_slice(&[
                0x1d, 0xcc, 0x4d, 0xe8, 0xde, 0xc7, 0x5d, 0x7a, 0xab, 0x85, 0xb5, 0x67, 0xb6, 0xcc,
                0xd4, 0x1a, 0xd3, 0x12, 0x45, 0x1b, 0x94, 0x8a, 0x74, 0x13, 0xf0, 0xa1, 0x42, 0xfd,
                0x40, 0xd4, 0x93, 0x47,
            ]),
            gas_limit: 30_000_000, // 30M default
            ..Default::default()
        }
    }

    /// Set the beneficiary (proposer/validator).
    pub fn with_beneficiary(mut self, beneficiary: Address) -> Self {
        self.beneficiary = beneficiary;
        self
    }

    /// Set gas limit.
    pub fn with_gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Set base fee (EIP-1559).
    pub fn with_base_fee(mut self, base_fee: u64) -> Self {
        self.base_fee_per_gas = Some(base_fee);
        self
    }
}

/// Ethereum-compatible block with header and transactions.
#[derive(Debug, Clone)]
pub struct Block<Tx> {
    /// Block header.
    pub header: BlockHeader,
    /// Transactions in the block.
    pub transactions: Vec<Tx>,
}

impl<Tx> Block<Tx> {
    /// Create a new block.
    pub fn new(header: BlockHeader, transactions: Vec<Tx>) -> Self {
        Self {
            header,
            transactions,
        }
    }

    /// Create a block for testing/development.
    pub fn for_testing(height: u64, transactions: Vec<Tx>) -> Self {
        Self {
            header: BlockHeader::new(height, 0, B256::ZERO),
            transactions,
        }
    }

    /// Get the block number.
    pub fn number(&self) -> u64 {
        self.header.number
    }

    /// Get the block timestamp.
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }

    /// Get the parent hash.
    pub fn parent_hash(&self) -> B256 {
        self.header.parent_hash
    }

    /// Get the beneficiary.
    pub fn beneficiary(&self) -> Address {
        self.header.beneficiary
    }

    /// Get gas limit.
    pub fn gas_limit(&self) -> u64 {
        self.header.gas_limit
    }

    /// Get gas used.
    pub fn gas_used(&self) -> u64 {
        self.header.gas_used
    }
}

impl<Tx> Default for Block<Tx> {
    fn default() -> Self {
        Self {
            header: BlockHeader::default(),
            transactions: Vec::new(),
        }
    }
}

// Implement the STF Block trait
impl<Tx> BlockTrait<Tx> for Block<Tx> {
    fn context(&self) -> BlockContext {
        BlockContext::new(self.header.number, self.header.timestamp)
    }

    fn txs(&self) -> &[Tx] {
        &self.transactions
    }

    fn gas_limit(&self) -> u64 {
        self.header.gas_limit
    }
}

/// Borsh-serializable block snapshot for archive storage.
///
/// Contains the essential block data needed for P2P sync and block serving.
/// Transactions are stored as their encoded byte representation.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct ArchivedBlock {
    pub number: u64,
    pub timestamp: u64,
    pub parent_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub block_hash: [u8; 32],
    pub gas_limit: u64,
    pub gas_used: u64,
    pub transactions: Vec<Vec<u8>>,
}

impl<Tx: Encodable> Block<Tx> {
    /// Create an archived snapshot of this block for storage.
    ///
    /// Encodes each transaction via `Encodable::encode()` and captures
    /// the block hash, state root, and gas used from execution results.
    pub fn to_archived(
        &self,
        block_hash: B256,
        state_root: B256,
        gas_used: u64,
    ) -> Result<ArchivedBlock, evolve_core::ErrorCode> {
        let transactions: Vec<Vec<u8>> = self
            .transactions
            .iter()
            .map(|tx| tx.encode())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ArchivedBlock {
            number: self.header.number,
            timestamp: self.header.timestamp,
            parent_hash: self.header.parent_hash.0,
            state_root: state_root.0,
            block_hash: block_hash.0,
            gas_limit: self.header.gas_limit,
            gas_used,
            transactions,
        })
    }
}

/// Builder for creating blocks.
#[derive(Debug)]
pub struct BlockBuilder<Tx> {
    header: BlockHeader,
    transactions: Vec<Tx>,
}

impl<Tx> Default for BlockBuilder<Tx> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Tx> BlockBuilder<Tx> {
    /// Create a new block builder.
    pub fn new() -> Self {
        Self {
            header: BlockHeader::default(),
            transactions: Vec::new(),
        }
    }

    /// Set the block number.
    pub fn number(mut self, number: u64) -> Self {
        self.header.number = number;
        self
    }

    /// Set the timestamp.
    pub fn timestamp(mut self, timestamp: u64) -> Self {
        self.header.timestamp = timestamp;
        self
    }

    /// Set the parent hash.
    pub fn parent_hash(mut self, hash: B256) -> Self {
        self.header.parent_hash = hash;
        self
    }

    /// Set the beneficiary.
    pub fn beneficiary(mut self, addr: Address) -> Self {
        self.header.beneficiary = addr;
        self
    }

    /// Set gas limit.
    pub fn gas_limit(mut self, limit: u64) -> Self {
        self.header.gas_limit = limit;
        self
    }

    /// Set base fee.
    pub fn base_fee(mut self, fee: u64) -> Self {
        self.header.base_fee_per_gas = Some(fee);
        self
    }

    /// Set extra data.
    pub fn extra_data(mut self, data: Bytes) -> Self {
        self.header.extra_data = data;
        self
    }

    /// Add a transaction.
    pub fn transaction(mut self, tx: Tx) -> Self {
        self.transactions.push(tx);
        self
    }

    /// Add multiple transactions.
    pub fn transactions(mut self, txs: impl IntoIterator<Item = Tx>) -> Self {
        self.transactions.extend(txs);
        self
    }

    /// Build the block.
    pub fn build(self) -> Block<Tx> {
        Block {
            header: self.header,
            transactions: self.transactions,
        }
    }
}
