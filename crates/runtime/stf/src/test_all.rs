use evolve_macros::account_impl;

use evolve_fungible_asset::FungibleAssetInterface;

#[account_impl(Asset)]
pub mod asset_account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};

    const ERR_INSUFFICIENT_BALANCE: ErrorCode = 1;

    pub struct Asset {
        pub name: Item<String>,
        pub balances: Map<AccountId, u128>,
    }

    impl Asset {
        pub fn new() -> Self {
            Asset {
                name: Item::new(0),
                balances: Map::new(1),
            }
        }

        #[init]
        pub fn initialize(
            &self,
            name: String,
            init_balances: Vec<(AccountId, u128)>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            for (addr, balance) in init_balances {
                self.balances.set(&addr, &balance, env)?;
            }
            self.name.set(&name, env)?;

            Ok(())
        }

        fn ignored(&self, _account_id: AccountId, _env: &mut dyn Environment) -> SdkResult<()> {
            // this fn is ignored bcz no query or exec
            todo!()
        }
    }

    impl FungibleAssetInterface for Asset {
        #[exec]
        fn transfer(
            &self,
            to: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            let new_sender_balance = self.balances.update(
                &env.sender(),
                |value| {
                    value
                        .unwrap_or_default()
                        .checked_sub(amount)
                        .ok_or(ERR_INSUFFICIENT_BALANCE)
                },
                env,
            )?;

            let new_recipient_balance =
                self.balances
                    .update(&to, |value| Ok(value.unwrap_or_default() + amount), env)?;

            Ok(())
        }

        #[query]
        fn metadata(&self, env: &dyn Environment) -> SdkResult<FungibleAssetMetadata> {
            todo!("impl");
        }

        #[query]
        fn get_balance(
            &self,
            account_id: AccountId,
            env: &dyn Environment,
        ) -> SdkResult<Option<u128>> {
            self.balances.get(&account_id, env)
        }
    }
}

#[account_impl(MacroTester)]
pub mod macro_tester {
    use super::asset_account::AssetRef;
    use crate::test_all::migration_test;
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::low_level::migrate_account;
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_events::EventsEmitter;
    use evolve_macros::{exec, init, query};

    #[derive(BorshDeserialize, BorshSerialize)]
    pub struct TestEvent {
        pub content: String,
    }

    pub struct MacroTester {
        atom: Item<AssetRef>,
        events_emitter: EventsEmitter,
        migration_happened: Item<bool>,
    }

    impl MacroTester {
        pub(crate) fn new() -> Self {
            MacroTester {
                atom: Item::new(0),
                events_emitter: EventsEmitter::new(),
                migration_happened: Item::new(1),
            }
        }
        #[init]
        fn initialize(&self, env: &mut dyn Environment) -> SdkResult<()> {
            // tests initting a new atom account
            let (atom, _) =
                AssetRef::initialize("atom".to_string(), vec![(env.whoami(), 1000)], env)?;
            self.atom.set(&atom, env)?;
            let self_balance = atom
                .get_balance(env.whoami(), env)?
                .expect("expected balance");
            assert_eq!(self_balance, 1000);

            // send balance
            let someone_else = AccountId::new(1000u128);
            atom.transfer(someone_else, 500, env)?;

            // check someone else balance
            let someone_else_balance = atom
                .get_balance(someone_else, env)?
                .expect("expected balance");
            assert_eq!(someone_else_balance, 500);

            // emit event
            self.events_emitter.emit_event(
                "TEST_EVENT".to_string(),
                TestEvent {
                    content: "i am a test event".to_string(),
                },
                env,
            )?;

            Ok(())
        }

        #[exec]
        fn do_stuff(&self, env: &mut dyn Environment) -> SdkResult<()> {
            Ok(())
        }

        #[exec(payable)]
        fn receive_money(&self, env: &mut dyn Environment) -> SdkResult<()> {
            if env.funds().is_empty() {
                panic!("no money received");
            }
            Ok(())
        }

        #[exec]
        fn do_migration(&self, env: &mut dyn Environment) -> SdkResult<()> {
            migrate_account(
                env.whoami(),
                "MigrationTester".to_string(),
                &migration_test::Migrate1to2Msg { value: 10 },
                vec![],
                env,
            )
        }

        #[query]
        fn atom_id(&self, env: &dyn Environment) -> SdkResult<AccountId> {
            Ok(self.atom.get(env)?.expect("expected atom_id").0)
        }
    }
}
#[account_impl(MigrationTester)]
mod migration_test {
    use evolve_collections::item::Item;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::{exec, init, query};
    use evolve_scheduler::scheduler_account::ERR_UNAUTHORIZED;

    pub struct MigrationTester {
        pub migration_happened: Item<u64>,
    }
    impl MigrationTester {
        pub const fn new() -> Self {
            Self {
                migration_happened: Item::new(0),
            }
        }

        #[init]
        fn initialize(&self, _env: &mut dyn Environment) -> SdkResult<()> {
            Ok(())
        }
        #[exec]
        pub fn migrate1to2(&self, value: u64, env: &mut dyn Environment) -> SdkResult<()> {
            // only runtime can call migrate.
            if env.sender() != RUNTIME_ACCOUNT_ID {
                return Err(ERR_UNAUTHORIZED);
            }

            // make migration happen.
            self.migration_happened.set(&value, env)?;
            Ok(())
        }

        #[query]
        pub fn value(&self, env: &dyn Environment) -> SdkResult<u64> {
            Ok(self.migration_happened.get(env)?.expect("expected value"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::asset_account::Asset;
    use crate::mocks::TestStf;
    use crate::test_all::macro_tester::MacroTester;
    use crate::test_all::migration_test::MigrationTester;
    use evolve_core::AccountId;
    use evolve_server_core::mocks::MockedAccountsCodeStorage;
    use evolve_server_core::WritableKV;
    use std::collections::HashMap;

    #[test]
    fn test() {
        let mut account_codes = MockedAccountsCodeStorage::new();

        account_codes.add_code(Asset::new()).unwrap();
        account_codes.add_code(MacroTester::new()).unwrap();
        account_codes.add_code(MigrationTester::new()).unwrap();

        let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        let (resp, state_changes) = TestStf::create_account(
            &storage,
            &mut account_codes,
            AccountId::new(100u128),
            "MacroTester".to_owned(),
            super::macro_tester::InitializeMsg {},
            vec![],
        )
        .unwrap();

        storage.apply_changes(state_changes.into_changes()).unwrap();

        // query account
        let query_resp = TestStf::query(
            &storage,
            &mut account_codes,
            resp.new_account_id,
            &super::macro_tester::AtomIdMsg {},
        )
        .expect("expected no error")
        .get::<AccountId>()
        .expect("expected AccountId");

        // force migration to happen
        let (_, state_changes) = TestStf::exec(
            &storage,
            &mut account_codes,
            AccountId::new(100u128),
            resp.new_account_id,
            &super::macro_tester::DoMigrationMsg {},
            vec![],
        )
        .unwrap();
        storage.apply_changes(state_changes.into_changes()).unwrap();

        // now ensure the account was swapped correctly
        let value_resp = TestStf::query(
            &storage,
            &mut account_codes,
            resp.new_account_id,
            &super::migration_test::ValueMsg {},
        )
        .unwrap()
        .get::<u64>()
        .expect("expected u64");
        assert_eq!(10, value_resp);
    }

    #[test]
    fn test_fungible_asset_transfer() {}
}
