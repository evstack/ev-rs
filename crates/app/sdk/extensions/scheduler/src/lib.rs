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
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{AccountId, ERR_UNAUTHORIZED};
    use evolve_testing::MockEnv;

    use crate::scheduler_account::Scheduler;

    fn setup_scheduler() -> (Scheduler, MockEnv) {
        let scheduler = Scheduler::default();
        let contract_address = AccountId::new(1);
        let mut env = MockEnv::new(contract_address, RUNTIME_ACCOUNT_ID);

        scheduler
            .initialize(Vec::new(), Vec::new(), &mut env)
            .expect("scheduler init must succeed");

        (scheduler, env)
    }

    #[test]
    fn update_begin_blockers_requires_runtime_sender() {
        let (scheduler, mut env) = setup_scheduler();
        env = env.with_sender(AccountId::new(999));

        let result = scheduler.update_begin_blockers(vec![AccountId::new(10)], &mut env);
        assert!(matches!(result, Err(e) if e == ERR_UNAUTHORIZED));

        let stored = scheduler
            .begin_block_accounts
            .may_get(&mut env)
            .unwrap()
            .unwrap();
        assert!(stored.is_empty());
    }

    #[test]
    fn update_end_blockers_requires_runtime_sender() {
        let (scheduler, mut env) = setup_scheduler();
        env = env.with_sender(AccountId::new(999));

        let result = scheduler.update_end_blockers(vec![AccountId::new(20)], &mut env);
        assert!(matches!(result, Err(e) if e == ERR_UNAUTHORIZED));

        let stored = scheduler
            .end_block_accounts
            .may_get(&mut env)
            .unwrap()
            .unwrap();
        assert!(stored.is_empty());
    }

    #[test]
    fn runtime_can_update_blockers() {
        let (scheduler, mut env) = setup_scheduler();

        scheduler
            .update_begin_blockers(vec![AccountId::new(10), AccountId::new(11)], &mut env)
            .unwrap();
        scheduler
            .update_end_blockers(vec![AccountId::new(20)], &mut env)
            .unwrap();

        let begin = scheduler
            .begin_block_accounts
            .may_get(&mut env)
            .unwrap()
            .unwrap();
        let end = scheduler
            .end_block_accounts
            .may_get(&mut env)
            .unwrap()
            .unwrap();
        assert_eq!(begin.len(), 2);
        assert_eq!(end.len(), 1);
    }

    #[test]
    fn schedule_methods_require_runtime_sender() {
        let (scheduler, mut env) = setup_scheduler();
        env = env.with_sender(AccountId::new(999));

        let begin_result = scheduler.schedule_begin_block(&mut env);
        assert!(matches!(begin_result, Err(e) if e == ERR_UNAUTHORIZED));

        let end_result = scheduler.schedule_end_block(&mut env);
        assert!(matches!(end_result, Err(e) if e == ERR_UNAUTHORIZED));
    }

    #[test]
    fn runtime_can_schedule_with_empty_blockers() {
        let (scheduler, mut env) = setup_scheduler();

        assert!(scheduler.schedule_begin_block(&mut env).is_ok());
        assert!(scheduler.schedule_end_block(&mut env).is_ok());
    }
}
