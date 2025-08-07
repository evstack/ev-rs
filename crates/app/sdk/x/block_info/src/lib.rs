use evolve_core::account_impl;

pub mod server;

#[account_impl(BlockInfo)]
pub mod account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_macros::{exec, init, query};

    /// A simple struct to store both block height and time in one query.
    #[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
    pub struct BlockDetails {
        pub height: u64,
        pub time_unix_ms: u64,
    }

    pub struct BlockInfo {
        pub time_unix_ms: Item<u64>,
        pub height: Item<u64>,
    }

    impl Default for BlockInfo {
        fn default() -> Self {
            Self::new()
        }
    }

    impl BlockInfo {
        pub const fn new() -> Self {
            Self {
                time_unix_ms: Item::new(0),
                height: Item::new(1),
            }
        }

        /// Initialize the contract with block height and time.
        #[init]
        pub fn initialize(
            &self,
            current_block: u64,
            current_time_unix_ms: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.height.set(&current_block, env)?;
            self.time_unix_ms.set(&current_time_unix_ms, env)?;
            Ok(())
        }

        /// Only the `RUNTIME_ACCOUNT_ID` may update block info.
        #[exec]
        pub fn set_block_info(
            &self,
            current_block: u64,
            current_time_unix_ms: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            self.height.set(&current_block, env)?;
            self.time_unix_ms.set(&current_time_unix_ms, env)?;

            Ok(())
        }

        /// Query the current height only.
        #[query]
        pub fn get_height(&self, env: &dyn Environment) -> SdkResult<u64> {
            self.height.get(env)
        }

        /// **New**: Query the current time only.
        #[query]
        pub fn get_time_unix_ms(&self, env: &dyn Environment) -> SdkResult<u64> {
            self.time_unix_ms.get(env)
        }

        /// **New**: Query both the height and time, returning a custom struct.
        #[query]
        pub fn get_block_details(&self, env: &dyn Environment) -> SdkResult<BlockDetails> {
            Ok(BlockDetails {
                height: self.height.get(env)?,
                time_unix_ms: self.time_unix_ms.get(env)?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::account::BlockInfo;
    use evolve_core::{runtime_api::RUNTIME_ACCOUNT_ID, AccountId, ERR_UNAUTHORIZED};
    use evolve_testing::MockEnv;

    /// A helper function to set up a mock environment and default BlockInfo instance.
    /// `sender_id` determines the initial `env.sender()`.
    fn setup_block_info(
        sender_id: AccountId,
        initial_block: u64,
        initial_time: u64,
    ) -> (BlockInfo, MockEnv) {
        // We'll arbitrarily choose the contract's address to be 1 for mock environment.
        let contract_address = AccountId::new(1);

        let mut env = MockEnv::new(contract_address, sender_id);

        let block_info = BlockInfo::default();

        // Initialize the contract with some starting height/time
        block_info
            .initialize(initial_block, initial_time, &mut env)
            .expect("initialize failed");

        (block_info, env)
    }

    #[test]
    fn test_initialize() {
        let initial_block = 100;
        let initial_time = 1650000000000; // 1,650,000,000,000 (approx example)
        let (block_info, env) = setup_block_info(AccountId::new(42), initial_block, initial_time);

        // Check `get_height` after initialization
        let height = block_info.get_height(&env).expect("get_height failed");
        assert_eq!(height, initial_block);

        // Check `get_time_unix_ms` after initialization
        let time = block_info
            .get_time_unix_ms(&env)
            .expect("get_time_unix_ms failed");
        assert_eq!(time, initial_time);

        // Check the combined query
        let details = block_info
            .get_block_details(&env)
            .expect("get_block_details failed");
        assert_eq!(details.height, initial_block);
        assert_eq!(details.time_unix_ms, initial_time);
    }

    #[test]
    fn test_set_block_info_authorized() {
        // RUNTIME_ACCOUNT_ID is the only authorized account to call `set_block_info`
        // Usually RUNTIME_ACCOUNT_ID is 0 in many frameworks, but it could be something else
        let runtime_sender_id = RUNTIME_ACCOUNT_ID;
        let initial_block = 200;
        let initial_time = 1651111111111;

        let (block_info, mut env) =
            setup_block_info(runtime_sender_id, initial_block, initial_time);

        // Attempt to set block info
        let new_block = 9999;
        let new_time = 9999999999999;

        let result = block_info.set_block_info(new_block, new_time, &mut env);
        assert!(
            result.is_ok(),
            "set_block_info should succeed for runtime sender"
        );

        // Check that the block info was updated
        let height = block_info.get_height(&env).unwrap();
        let time = block_info.get_time_unix_ms(&env).unwrap();

        assert_eq!(height, new_block, "height should have been updated");
        assert_eq!(time, new_time, "time_unix_ms should have been updated");
    }

    #[test]
    fn test_set_block_info_unauthorized() {
        // We will *not* set the sender to RUNTIME_ACCOUNT_ID
        let unauthorized_sender_id = AccountId::new(999);
        let (block_info, mut env) = setup_block_info(unauthorized_sender_id, 300, 1652222222222);

        let new_block = 5555;
        let new_time = 5555555555555;

        let result = block_info.set_block_info(new_block, new_time, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_UNAUTHORIZED),
            "Expected `ERR_UNAUTHORIZED` for non-runtime sender"
        );

        // Ensure values were not updated
        let height = block_info.get_height(&env).unwrap();
        let time = block_info.get_time_unix_ms(&env).unwrap();
        assert_eq!(height, 300, "height should remain unchanged");
        assert_eq!(time, 1652222222222, "time_unix_ms should remain unchanged");
    }

    #[test]
    fn test_get_time_unix_ms() {
        let (block_info, env) = setup_block_info(AccountId::new(42), 1234, 4321);
        let time = block_info
            .get_time_unix_ms(&env)
            .expect("get_time_unix_ms should succeed");
        assert_eq!(time, 4321);
    }

    #[test]
    fn test_get_block_details() {
        let initial_block = 777;
        let initial_time = 7777777777777;
        let (block_info, env) = setup_block_info(AccountId::new(42), initial_block, initial_time);

        let details = block_info
            .get_block_details(&env)
            .expect("get_block_details failed");
        assert_eq!(details.height, initial_block);
        assert_eq!(details.time_unix_ms, initial_time);
    }
}
