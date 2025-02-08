use evolve_macros::account_impl;

#[account_impl(Asset)]
pub mod asset_account {
    use evolve_collections::{Item, Map};
    use evolve_core::encoding::Encodable;
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
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

        #[exec]
        pub fn transfer(
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
        pub fn get_balance(
            &self,
            account_id: AccountId,
            env: &dyn Environment,
        ) -> SdkResult<Option<u128>> {
            self.balances.get(&account_id, env)
        }

        fn ignored(&self, _account_id: AccountId, _env: &mut dyn Environment) -> SdkResult<()> {
            // this fn is ignored bcz no query or exec
            todo!()
        }
    }

    pub struct AssetAccount(::evolve_collections::Item<::evolve_core::AccountId>);

    impl AssetAccount {
        pub fn new(prefix: u8) -> Self {
            Self(::evolve_collections::Item::new(prefix))
        }

        pub fn set_account_id_ptr(
            &self,
            account_id: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.0.set(&account_id, env)
        }

        pub fn initialize(
            &self,
            name: String,
            init_balances: Vec<(::evolve_core::AccountId, u128)>,
            env: &mut dyn ::evolve_core::Environment,
        ) -> SdkResult<()> { // the result of init market
            let (self_account_id, resp) = ::evolve_core::low_level::create_account(
                "Asset".to_string(),
                &InitializeMsg {
                    name,
                    init_balances,
                },
                env,
            )?;

            self.0.set(&self_account_id, env)?;

            Ok(resp)
        }

        pub fn transfer(
            &self,
            to: ::evolve_core::AccountId,
            amount: u128,
            env: &mut dyn ::evolve_core::Environment,
        ) -> ::evolve_core::SdkResult<()> { // result of exec marker
            ::evolve_core::low_level::exec_account(
                self.0
                    .get(env)?
                    .ok_or(::evolve_core::ERR_ACCOUNT_NOT_INITIALIZED)?,
                TransferMsg::FUNCTION_IDENTIFIER,
                &TransferMsg { to, amount },
                env,
            )
        }

        pub fn get_balance(
            &self,
            account_id: ::evolve_core::AccountId,
            env: &dyn ::evolve_core::Environment,
        ) -> ::evolve_core::SdkResult<Option<u128>> { // result of query market
            ::evolve_core::low_level::query_account(
                self.0
                    .get(env)?
                    .ok_or(::evolve_core::ERR_ACCOUNT_NOT_INITIALIZED)?,
                GetBalanceMsg::FUNCTION_IDENTIFIER,
                &GetBalanceMsg { account_id },
                env,
            )
        }
    }
}

#[account_impl(MacroTester)]
pub mod macro_tester {
    use crate::test_all::asset_account::AssetAccount;
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_macros::{exec, init, query};

    pub struct MacroTester {
        atom: AssetAccount,
    }

    impl MacroTester {
        pub(crate) fn new() -> Self {
            let account = AssetAccount::new(0);
            MacroTester { atom: account }
        }
        #[init]
        fn initialize(&self, env: &mut dyn Environment) -> SdkResult<()> {
            // tests initting a new atom account
            self.atom
                .initialize("atom".to_string(), vec![(env.whoami(), 1000)], env)?;
            let self_balance = self
                .atom
                .get_balance(env.whoami(), env)?
                .expect("expected balance");
            assert_eq!(self_balance, 1000);

            // send balance
            let someone_else = AccountId::new(1000u128);
            self.atom.transfer(someone_else, 500, env)?;

            // check someone else balance
            let someone_else_balance = self
                .atom
                .get_balance(someone_else, env)?
                .expect("expected balance");
            assert_eq!(someone_else_balance, 500);

            Ok(())
        }

        #[exec]
        fn dostuff(&self, env: &mut dyn Environment) -> SdkResult<()> {
            todo!("impl")
        }

        #[query]
        fn query(&self, env: &dyn Environment) -> SdkResult<()> {
            todo!("impl")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::asset_account::Asset;
    use crate::test::TestStf;
    use crate::test_all::macro_tester::MacroTester;
    use evolve_core::encoding::Encodable;
    use evolve_core::{AccountId, Message};
    use evolve_server_core::mocks::MockedAccountsCodeStorage;
    use evolve_server_core::{AccountsCodeStorage, WritableKV};
    use std::collections::HashMap;

    #[test]
    fn test() {
        let mut account_codes = MockedAccountsCodeStorage::new();

        account_codes.add_code(Asset::new()).unwrap();
        account_codes.add_code(MacroTester::new()).unwrap();

        let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        let (resp, state_changes) = TestStf::create_account(
            &storage,
            &mut account_codes,
            AccountId::new(100u128),
            "MacroTester".to_owned(),
            Message::from(super::macro_tester::InitializeMsg {}.encode().unwrap()),
        )
        .unwrap();

        storage.apply_changes(state_changes.into_changes()).unwrap();
    }
}
