//! Breakpoint system for debugging.
//!
//! Breakpoints allow pausing execution at specific points during replay,
//! enabling inspection and analysis.

use crate::trace::{StateSnapshot, TraceEvent};
use evolve_core::AccountId;
use serde::{Deserialize, Serialize};

/// Convert AccountId to serializable bytes.
fn account_to_bytes(id: AccountId) -> [u8; 16] {
    let bytes = id.as_bytes();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[..16]);
    arr
}

/// A breakpoint condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Breakpoint {
    /// Break at a specific block height.
    OnBlock(u64),

    /// Break at a specific transaction.
    OnTx([u8; 32]),

    /// Break on any transaction involving this account (stored as bytes).
    OnAccount([u8; 16]),

    /// Break when a specific storage key is modified.
    OnStorageKey(Vec<u8>),

    /// Break on any error.
    OnError,

    /// Break when gas is exhausted.
    OnGasExhausted,

    /// Break on state changes to keys matching a prefix.
    OnStoragePrefix(Vec<u8>),

    /// Break on events with matching name.
    OnEvent(String),

    /// Break at a specific event index.
    OnEventIndex(usize),

    /// Composite: break if ALL conditions match.
    All(Vec<Breakpoint>),

    /// Composite: break if ANY condition matches.
    Any(Vec<Breakpoint>),

    /// Negation: break if condition does NOT match.
    Not(Box<Breakpoint>),
}

impl Breakpoint {
    /// Creates a breakpoint that triggers on any transaction from or to an account.
    pub fn on_account(account: AccountId) -> Self {
        Breakpoint::OnAccount(account_to_bytes(account))
    }

    /// Creates a breakpoint that triggers on a specific block.
    pub fn on_block(height: u64) -> Self {
        Breakpoint::OnBlock(height)
    }

    /// Creates a breakpoint that triggers on a specific transaction.
    pub fn on_tx(tx_id: [u8; 32]) -> Self {
        Breakpoint::OnTx(tx_id)
    }

    /// Creates a breakpoint that triggers when a storage key is modified.
    pub fn on_storage_key(key: impl Into<Vec<u8>>) -> Self {
        Breakpoint::OnStorageKey(key.into())
    }

    /// Creates a breakpoint that triggers on any error.
    pub fn on_error() -> Self {
        Breakpoint::OnError
    }

    /// Creates a composite breakpoint requiring all conditions.
    pub fn all(breakpoints: Vec<Breakpoint>) -> Self {
        Breakpoint::All(breakpoints)
    }

    /// Creates a composite breakpoint requiring any condition.
    pub fn any(breakpoints: Vec<Breakpoint>) -> Self {
        Breakpoint::Any(breakpoints)
    }

    /// Creates a negated breakpoint.
    pub fn not(breakpoint: Breakpoint) -> Self {
        Breakpoint::Not(Box::new(breakpoint))
    }

    /// Checks if this breakpoint matches the given event and state.
    pub fn matches(&self, event: &TraceEvent, state: &StateSnapshot) -> bool {
        match self {
            Breakpoint::OnBlock(height) => {
                matches!(event, TraceEvent::BlockStart { height: h, .. } if h == height)
            }

            Breakpoint::OnTx(tx_id) => {
                matches!(event, TraceEvent::TxStart { tx_id: id, .. } if id == tx_id)
            }

            Breakpoint::OnAccount(account) => match event {
                TraceEvent::TxStart {
                    sender, recipient, ..
                } => sender == account || recipient == account,
                TraceEvent::Call { from, to, .. } => from == account || to == account,
                _ => false,
            },

            Breakpoint::OnStorageKey(key) => {
                matches!(event, TraceEvent::StateChange { key: k, .. } if k == key)
            }

            Breakpoint::OnStoragePrefix(prefix) => {
                matches!(event, TraceEvent::StateChange { key, .. } if key.starts_with(prefix))
            }

            Breakpoint::OnError => matches!(event, TraceEvent::Error { .. }),

            Breakpoint::OnGasExhausted => {
                matches!(event, TraceEvent::GasCharge { remaining: 0, .. })
            }

            Breakpoint::OnEvent(name) => {
                matches!(event, TraceEvent::EventEmitted { name: n, .. } if n == name)
            }

            Breakpoint::OnEventIndex(idx) => event.event_index() == *idx,

            Breakpoint::All(breakpoints) => breakpoints.iter().all(|bp| bp.matches(event, state)),

            Breakpoint::Any(breakpoints) => breakpoints.iter().any(|bp| bp.matches(event, state)),

            Breakpoint::Not(bp) => !bp.matches(event, state),
        }
    }

    /// Returns a human-readable description of this breakpoint.
    pub fn description(&self) -> String {
        match self {
            Breakpoint::OnBlock(height) => format!("block {height}"),
            Breakpoint::OnTx(tx_id) => format!("tx {:02x}{:02x}...", tx_id[0], tx_id[1]),
            Breakpoint::OnAccount(account) => format!("account {:02x?}", account),
            Breakpoint::OnStorageKey(key) => {
                if let Ok(s) = std::str::from_utf8(key) {
                    format!("storage key \"{s}\"")
                } else {
                    format!("storage key {:?}", key)
                }
            }
            Breakpoint::OnStoragePrefix(prefix) => {
                if let Ok(s) = std::str::from_utf8(prefix) {
                    format!("storage prefix \"{s}\"")
                } else {
                    format!("storage prefix {:?}", prefix)
                }
            }
            Breakpoint::OnError => "any error".to_string(),
            Breakpoint::OnGasExhausted => "gas exhausted".to_string(),
            Breakpoint::OnEvent(name) => format!("event \"{name}\""),
            Breakpoint::OnEventIndex(idx) => format!("event index {idx}"),
            Breakpoint::All(bps) => {
                let descs: Vec<_> = bps.iter().map(|bp| bp.description()).collect();
                format!("all of [{}]", descs.join(", "))
            }
            Breakpoint::Any(bps) => {
                let descs: Vec<_> = bps.iter().map(|bp| bp.description()).collect();
                format!("any of [{}]", descs.join(", "))
            }
            Breakpoint::Not(bp) => format!("not ({})", bp.description()),
        }
    }
}

/// Builder for creating complex breakpoint conditions.
pub struct BreakpointBuilder {
    conditions: Vec<Breakpoint>,
    mode: BuilderMode,
}

#[derive(Clone, Copy)]
enum BuilderMode {
    All,
    Any,
}

impl BreakpointBuilder {
    /// Creates a builder that requires ALL conditions.
    pub fn all() -> Self {
        Self {
            conditions: Vec::new(),
            mode: BuilderMode::All,
        }
    }

    /// Creates a builder that requires ANY condition.
    pub fn any() -> Self {
        Self {
            conditions: Vec::new(),
            mode: BuilderMode::Any,
        }
    }

    /// Adds a condition.
    pub fn with(mut self, bp: Breakpoint) -> Self {
        self.conditions.push(bp);
        self
    }

    /// Adds a block condition.
    pub fn on_block(self, height: u64) -> Self {
        self.with(Breakpoint::OnBlock(height))
    }

    /// Adds a transaction condition.
    pub fn on_tx(self, tx_id: [u8; 32]) -> Self {
        self.with(Breakpoint::OnTx(tx_id))
    }

    /// Adds an account condition.
    pub fn on_account(self, account: AccountId) -> Self {
        self.with(Breakpoint::OnAccount(account_to_bytes(account)))
    }

    /// Adds a storage key condition.
    pub fn on_storage_key(self, key: impl Into<Vec<u8>>) -> Self {
        self.with(Breakpoint::OnStorageKey(key.into()))
    }

    /// Adds an error condition.
    pub fn on_error(self) -> Self {
        self.with(Breakpoint::OnError)
    }

    /// Builds the final breakpoint.
    pub fn build(self) -> Breakpoint {
        if self.conditions.len() == 1 {
            self.conditions.into_iter().next().unwrap()
        } else {
            match self.mode {
                BuilderMode::All => Breakpoint::All(self.conditions),
                BuilderMode::Any => Breakpoint::Any(self.conditions),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block_start(height: u64) -> TraceEvent {
        TraceEvent::BlockStart {
            height,
            timestamp_ms: 1000,
            event_index: 0,
        }
    }

    fn make_tx_start(tx_id: [u8; 32], sender: AccountId, recipient: AccountId) -> TraceEvent {
        TraceEvent::TxStart {
            tx_id,
            sender: account_to_bytes(sender),
            recipient: account_to_bytes(recipient),
            event_index: 0,
        }
    }

    fn make_state_change(key: Vec<u8>) -> TraceEvent {
        TraceEvent::StateChange {
            key,
            old_value: None,
            new_value: Some(b"value".to_vec()),
            event_index: 0,
        }
    }

    fn make_error() -> TraceEvent {
        TraceEvent::Error {
            code: 1,
            message: "test error".to_string(),
            event_index: 0,
        }
    }

    #[test]
    fn test_on_block() {
        let bp = Breakpoint::on_block(5);
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_block_start(5), &state));
        assert!(!bp.matches(&make_block_start(6), &state));
    }

    #[test]
    fn test_on_account() {
        let account = AccountId::new(42);
        let other = AccountId::new(100);
        let bp = Breakpoint::on_account(account);
        let state = StateSnapshot::empty();

        // Matches when account is sender
        assert!(bp.matches(&make_tx_start([0; 32], account, other), &state));

        // Matches when account is recipient
        assert!(bp.matches(&make_tx_start([0; 32], other, account), &state));

        // Doesn't match when account not involved
        assert!(!bp.matches(&make_tx_start([0; 32], other, other), &state));
    }

    #[test]
    fn test_on_storage_key() {
        let bp = Breakpoint::on_storage_key(b"my_key".to_vec());
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_state_change(b"my_key".to_vec()), &state));
        assert!(!bp.matches(&make_state_change(b"other_key".to_vec()), &state));
    }

    #[test]
    fn test_on_storage_prefix() {
        let bp = Breakpoint::OnStoragePrefix(b"balance:".to_vec());
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_state_change(b"balance:user1".to_vec()), &state));
        assert!(!bp.matches(&make_state_change(b"nonce:user1".to_vec()), &state));
    }

    #[test]
    fn test_on_error() {
        let bp = Breakpoint::on_error();
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_error(), &state));
        assert!(!bp.matches(&make_block_start(0), &state));
    }

    #[test]
    fn test_all_composite() {
        let bp = Breakpoint::all(vec![
            Breakpoint::OnBlock(5),
            Breakpoint::OnBlock(5), // Same condition twice
        ]);
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_block_start(5), &state));
        assert!(!bp.matches(&make_block_start(6), &state));
    }

    #[test]
    fn test_any_composite() {
        let bp = Breakpoint::any(vec![Breakpoint::OnBlock(5), Breakpoint::OnBlock(6)]);
        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_block_start(5), &state));
        assert!(bp.matches(&make_block_start(6), &state));
        assert!(!bp.matches(&make_block_start(7), &state));
    }

    #[test]
    fn test_not() {
        let bp = Breakpoint::not(Breakpoint::OnBlock(5));
        let state = StateSnapshot::empty();

        assert!(!bp.matches(&make_block_start(5), &state));
        assert!(bp.matches(&make_block_start(6), &state));
    }

    #[test]
    fn test_builder() {
        let bp = BreakpointBuilder::any()
            .on_block(5)
            .on_block(10)
            .on_error()
            .build();

        let state = StateSnapshot::empty();

        assert!(bp.matches(&make_block_start(5), &state));
        assert!(bp.matches(&make_block_start(10), &state));
        assert!(bp.matches(&make_error(), &state));
        assert!(!bp.matches(&make_block_start(7), &state));
    }

    #[test]
    fn test_description() {
        let bp = Breakpoint::on_block(5);
        assert_eq!(bp.description(), "block 5");

        let bp = Breakpoint::on_error();
        assert_eq!(bp.description(), "any error");

        let bp = Breakpoint::on_storage_key(b"test");
        assert!(bp.description().contains("test"));
    }
}
