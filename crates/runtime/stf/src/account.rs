use evolve_macros::account_impl;

#[account_impl(Asset)]
pub mod asset_account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::{Item, Map};
    use evolve_core::encoding::Encodable;
    use evolve_core::{
        AccountCode, AccountId, Environment, InvokeRequest, InvokeResponse, Message, SdkResult,
        ERR_UNKNOWN_FUNCTION,
    };
    use evolve_macros::{exec, init, query};

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
            env: &mut dyn Environment,
            name: String,
            init_balances: Vec<(AccountId, u128)>,
        ) -> SdkResult<()> {
            for (addr, balance) in init_balances {
                self.balances.set(env, addr, balance)?;
            }
            self.name.set(env, name)?;

            Ok(())
        }

        #[exec]
        pub fn transfer(
            &self,
            env: &mut dyn Environment,
            to: AccountId,
            amount: u128,
        ) -> SdkResult<()> {
            todo!()
        }

        #[query]
        pub fn get_balance(
            &self,
            env: &dyn Environment,
            account_id: AccountId,
        ) -> SdkResult<Option<u128>> {
            self.balances.get(env, account_id)
        }

        fn ignored(&self, env: &mut dyn Environment, account_id: AccountId) -> SdkResult<()> {
            // this fn is ignored bcz no query or exec
            todo!()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::asset_account::Asset;
    use crate::account::asset_account;
    use crate::test::TestStf;
    use evolve_core::encoding::Encodable;
    use evolve_core::{AccountId, Message};
    use evolve_server_core::mocks::MockedAccountsCodeStorage;
    use evolve_server_core::AccountsCodeStorage;
    use std::collections::HashMap;

    #[test]
    fn test() {
        let mut account_codes = MockedAccountsCodeStorage::new();

        let asset = Asset::new();

        account_codes.add_code(asset).unwrap();

        let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        let atom_id = TestStf::create_account(
            &mut storage,
            &mut account_codes,
            AccountId::new(100u128),
            "asset".to_owned(),
            Message::from(
                asset_account::InitializeMsg {
                    name: "atom".to_string(),
                    init_balances: vec![(AccountId::new(1u128), 1000u128)],
                }
                .encode()
                .unwrap(),
            ),
        )
        .unwrap()
        .new_account_id;

        // try get balance
        let req = asset_account::GetBalanceMsg{
            account_id: AccountId::new(1u128),
        };
    }
}
