use evolve_macros::account_impl;

#[account_impl(Token)]
pub mod account {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{define_error, AccountId, Environment, SdkResult, ERR_UNAUTHORIZED};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};

    define_error!(ERR_NOT_ENOUGH_BALANCE, 0x1, "not enough balance");

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

        pub fn mint_unchecked(
            &self,
            recipient: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
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
        pub fn mint(
            &self,
            recipient: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if self.supply_manager.get(env)? != Some(env.sender()) {
                return Err(ERR_UNAUTHORIZED);
            }
            self.mint_unchecked(recipient, amount, env)
        }

        pub fn burn_unchecked(
            &self,
            from_account: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            self.balances.update(
                &from_account,
                |balance| Ok(balance.unwrap_or_default() - amount),
                env,
            )?;
            self.total_supply
                .update(|supply| Ok(supply.unwrap_or_default() - amount), env)?;

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
            self.burn_unchecked(from_account, amount, env)
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

        #[query]
        fn total_supply(&self, env: &dyn Environment) -> SdkResult<u128> {
            self.total_supply.get(env)
        }
    }
}

#[cfg(test)]
mod tests {
    use evolve_core::{AccountId, ERR_UNAUTHORIZED};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};

    use crate::account::Token;
    // The generated account struct
    use crate::account::ERR_NOT_ENOUGH_BALANCE;

    use evolve_testing::MockEnv;

    /// Helper to initialize a `Token` with some default data.
    /// Returns `(token_instance, env)`.
    fn setup_token(
        supply_manager_id: u128,
        initial_holder_id: u128,
        initial_balance: u128,
    ) -> (Token, MockEnv) {
        // We'll make the contract's address distinct from the manager and holder
        let contract_address = AccountId::new(1);
        let sender_address = AccountId::new(supply_manager_id);

        // Initialize environment such that `env.sender()` is the supply manager
        let mut env = MockEnv::new(contract_address, sender_address);

        // Create the token instance
        let token = Token::default();

        // Setup some metadata
        let metadata = FungibleAssetMetadata {
            name: "TestToken".to_string(),
            symbol: "TT".to_string(),
            decimals: 2,
            icon_url: "".to_string(),
            description: "".to_string(),
        };

        // Provide initial balances in a vector
        let balances = vec![(AccountId::new(initial_holder_id), initial_balance)];

        // Initialize the token
        //   supply_manager = Some(AccountId::new(supply_manager_id))
        token
            .initialize(
                metadata,
                balances,
                Some(AccountId::new(supply_manager_id)),
                &mut env,
            )
            .expect("Failed to initialize token");

        (token, env)
    }

    #[test]
    fn test_initialize_and_metadata() {
        // Supply manager = 42, initial holder = 10, balance = 1_000
        let (token, env) = setup_token(42, 10, 1000);

        // Check the metadata query
        let metadata = token.metadata(&env).expect("metadata query failed");
        assert_eq!(metadata.name, "TestToken");
        assert_eq!(metadata.symbol, "TT");
        assert_eq!(metadata.decimals, 2);

        // Check the initial holder's balance
        let balance = token
            .get_balance(AccountId::new(10), &env)
            .expect("get_balance failed")
            .unwrap();
        assert_eq!(balance, 1000);

        // Check the supply manager
        let supply_manager = token.supply_manager.get(&env).unwrap();
        assert_eq!(supply_manager, Some(AccountId::new(42)));
    }

    #[test]
    fn test_mint_authorized() {
        // supply_manager = 100, initial_holder = 10, balance=500
        let (token, mut env) = setup_token(100, 10, 500);

        // We'll mint to holder 10 some extra tokens.
        let holder = AccountId::new(10);
        let before_balance = token.get_balance(holder, &env).unwrap().unwrap();
        assert_eq!(before_balance, 500);

        // Authorized call: the environment's sender == supply_manager(100).
        let result = token.mint(holder, 200, &mut env);
        assert!(
            result.is_ok(),
            "Mint should succeed under authorized manager"
        );

        let after_balance = token.get_balance(holder, &env).unwrap().unwrap();
        assert_eq!(after_balance, 700, "Should have minted 200 more tokens");
    }

    #[test]
    fn test_mint_unauthorized() {
        // supply_manager = 100, but environment sender is changed to 999
        let (token, mut env) = setup_token(100, 10, 500);

        // Change the environment's sender to something not the supply_manager
        let new_sender = AccountId::new(999);
        env = env.with_sender(new_sender);

        // Attempt to mint
        let result = token.mint(AccountId::new(10), 200, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_UNAUTHORIZED),
            "Mint should fail with ERR_UNAUTHORIZED"
        );
    }

    #[test]
    fn test_burn_authorized() {
        // supply_manager = 42, initial_holder = 10, balance=1000
        let (token, mut env) = setup_token(42, 10, 1000);

        // We'll burn from holder 10
        let holder = AccountId::new(10);

        let before_balance = token.get_balance(holder, &env).unwrap().unwrap();
        assert_eq!(before_balance, 1000);

        // environment's sender is supply_manager=42 -> authorized
        let result = token.burn(holder, 300, &mut env);
        assert!(
            result.is_ok(),
            "Burn should succeed under authorized manager"
        );

        let after_balance = token.get_balance(holder, &env).unwrap().unwrap();
        assert_eq!(
            after_balance, 700,
            "Should have burned 300 tokens from holder"
        );
    }

    #[test]
    fn test_burn_unauthorized() {
        let (token, mut env) = setup_token(42, 10, 500);

        // Change environment's sender to 999
        env = env.with_sender(AccountId::new(999));

        // Attempt to burn
        let result = token.burn(AccountId::new(10), 100, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_UNAUTHORIZED),
            "Burn should fail with ERR_UNAUTHORIZED"
        );
    }

    #[test]
    fn test_transfer_sufficient_balance() {
        // supply_manager=1, just ignore it for now. initial_holder=50, balance=1000
        let (token, mut env) = setup_token(1, 50, 1000);

        // For transfer, the environment's sender is the one transferring.
        // We'll assume the initial holder(50) is the same as env.sender,
        // so that holder can transfer their tokens.
        env = env.with_sender(AccountId::new(50));

        // Transfer 300 tokens from 50 -> 60
        let recipient = AccountId::new(60);
        let result = token.transfer(recipient, 300, &mut env);
        assert!(
            result.is_ok(),
            "Transfer should succeed with enough balance"
        );

        let from_balance = token
            .get_balance(AccountId::new(50), &env)
            .unwrap()
            .unwrap();
        assert_eq!(
            from_balance, 700,
            "Should have subtracted 300 tokens from sender"
        );

        let to_balance = token.get_balance(recipient, &env).unwrap().unwrap();
        assert_eq!(to_balance, 300, "Should have added 300 tokens to recipient");
    }

    #[test]
    fn test_transfer_insufficient_balance() {
        let (token, mut env) = setup_token(1, 50, 100);

        env = env.with_sender(AccountId::new(50));

        // Attempt to transfer 999, but user only has 100
        let result = token.transfer(AccountId::new(60), 999, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_NOT_ENOUGH_BALANCE),
            "Should fail with ERR_NOT_ENOUGH_BALANCE"
        );
    }

    #[test]
    fn test_get_balance_not_set_yet() {
        // supply_manager=2, initial_holder=10, initial_balance=100
        let (token, env) = setup_token(2, 10, 100);

        // Query an account that wasn't given an initial balance
        let balance = token
            .get_balance(AccountId::new(999), &env)
            .expect("call failed");
        assert_eq!(balance, None, "Account 999 should not have a balance yet");
    }
}
