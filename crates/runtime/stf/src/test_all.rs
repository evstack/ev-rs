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
        pub fn initialize(
            &self,
            name: String,
            init_balances: Vec<(::evolve_core::AccountId, u128)>,
            env: &mut dyn ::evolve_core::Environment,
        ) -> SdkResult<()> {
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
        ) -> ::evolve_core::SdkResult<()> {
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
        ) -> ::evolve_core::SdkResult<Option<u128>> {
            ::evolve_core::low_level::query_account(
                self.0
                    .get(env)?
                    .ok_or(::evolve_core::ERR_ACCOUNT_NOT_INITIALIZED)?,
                GetBalanceMsg::FUNCTION_IDENTIFIER,
                &GetBalanceMsg {
                    account_id,
                },
                env
            )
        }

    }

}


#[account_impl(MacroTester)]
mod macro_tester {
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::{exec, init, query};
    use crate::test_all::asset_account::AssetAccount;

    struct MacroTester {
        account: AssetAccount,
    }

    impl MacroTester {
        fn new() -> Self {
            let account = AssetAccount::new(0);
            MacroTester { account }
        }
        #[init]
        fn initialize(
            &self,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            todo!("impl")
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
    use crate::test_all::asset_account;
    use evolve_core::encoding::Encodable;
    use evolve_core::{AccountCode, AccountId, InvokeRequest, Message};
    use evolve_server_core::mocks::MockedAccountsCodeStorage;
    use evolve_server_core::{AccountsCodeStorage, WritableKV};
    use std::collections::HashMap;

    #[test]
    fn test() {
        let mut account_codes = MockedAccountsCodeStorage::new();

        let asset = Asset::new();

        account_codes.add_code(asset).unwrap();

        let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        let (resp, state_changes) = TestStf::create_account(
            &storage,
            &mut account_codes,
            AccountId::new(100u128),
            "Asset".to_owned(),
            Message::from(
                asset_account::InitializeMsg {
                    name: "atom".to_string(),
                    init_balances: vec![(AccountId::new(1u128), 1000u128)],
                }
                .encode()
                .unwrap(),
            ),
        )
        .unwrap();

        storage.apply_changes(state_changes.into_changes()).unwrap();

        // try get balance
        let req = asset_account::GetBalanceMsg {
            account_id: AccountId::new(1u128),
        };

        let balance = TestStf::query(
            &storage,
            &mut account_codes,
            resp.new_account_id,
            InvokeRequest::new(
                asset_account::GetBalanceMsg::FUNCTION_IDENTIFIER,
                Message::from(req.encode().unwrap()),
            ),
        )
        .expect("query failed")
        .try_into_decodable::<Option<u128>>()
        .expect("failed to decode response");

        assert_eq!(balance, Some(1000u128));
    }
}
