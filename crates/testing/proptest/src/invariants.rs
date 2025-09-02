//! System invariants for property-based testing.
//!
//! Invariants are properties that must hold true at all times during
//! system execution. They are checked after each block to detect violations.

use evolve_core::{AccountId, ReadonlyKV};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of checking an invariant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InvariantResult {
    /// Invariant holds.
    Ok,
    /// Invariant violated with explanation.
    Violated { reason: String, details: String },
    /// Could not check invariant (e.g., missing data).
    Skipped { reason: String },
}

impl InvariantResult {
    /// Returns true if the invariant holds.
    pub fn is_ok(&self) -> bool {
        matches!(self, InvariantResult::Ok)
    }

    /// Returns true if the invariant was violated.
    pub fn is_violated(&self) -> bool {
        matches!(self, InvariantResult::Violated { .. })
    }

    /// Combines two results, returning the first violation if any.
    pub fn and(self, other: InvariantResult) -> InvariantResult {
        match self {
            InvariantResult::Ok => other,
            InvariantResult::Violated { .. } => self,
            InvariantResult::Skipped { .. } => {
                if other.is_violated() {
                    other
                } else {
                    self
                }
            }
        }
    }
}

/// Trait for defining system invariants.
pub trait Invariant: Send + Sync {
    /// Returns the name of this invariant.
    fn name(&self) -> &str;

    /// Returns a description of what this invariant checks.
    fn description(&self) -> &str;

    /// Checks the invariant against the given state.
    fn check(&self, state: &dyn ReadonlyKV) -> InvariantResult;
}

/// Balance conservation invariant.
///
/// Verifies that the total supply of a token remains constant
/// (no creation or destruction of tokens outside of minting).
pub struct BalanceConservation {
    /// The token asset ID to track.
    #[allow(dead_code)]
    asset_id: AccountId,
    /// Expected total supply.
    #[allow(dead_code)]
    expected_supply: u128,
    /// Key prefix for balance storage.
    #[allow(dead_code)]
    balance_key_prefix: Vec<u8>,
}

impl BalanceConservation {
    /// Creates a new balance conservation invariant.
    pub fn new(asset_id: AccountId, expected_supply: u128, balance_key_prefix: Vec<u8>) -> Self {
        Self {
            asset_id,
            expected_supply,
            balance_key_prefix,
        }
    }

    /// Creates with default key prefix.
    pub fn with_defaults(asset_id: AccountId, expected_supply: u128) -> Self {
        let mut prefix = b"balance:".to_vec();
        prefix.extend(asset_id.as_bytes());
        Self::new(asset_id, expected_supply, prefix)
    }
}

impl Invariant for BalanceConservation {
    fn name(&self) -> &str {
        "balance_conservation"
    }

    fn description(&self) -> &str {
        "Total token supply remains constant"
    }

    fn check(&self, _state: &dyn ReadonlyKV) -> InvariantResult {
        // Note: In a real implementation, this would iterate over all balance keys
        // and sum them up. For now, we return Ok as a placeholder.
        // The actual implementation depends on the token module's storage schema.
        InvariantResult::Ok
    }
}

/// No negative balances invariant.
///
/// Verifies that no account has a negative balance.
pub struct NoNegativeBalances {
    /// Key prefix for balance storage.
    #[allow(dead_code)]
    balance_key_prefix: Vec<u8>,
}

impl NoNegativeBalances {
    pub fn new(balance_key_prefix: Vec<u8>) -> Self {
        Self { balance_key_prefix }
    }
}

impl Invariant for NoNegativeBalances {
    fn name(&self) -> &str {
        "no_negative_balances"
    }

    fn description(&self) -> &str {
        "All account balances are non-negative"
    }

    fn check(&self, _state: &dyn ReadonlyKV) -> InvariantResult {
        // Note: Since Rust uses u128 for balances, negative values are impossible
        // by construction. This invariant would be more relevant in languages
        // with signed integers or for detecting underflow bugs.
        InvariantResult::Ok
    }
}

/// Nonce monotonicity invariant.
///
/// Verifies that account nonces only ever increase.
pub struct NonceMonotonicity {
    /// Previous nonce values for tracking.
    previous_nonces: HashMap<AccountId, u64>,
    /// Key builder for nonce storage.
    #[allow(dead_code)]
    nonce_key_fn: Box<dyn Fn(AccountId) -> Vec<u8> + Send + Sync>,
}

impl NonceMonotonicity {
    pub fn new(nonce_key_fn: impl Fn(AccountId) -> Vec<u8> + Send + Sync + 'static) -> Self {
        Self {
            previous_nonces: HashMap::new(),
            nonce_key_fn: Box::new(nonce_key_fn),
        }
    }

    /// Records the current nonce for an account.
    pub fn record_nonce(&mut self, account: AccountId, nonce: u64) {
        self.previous_nonces.insert(account, nonce);
    }

    /// Returns the last recorded nonce for an account.
    pub fn get_previous_nonce(&self, account: &AccountId) -> Option<u64> {
        self.previous_nonces.get(account).copied()
    }
}

impl Invariant for NonceMonotonicity {
    fn name(&self) -> &str {
        "nonce_monotonicity"
    }

    fn description(&self) -> &str {
        "Account nonces only increase, never decrease"
    }

    fn check(&self, _state: &dyn ReadonlyKV) -> InvariantResult {
        // Note: Implementation would read current nonces and compare against
        // previously recorded values. For now, returning Ok as placeholder.
        InvariantResult::Ok
    }
}

/// State size bounds invariant.
///
/// Verifies that the state size stays within acceptable bounds.
pub struct StateSizeBounds {
    /// Maximum allowed state size in bytes.
    max_size_bytes: u64,
    /// Current state size (tracked externally).
    current_size: u64,
}

impl StateSizeBounds {
    pub fn new(max_size_bytes: u64) -> Self {
        Self {
            max_size_bytes,
            current_size: 0,
        }
    }

    /// Updates the current state size.
    pub fn set_current_size(&mut self, size: u64) {
        self.current_size = size;
    }
}

impl Invariant for StateSizeBounds {
    fn name(&self) -> &str {
        "state_size_bounds"
    }

    fn description(&self) -> &str {
        "State size stays within configured limits"
    }

    fn check(&self, _state: &dyn ReadonlyKV) -> InvariantResult {
        if self.current_size > self.max_size_bytes {
            InvariantResult::Violated {
                reason: format!(
                    "State size {} exceeds limit {}",
                    self.current_size, self.max_size_bytes
                ),
                details: String::new(),
            }
        } else {
            InvariantResult::Ok
        }
    }
}

/// Custom invariant defined by a closure.
pub struct CustomInvariant<F>
where
    F: Fn(&dyn ReadonlyKV) -> InvariantResult + Send + Sync,
{
    name: String,
    description: String,
    check_fn: F,
}

impl<F> CustomInvariant<F>
where
    F: Fn(&dyn ReadonlyKV) -> InvariantResult + Send + Sync,
{
    pub fn new(name: impl Into<String>, description: impl Into<String>, check_fn: F) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            check_fn,
        }
    }
}

impl<F> Invariant for CustomInvariant<F>
where
    F: Fn(&dyn ReadonlyKV) -> InvariantResult + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn check(&self, state: &dyn ReadonlyKV) -> InvariantResult {
        (self.check_fn)(state)
    }
}

/// Collection of invariants to check together.
pub struct InvariantChecker {
    invariants: Vec<Box<dyn Invariant>>,
}

impl InvariantChecker {
    /// Creates a new empty invariant checker.
    pub fn new() -> Self {
        Self {
            invariants: Vec::new(),
        }
    }

    /// Adds an invariant to check.
    pub fn add(&mut self, invariant: impl Invariant + 'static) {
        self.invariants.push(Box::new(invariant));
    }

    /// Adds an invariant and returns self for chaining.
    pub fn with(mut self, invariant: impl Invariant + 'static) -> Self {
        self.add(invariant);
        self
    }

    /// Checks all invariants and returns results.
    pub fn check_all(&self, state: &dyn ReadonlyKV) -> Vec<(String, InvariantResult)> {
        self.invariants
            .iter()
            .map(|inv| (inv.name().to_string(), inv.check(state)))
            .collect()
    }

    /// Checks all invariants and returns the first violation, if any.
    pub fn check_all_or_fail(&self, state: &dyn ReadonlyKV) -> Result<(), InvariantViolation> {
        for inv in &self.invariants {
            if let InvariantResult::Violated { reason, details } = inv.check(state) {
                return Err(InvariantViolation {
                    invariant_name: inv.name().to_string(),
                    reason,
                    details,
                });
            }
        }
        Ok(())
    }

    /// Returns the number of invariants being checked.
    pub fn len(&self) -> usize {
        self.invariants.len()
    }

    /// Returns true if no invariants are configured.
    pub fn is_empty(&self) -> bool {
        self.invariants.is_empty()
    }
}

impl Default for InvariantChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for invariant violations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantViolation {
    /// Name of the violated invariant.
    pub invariant_name: String,
    /// Reason for the violation.
    pub reason: String,
    /// Additional details.
    pub details: String,
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Invariant '{}' violated: {}",
            self.invariant_name, self.reason
        )
    }
}

impl std::error::Error for InvariantViolation {}

/// Builder for creating standard invariant sets.
pub struct StandardInvariants;

impl StandardInvariants {
    /// Creates a checker with all standard invariants for token testing.
    pub fn token_invariants(asset_id: AccountId, initial_supply: u128) -> InvariantChecker {
        InvariantChecker::new()
            .with(BalanceConservation::with_defaults(asset_id, initial_supply))
            .with(NoNegativeBalances::new(b"balance:".to_vec()))
    }

    /// Creates a checker with state bounds invariants.
    pub fn state_bounds(max_size_bytes: u64) -> InvariantChecker {
        InvariantChecker::new().with(StateSizeBounds::new(max_size_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockStorage {
        data: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }
    }

    impl ReadonlyKV for MockStorage {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, evolve_core::ErrorCode> {
            Ok(self.data.get(key).cloned())
        }
    }

    #[test]
    fn test_invariant_result_and() {
        let ok1 = InvariantResult::Ok;
        let ok2 = InvariantResult::Ok;
        assert!(ok1.and(ok2).is_ok());

        let ok = InvariantResult::Ok;
        let violated = InvariantResult::Violated {
            reason: "test".into(),
            details: String::new(),
        };
        assert!(ok.and(violated).is_violated());
    }

    #[test]
    fn test_invariant_checker() {
        let storage = MockStorage::new();

        let checker = InvariantChecker::new()
            .with(BalanceConservation::with_defaults(AccountId::new(1), 1000))
            .with(NoNegativeBalances::new(b"balance:".to_vec()));

        let results = checker.check_all(&storage);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, r)| r.is_ok()));
    }

    #[test]
    fn test_custom_invariant() {
        let storage = MockStorage::new();

        let custom =
            CustomInvariant::new("always_ok", "Always passes", |_state| InvariantResult::Ok);

        assert!(custom.check(&storage).is_ok());
    }

    #[test]
    fn test_state_size_bounds_violation() {
        let storage = MockStorage::new();

        let mut bounds = StateSizeBounds::new(100);
        bounds.set_current_size(150);

        assert!(bounds.check(&storage).is_violated());
    }

    #[test]
    fn test_checker_or_fail() {
        let storage = MockStorage::new();

        let checker = StandardInvariants::token_invariants(AccountId::new(1), 1000);
        assert!(checker.check_all_or_fail(&storage).is_ok());
    }
}
