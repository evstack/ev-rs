use evolve_macros::account_impl;

#[account_impl(Token)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};

    pub const ERR_NOT_ENOUGH_BALANCE: ErrorCode = ErrorCode::new(0, "not enough balance balance");

    pub struct Token {
        pub metadata: Item<FungibleAssetMetadata>,
        pub balances: Map<AccountId, u128>,
    }

    impl Token {
        pub const fn new() -> Self {
            Self {
                metadata: Item::new(0),
                balances: Map::new(1),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            metadata: FungibleAssetMetadata,
            balances: Vec<(AccountId, u128)>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.metadata.set(&metadata, env)?;
            for (account, balance) in balances {
                self.balances.set(&account, &balance, env)?
            }
            Ok(())
        }
    }

    impl FungibleAssetInterface for Token {
        #[exec]
        fn transfer(
            &self,
            to: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.balances.update(
                &env.sender(),
                |balance| {
                    Ok(balance
                        .unwrap_or_default()
                        .checked_sub(amount)
                        .ok_or(ERR_NOT_ENOUGH_BALANCE)?)
                },
                env,
            )?;

            self.balances
                .update(&to, |balance| Ok(balance.unwrap_or_default() + amount), env)?;

            Ok(())
        }

        #[query]
        fn metadata(&self, env: &dyn Environment) -> SdkResult<FungibleAssetMetadata> {
            Ok(self.metadata.get(env)?.unwrap())
        }

        #[query]
        fn get_balance(
            &self,
            account: AccountId,
            env: &dyn Environment,
        ) -> SdkResult<Option<u128>> {
            self.balances.get(&account, env)
        }
    }
}
