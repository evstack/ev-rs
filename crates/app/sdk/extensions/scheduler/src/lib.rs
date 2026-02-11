pub mod server;

use evolve_core::account_impl;

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
    use evolve_collections::item::Item;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{AccountId, Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_macros::{exec, init};

    #[derive(evolve_core::AccountState)]
    pub struct Scheduler {
        #[storage(0)]
        pub begin_block_accounts: Item<Vec<BeginBlockAccountInterfaceRef>>,
        #[storage(1)]
        pub end_block_accounts: Item<Vec<EndBlockAccountInterfaceRef>>,
    }

    impl Scheduler {
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
            for account in self.begin_block_accounts.may_get(env)?.unwrap() {
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
            for account in self.end_block_accounts.may_get(env)?.unwrap() {
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
