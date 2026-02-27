//! Core genesis types.

use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::{AccountId, InvokeRequest};
use serde::{Deserialize, Serialize};

/// Default block gas limit (30 million).
pub const DEFAULT_BLOCK_GAS_LIMIT: u64 = 30_000_000;

/// Consensus parameters stored in genesis and state.
///
/// These parameters are critical for consensus - all nodes must agree on them
/// to validate blocks correctly. They are set at genesis and stored in state
/// so syncing nodes can read them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct ConsensusParams {
    /// Maximum gas allowed per block.
    /// Transactions that would exceed this limit are not included in the block.
    pub block_gas_limit: u64,
}

impl Default for ConsensusParams {
    fn default() -> Self {
        Self {
            block_gas_limit: DEFAULT_BLOCK_GAS_LIMIT,
        }
    }
}

impl ConsensusParams {
    /// Create new consensus params with the given block gas limit.
    pub fn new(block_gas_limit: u64) -> Self {
        Self { block_gas_limit }
    }
}

/// A genesis transaction.
///
/// Genesis transactions are similar to regular transactions but:
/// - Have no signature (authentication is skipped)
/// - Use infinite gas
/// - Are only valid at block height 0
#[derive(Debug, Clone)]
pub struct GenesisTx {
    /// Optional identifier for this transaction.
    /// Used for referencing created accounts in later transactions.
    pub id: Option<String>,

    /// The sender account ID.
    /// For genesis, this is typically the system account (AccountId::from_bytes([0u8; 32]))
    /// or a virtual sender specified in the genesis file.
    pub sender: AccountId,

    /// The recipient account ID.
    /// This is the account that will handle the message.
    pub recipient: AccountId,

    /// The invoke request containing the message to send.
    pub request: InvokeRequest,
}

impl GenesisTx {
    /// Creates a new genesis transaction.
    pub fn new(sender: AccountId, recipient: AccountId, request: InvokeRequest) -> Self {
        Self {
            id: None,
            sender,
            recipient,
            request,
        }
    }

    /// Creates a new genesis transaction with an ID.
    pub fn with_id(
        id: impl Into<String>,
        sender: AccountId,
        recipient: AccountId,
        request: InvokeRequest,
    ) -> Self {
        Self {
            id: Some(id.into()),
            sender,
            recipient,
            request,
        }
    }
}
