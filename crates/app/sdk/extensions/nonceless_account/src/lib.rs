//! Nonceless Account module for timestamp-based replay protection.
//!
//! This module provides account-level replay protection using timestamps
//! instead of sequential nonces, enabling high-throughput parallel transactions.
//!
//! ## Replay Protection
//!
//! Timestamp monotonicity: Each tx must have `timestamp > last_seen_timestamp`.
//!
//! ## Throughput
//!
//! With millisecond timestamps and strict ordering (`>`), max throughput is
//! ~1000 tx/sec per account.
//!
//! ## Usage
//!
//! This module only handles replay protection state. Actual value transfers
//! are handled separately at the execution layer (similar to ETH nonce accounts).

use evolve_core::account_impl;

#[account_impl(NoncelessAccount)]
pub mod account {
    use evolve_collections::item::Item;
    use evolve_core::{
        define_error, AccountId, Environment, EnvironmentQuery, SdkResult, ERR_UNAUTHORIZED,
    };
    use evolve_macros::{exec, init, query};

    define_error!(
        ERR_TIMESTAMP_REPLAY,
        0x1,
        "timestamp must be greater than last seen"
    );

    /// Account state for timestamp-based replay protection.
    #[derive(evolve_core::AccountState)]
    pub struct NoncelessAccount {
        /// Last seen timestamp (milliseconds) for replay protection.
        /// Any new transaction must have timestamp > last_seen_timestamp.
        #[storage(0)]
        pub last_seen_timestamp: Item<u64>,

        /// Owner address that can authorize transactions.
        #[storage(1)]
        pub owner: Item<AccountId>,
    }

    impl NoncelessAccount {
        /// Initialize a new nonceless account.
        #[init]
        pub fn initialize(&self, owner: AccountId, env: &mut dyn Environment) -> SdkResult<()> {
            self.owner.set(&owner, env)?;
            self.last_seen_timestamp.set(&0, env)?;
            Ok(())
        }

        /// Verify timestamp for replay protection, then update state.
        ///
        /// This is the core replay protection check. Call this before executing
        /// any state-changing operation for the account.
        ///
        /// # Arguments
        /// * `timestamp` - Transaction timestamp in milliseconds (must be > last_seen)
        #[exec]
        pub fn verify_and_update(
            &self,
            timestamp: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // 1. Verify sender is owner
            let owner = self.owner.get(env)?;
            if env.sender() != owner {
                return Err(ERR_UNAUTHORIZED);
            }

            // 2. Verify timestamp is strictly greater than last seen
            let last_ts = self.last_seen_timestamp.may_get(env)?.unwrap_or(0);
            if timestamp <= last_ts {
                return Err(ERR_TIMESTAMP_REPLAY);
            }

            // 3. Update last seen timestamp
            self.last_seen_timestamp.set(&timestamp, env)?;

            Ok(())
        }

        /// Get the last seen timestamp (milliseconds).
        #[query]
        pub fn get_last_timestamp(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u64> {
            self.last_seen_timestamp
                .may_get(env)
                .map(|t| t.unwrap_or(0))
        }

        /// Get the owner of this account.
        #[query]
        pub fn get_owner(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<AccountId> {
            self.owner.get(env)
        }
    }
}

#[cfg(test)]
mod tests {
    use evolve_core::{AccountId, ERR_UNAUTHORIZED};
    use evolve_testing::MockEnv;

    use crate::account::{NoncelessAccount, ERR_TIMESTAMP_REPLAY};

    fn setup_account(owner_id: u128) -> (NoncelessAccount, MockEnv) {
        let contract_address = AccountId::new(1);
        let owner = AccountId::new(owner_id);
        let mut env = MockEnv::new(contract_address, owner);

        let account = NoncelessAccount::default();
        account
            .initialize(owner, &mut env)
            .expect("initialization failed");

        (account, env)
    }

    #[test]
    fn test_initialize() {
        let (account, mut env) = setup_account(42);

        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 0);
        assert_eq!(account.get_owner(&mut env).unwrap(), AccountId::new(42));
    }

    #[test]
    fn test_verify_and_update_success() {
        let (account, mut env) = setup_account(42);

        account.verify_and_update(1000, &mut env).unwrap();
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 1000);

        account.verify_and_update(1500, &mut env).unwrap();
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 1500);
    }

    #[test]
    fn test_timestamp_replay_rejected() {
        let (account, mut env) = setup_account(42);

        // First update succeeds
        account.verify_and_update(1000, &mut env).unwrap();

        // Same timestamp fails
        let result = account.verify_and_update(1000, &mut env);
        assert!(matches!(result, Err(e) if e == ERR_TIMESTAMP_REPLAY));
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 1000);

        // Lower timestamp fails
        let result = account.verify_and_update(500, &mut env);
        assert!(matches!(result, Err(e) if e == ERR_TIMESTAMP_REPLAY));
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 1000);

        // Higher timestamp succeeds
        let result = account.verify_and_update(2000, &mut env);
        assert!(result.is_ok());
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 2000);
    }

    #[test]
    fn test_unauthorized() {
        let (account, mut env) = setup_account(42);

        // Change sender to non-owner
        env = env.with_sender(AccountId::new(999));

        let result = account.verify_and_update(1000, &mut env);
        assert!(matches!(result, Err(e) if e == ERR_UNAUTHORIZED));
        assert_eq!(account.get_last_timestamp(&mut env).unwrap(), 0);
    }
}
