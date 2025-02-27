use evolve_macros::account_impl;

#[account_impl(Asset)]
pub mod asset {
    use evolve_collections::Map;
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};

    pub struct Asset {
        balances: Map<AccountId, u128>,
    }

    impl Asset {
        pub const fn new() -> Self {
            Self {
                balances: Map::new(0),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            balances: Vec<(AccountId, u128)>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            for (id, amount) in balances {
                self.balances.set(&id, &amount, env)?
            }

            Ok(())
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
            let sender = env.sender();
            let new_sender_balance = self.balances.update(
                &env.sender(),
                |value| {
                    let balance = value.unwrap_or_default();
                    balance.checked_sub(amount).ok_or(1)
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

#[account_impl(Staking)]
pub mod staking {
    use evolve_collections::{Item, Map};
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_macros::{exec, init, query};

    pub struct Staking {
        delegations: Map<AccountId, u128>,
        staking_asset: Item<AccountId>,
    }

    impl Staking {
        pub const ERR_NO_FUNDS: ErrorCode = 0;
        pub const ERR_INVALID_STAKING_ASSET: ErrorCode = 1;

        pub const fn new() -> Self {
            Self {
                delegations: Map::new(0),
                staking_asset: Item::new(1),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            staking_asset: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.staking_asset.set(&staking_asset, env)?;
            Ok(())
        }

        #[exec(payable)]
        pub fn stake(&self, env: &mut dyn Environment) -> SdkResult<()> {
            // check if user provided staking asset
            if env.funds().len() != 1 {
                return Err(Self::ERR_NO_FUNDS);
            }

            let stake_amount = env.funds()[0].amount;
            let stake_asset = env.funds()[0].asset_id;

            // invalid staking asset
            if stake_asset != self.staking_asset.get(env)?.unwrap() {
                return Err(Self::ERR_INVALID_STAKING_ASSET);
            }

            // increase delegation count
            let sender = env.sender();
            self.delegations.update(
                &env.sender(),
                |delegation| Ok(delegation.unwrap_or_default() + stake_amount),
                env,
            )?;
            Ok(())
        }

        #[query]
        pub fn query_balance(&self, env: &dyn Environment) -> SdkResult<u128> {
            todo!("impl")
        }
    }
}

#[account_impl(Lst)]
pub mod lst {
    use crate::example_test::asset::AssetRef;
    use crate::example_test::staking::StakingRef;
    use evolve_collections::Item;
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_macros::{exec, init};

    pub struct Lst {
        staking: Item<StakingRef>,
        lst_token: Item<AssetRef>,
        stake_asset: Item<AccountId>,
        total_staked_amount: Item<u128>,
    }

    impl Lst {
        pub const fn new() -> Self {
            Self {
                staking: Item::new(0),
                lst_token: Item::new(1),
                stake_asset: Item::new(2),
                total_staked_amount: Item::new(3),
            }
        }
    }

    impl Lst {
        #[init]
        pub fn initialize(
            &self,
            staking_address: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // set staking
            self.staking.set(&StakingRef(staking_address), env)?;
            // create lst token
            let lst_token = AssetRef::initialize(vec![], env)?.0;
            self.lst_token.set(&lst_token, env)?;
            Ok(())
        }
        #[exec(payable)]
        pub fn mint_lst(&self, env: &mut dyn Environment) -> SdkResult<()> {
            let token = env.funds().get(0).cloned().unwrap();

            // update total staked amount
            self.total_staked_amount.update(
                |v| Ok(v.unwrap_or_default().checked_add(token.amount).unwrap()),
                env,
            )?;

            // stake
            self.staking.get(env)?.unwrap().stake(vec![token], env)?;

            // TODO :mint

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::example_test::asset::Asset;
    use crate::example_test::lst::Lst;
    use crate::example_test::staking::Staking;
    use crate::mocks::TestStf;
    use evolve_core::{AccountId, FungibleAsset};
    use evolve_server_core::mocks::MockedAccountsCodeStorage;
    use evolve_server_core::{AccountsCodeStorage, WritableKV};
    use std::collections::HashMap;

    #[test]
    fn test() {
        let mut account_codes = MockedAccountsCodeStorage::new();

        account_codes.add_code(Asset::new()).unwrap();
        account_codes.add_code(Staking::new()).unwrap();
        account_codes.add_code(Lst::new()).unwrap();

        let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

        let alice = AccountId::new(1000u128);

        // create atom
        let (atom_token, state) = TestStf::create_account(
            &storage,
            &mut account_codes,
            alice,
            "Asset".to_string(),
            super::asset::InitializeMsg {
                balances: vec![(alice, 100000)],
            },
            vec![],
        )
        .unwrap();

        storage.apply_changes(state.into_changes()).unwrap();

        // create staking
        let (staking, state) = TestStf::create_account(
            &storage,
            &mut account_codes,
            alice,
            "Staking".to_string(),
            super::staking::InitializeMsg {
                staking_asset: atom_token.new_account_id,
            },
            vec![],
        )
        .unwrap();

        storage.apply_changes(state.into_changes()).unwrap();

        // create lst
        let (lst, state) = TestStf::create_account(
            &storage,
            &mut account_codes,
            alice,
            "Lst".to_string(),
            super::lst::InitializeMsg {
                staking_address: staking.new_account_id,
            },
            vec![],
        )
        .unwrap();
        storage.apply_changes(state.into_changes()).unwrap();

        // finally execute a message in which alice execs via LST
        let resp = TestStf::exec(
            &storage,
            &mut account_codes,
            alice,
            lst.new_account_id,
            &super::lst::MintLstMsg {},
            vec![FungibleAsset {
                asset_id: atom_token.new_account_id,
                amount: 100,
            }],
        )
        .unwrap();
    }
}
