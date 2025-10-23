//! Core genesis types.

use evolve_core::{AccountId, InvokeRequest};

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
    /// For genesis, this is typically the system account (AccountId::new(0))
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
