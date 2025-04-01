use evolve_macros::account_impl;

pub mod server;
#[account_impl(BlockInfo)]
pub mod account {
    use evolve_collections::item::Item;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_macros::{exec, init};

    pub struct BlockInfo {
        pub block_time_unix_ms: Item<u64>,
        pub block: Item<u64>,
    }

    impl Default for BlockInfo {
        fn default() -> Self {
            Self::new()
        }
    }

    impl BlockInfo {
        pub const fn new() -> Self {
            Self {
                block_time_unix_ms: Item::new(0),
                block: Item::new(1),
            }
        }
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

            self.block.set(&current_block, env)?;
            self.block_time_unix_ms.set(&current_time_unix_ms, env)?;

            Ok(())
        }
        #[init]
        pub fn initialize(
            &self,
            current_block: u64,
            current_time_unix_ms: u64,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.block.set(&current_block, env)?;
            self.block_time_unix_ms.set(&current_time_unix_ms, env)?;

            Ok(())
        }
    }
}
