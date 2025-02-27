pub mod server;

use evolve_core::Environment;
use evolve_macros::account_impl;

#[account_impl(BeginBlockAccountInterface)]
pub mod begin_block_account_interface {
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::exec;

    pub trait BeginBlockAccountInterface {
        #[exec]
        fn do_begin_block(&self, env: &mut dyn Environment) -> SdkResult<()>;
    }
}

#[account_impl(EndBlockAccountInterface)]
pub mod end_block_account_interface {
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::exec;

    pub trait EndBlockAccountInterface {
        #[exec]
        fn do_end_block(&self, env: &mut dyn Environment) -> SdkResult<()>;
    }
}

#[account_impl(Scheduler)]
pub mod scheduler_account {
    use crate::begin_block_account_interface::BeginBlockAccountInterfaceRef;
    use crate::end_block_account_interface::EndBlockAccountInterfaceRef;
    use evolve_collections::Item;
    use evolve_core::well_known::RUNTIME_ACCOUNT_ID;
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_macros::{exec, init};

    pub const ERR_UNAUTHORIZED: ErrorCode = 0;

    pub struct Scheduler {
        pub begin_block_accounts: Item<Vec<BeginBlockAccountInterfaceRef>>,
        pub end_block_accounts: Item<Vec<EndBlockAccountInterfaceRef>>,
    }

    impl Scheduler {
        pub const fn new() -> Scheduler {
            Scheduler {
                begin_block_accounts: Item::new(0),
                end_block_accounts: Item::new(1),
            }
        }

        #[init]
        pub fn initialize(
            &self,
            begin_block_accounts: Vec<AccountId>,
            end_block_accounts: Vec<AccountId>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.begin_block_accounts.set(
                &begin_block_accounts.into_iter().map(Into::into).collect(),
                env,
            )?;
            self.end_block_accounts.set(
                &end_block_accounts.into_iter().map(Into::into).collect(),
                env,
            )?;
            Ok(())
        }

        #[exec]
        pub fn update_begin_blockers(
            &self,
            new_begin_blockers: Vec<AccountId>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            self.begin_block_accounts.set(
                &new_begin_blockers.into_iter().map(Into::into).collect(),
                env,
            )?;

            Ok(())
        }
        #[exec]
        pub fn update_end_blockers(
            &self,
            new_end_blockers: Vec<AccountId>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            self.end_block_accounts
                .set(&new_end_blockers.into_iter().map(Into::into).collect(), env)?;

            Ok(())
        }

        #[exec]
        pub fn schedule_begin_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
            // only runtime can
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            // if one begin block fails should we continue?? TODO
            for account in self.begin_block_accounts.get(env)?.unwrap() {
                account.do_begin_block(env)?;
            }

            Ok(())
        }

        #[exec]
        pub fn schedule_end_block(&self, env: &mut dyn Environment) -> SdkResult<()> {
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            // if one end block fails should we continue?? TODO
            for account in self.end_block_accounts.get(env)?.unwrap() {
                account.do_end_block(env)?;
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_works() {}
}
