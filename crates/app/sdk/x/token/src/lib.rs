use evolve_macros::account_impl;

#[account_impl(Token)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult, ERR_UNAUTHORIZED};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};

    pub const ERR_NOT_ENOUGH_BALANCE: ErrorCode = ErrorCode::new(0, "not enough balance balance");

    pub struct Token {
        pub metadata: Item<FungibleAssetMetadata>,
        pub balances: Map<AccountId, u128>,
        pub total_supply: Item<u128>,
        pub supply_manager: Item<Option<AccountId>>,
    }

    impl Default for Token {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Token {
        pub const fn new() -> Self {
            Self {
                metadata: Item::new(0),
                balances: Map::new(1),
                total_supply: Item::new(2),
                supply_manager: Item::new(3),
            }
        }
        #[init]
        pub fn initialize(
            &self,
            metadata: FungibleAssetMetadata,
            balances: Vec<(AccountId, u128)>,
            supply_manager: Option<AccountId>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.metadata.set(&metadata, env)?;
            let mut total_supply = 0;
            for (account, balance) in balances {
                self.balances.set(&account, &balance, env)?;
                total_supply += balance;
            }
            self.supply_manager.set(&supply_manager, env)?;
            self.total_supply.set(&total_supply, env)?;

            Ok(())
        }

        #[exec]
        pub fn mint(
            &self,
            recipient: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if self.supply_manager.get(env)? != Some(env.sender()) {
                return Err(ERR_UNAUTHORIZED);
            }
            self.balances.update(
                &recipient,
                |balance| Ok(balance.unwrap_or_default() + amount),
                env,
            )?;
            self.total_supply
                .update(|supply| Ok(supply.unwrap_or_default() + amount), env)?;

            Ok(())
        }

        #[exec]
        pub fn burn(
            &self,
            from_account: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if self.supply_manager.get(env)? != Some(env.sender()) {
                return Err(ERR_UNAUTHORIZED);
            }
            self.balances.update(
                &from_account,
                |balance| Ok(balance.unwrap_or_default() - amount),
                env,
            )?;
            self.total_supply
                .update(|supply| Ok(supply.unwrap_or_default() - amount), env)?;

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
                    balance
                        .unwrap_or_default()
                        .checked_sub(amount)
                        .ok_or(ERR_NOT_ENOUGH_BALANCE)
                },
                env,
            )?;

            self.balances
                .update(&to, |balance| Ok(balance.unwrap_or_default() + amount), env)?;

            Ok(())
        }

        #[query]
        fn metadata(&self, env: &dyn Environment) -> SdkResult<FungibleAssetMetadata> {
            Ok(self.metadata.may_get(env)?.unwrap())
        }

        #[query]
        fn get_balance(
            &self,
            account: AccountId,
            env: &dyn Environment,
        ) -> SdkResult<Option<u128>> {
            self.balances.may_get(&account, env)
        }
    }
}
