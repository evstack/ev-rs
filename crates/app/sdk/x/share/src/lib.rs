use evolve_core::account_impl;

#[account_impl(Share)]
pub mod share {
    use evolve_collections::{item::Item, map::Map};
    use evolve_core::{
        AccountId, ERR_UNAUTHORIZED, Environment, EnvironmentQuery, FungibleAsset, SdkResult,
        define_error, ensure, one_coin,
    };
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};
    use evolve_token::account::TokenRef;

    define_error!(ERR_INVALID_COIN, 0x1, "invalid coin");
    define_error!(ERR_NOT_ENOUGH_BALANCE, 0x2, "not enough balance");
    define_error!(ERR_OVERFLOW, 0x3, "arithmetic overflow");

    #[derive(evolve_core::AccountState)]
    pub struct Share {
        #[storage(0)]
        metadata: Item<FungibleAssetMetadata>,
        #[storage(1)]
        balances: Map<AccountId, u128>,
        #[storage(2)]
        total_supply: Item<u128>,
        #[storage(3)]
        pub(crate) manager: Item<AccountId>,
        #[storage(4)]
        pub(crate) coin_deposited: Item<FungibleAsset>,
    }

    impl Share {
        // -------------------------------------------------------------------------
        //                  1) Share Logic (Custom Implementation)
        // -------------------------------------------------------------------------

        /// Initialize the contract with metadata, initial balances, manager, and backing asset ID.
        /// `coin_deposited.amount` starts at 0. This sets up an empty “pool” of underlying at first.
        #[init]
        pub fn initialize(
            &self,
            metadata: FungibleAssetMetadata,
            initial_balances: Vec<(AccountId, u128)>,
            manager: AccountId,
            backing_asset: AccountId,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // Set up metadata
            self.metadata.set(&metadata, env)?;

            // Initialize balances
            let mut total = 0;
            for (account, bal) in initial_balances {
                self.balances.set(&account, &bal, env)?;
                total += bal;
            }
            self.total_supply.set(&total, env)?;

            // Manager
            self.manager.set(&manager, env)?;

            // Underlying deposit starts at 0
            self.coin_deposited.set(
                &FungibleAsset {
                    asset_id: backing_asset,
                    amount: 0,
                },
                env,
            )?;

            Ok(())
        }

        /// Deposit some underlying coin to receive newly minted shares (ratio-based).
        /// Only the `manager` can call this method.
        #[exec]
        pub fn mint(&self, recipient: AccountId, env: &mut dyn Environment) -> SdkResult<()> {
            // Only manager can call
            ensure!(env.sender() == self.manager.get(env)?, ERR_UNAUTHORIZED);

            // Must include exactly one coin deposit in this call
            let deposit = one_coin(env)?;
            // Must be the backing asset
            let mut backing = self.coin_deposited.get(env)?;
            ensure!(deposit.asset_id == backing.asset_id, ERR_INVALID_COIN);

            let total_supply_before = self.total_supply.get(env)?;
            let coin_deposited_before = backing.amount;

            // If no underlying deposit yet, we do a 1:1 for the very first deposit
            let shares_to_mint = if coin_deposited_before == 0 {
                // The very first deposit: 1 share per 1 underlying
                deposit.amount
            } else {
                // minted_shares = deposit.amount * total_supply / coin_deposited
                let numerator = deposit
                    .amount
                    .checked_mul(total_supply_before)
                    .ok_or(ERR_OVERFLOW)?;
                numerator / coin_deposited_before
            };

            // Update underlying deposit in storage
            backing.amount = backing
                .amount
                .checked_add(deposit.amount)
                .ok_or(ERR_OVERFLOW)?;
            self.coin_deposited.set(&backing, env)?;

            // Actually increase shares in circulation
            self._mint_unchecked(recipient, shares_to_mint, env)?;

            Ok(())
        }

        /// Burns share tokens to redeem underlying. The user must `one_coin` the shares in,
        /// and we send them underlying proportionally.
        /// The ratio is coin_deposited : total_supply.
        #[exec(payable)]
        pub fn burn_for_underlying(&self, env: &mut dyn Environment) -> SdkResult<()> {
            let shares_in = one_coin(env)?;
            ensure!(shares_in.asset_id == env.whoami(), ERR_INVALID_COIN);

            let total_supply_before = self.total_supply.get(env)?;
            ensure!(
                shares_in.amount <= total_supply_before,
                ERR_NOT_ENOUGH_BALANCE
            );

            let mut backing = self.coin_deposited.get(env)?;
            let coin_deposited_before = backing.amount;

            // underlying_out = shares_in * coin_deposited_before / total_supply_before
            let numerator = shares_in
                .amount
                .checked_mul(coin_deposited_before)
                .ok_or(ERR_OVERFLOW)?;
            let underlying_out = numerator / total_supply_before;

            // Burn the shares from supply
            self._burn_unchecked(env.sender(), shares_in.amount, env)?;

            // Reduce the contract’s underlying deposit
            backing.amount = backing.amount.saturating_sub(underlying_out);
            // transfer out
            TokenRef::new(backing.asset_id).transfer(env.sender(), underlying_out, env)?;
            // update coin deposited
            self.coin_deposited.set(&backing, env)?;

            Ok(())
        }

        /// Slash forcibly reduces the underlying deposit (e.g., penalty),
        /// only callable by the manager. This worsens the share-to-underlying ratio
        /// but does not change the total share supply.
        #[exec]
        pub fn slash(&self, amount: u128, env: &mut dyn Environment) -> SdkResult<()> {
            ensure!(env.sender() == self.manager.get(env)?, ERR_UNAUTHORIZED);

            self.coin_deposited.update(
                |old| {
                    let mut old = old.unwrap();
                    old.amount = old.amount.saturating_sub(amount);
                    Ok(old)
                },
                env,
            )?;

            Ok(())
        }

        // -------------------------------------------------------------------------
        //            2) "Token-like" Implementation (internal helpers)
        // -------------------------------------------------------------------------

        /// Internal function to increment a recipient's balance and total supply.
        fn _mint_unchecked(
            &self,
            recipient: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // Increase `recipient` balance
            self.balances.update(
                &recipient,
                |balance| Ok(balance.unwrap_or_default() + amount),
                env,
            )?;
            // Increase total supply
            self.total_supply
                .update(|old| Ok(old.unwrap_or_default() + amount), env)?;

            Ok(())
        }

        /// Internal function to reduce a holder's balance and total supply.
        fn _burn_unchecked(
            &self,
            from_account: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // Decrease `from_account` balance
            self.balances.update(
                &from_account,
                |balance| {
                    balance
                        .unwrap_or_default()
                        .checked_sub(amount)
                        .ok_or(ERR_NOT_ENOUGH_BALANCE)
                },
                env,
            )?;
            // Decrease total supply
            self.total_supply.update(
                |old| {
                    old.unwrap_or_default()
                        .checked_sub(amount)
                        .ok_or(ERR_NOT_ENOUGH_BALANCE)
                },
                env,
            )?;
            Ok(())
        }
    }

    // -----------------------------------------------------------------------------
    //                 Implementation of the FungibleAssetInterface
    // -----------------------------------------------------------------------------
    impl FungibleAssetInterface for Share {
        #[exec]
        fn transfer(
            &self,
            to: AccountId,
            amount: u128,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            // Reduce sender’s balance
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
            // Increase recipient’s balance
            self.balances
                .update(&to, |balance| Ok(balance.unwrap_or_default() + amount), env)?;
            Ok(())
        }

        #[query]
        fn metadata(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<FungibleAssetMetadata> {
            Ok(self.metadata.may_get(env)?.unwrap())
        }

        #[query]
        fn get_balance(
            &self,
            account: AccountId,
            env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<Option<u128>> {
            self.balances.may_get(&account, env)
        }

        #[query]
        fn total_supply(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u128> {
            self.total_supply.get(env)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::share::{ERR_INVALID_COIN, ERR_NOT_ENOUGH_BALANCE, Share, TransferMsg};
    use evolve_core::{AccountId, ERR_UNAUTHORIZED, EnvironmentQuery, FungibleAsset};
    use evolve_fungible_asset::{FungibleAssetInterface, FungibleAssetMetadata};
    use evolve_testing::MockEnv;

    fn setup_share(
        manager_id: u128,
        initial_holder_id: u128,
        initial_holder_balance: u128,
        backing_asset_id: u128,
    ) -> (Share, MockEnv) {
        // We'll make the contract's address distinct from the manager and holder
        let contract_address = AccountId::new(1);
        let sender_address = AccountId::new(manager_id); // manager is the sender

        // Initialize environment such that `env.sender()` is the manager
        let mut env = MockEnv::new(contract_address, sender_address);

        // Create the share instance
        let share = Share::new();

        // Setup some metadata
        let metadata = FungibleAssetMetadata {
            name: "ShareToken".to_string(),
            symbol: "ST".to_string(),
            decimals: 6,
            icon_url: "".to_string(),
            description: "A share token".to_string(),
        };

        // Provide initial balances
        let initial_balances = vec![(AccountId::new(initial_holder_id), initial_holder_balance)];

        // Initialize the contract
        // coin_deposited.amount starts at 0, with backing_asset=backing_asset_id
        share
            .initialize(
                metadata,
                initial_balances,
                AccountId::new(manager_id),
                AccountId::new(backing_asset_id),
                &mut env,
            )
            .expect("Failed to initialize Share contract");

        (share, env)
    }

    // -------------------------------------------------------------------------
    //  Basic Tests (Initialize, Transfer, etc.)
    // -------------------------------------------------------------------------

    #[test]
    fn test_initialize_and_metadata() {
        // manager=42, initial_holder=10, holder_balance=1_000, backing_asset=999
        let (share, mut env) = setup_share(42, 10, 1000, 999);

        // Check the metadata
        let md = share.metadata(&mut env).expect("metadata query failed");
        assert_eq!(md.name, "ShareToken");
        assert_eq!(md.symbol, "ST");
        assert_eq!(md.decimals, 6);

        // Check the initial holder's balance
        let bal = share
            .get_balance(AccountId::new(10), &mut env)
            .expect("get_balance failed")
            .unwrap();
        assert_eq!(bal, 1000);

        // Check total supply
        let total_supply = share
            .total_supply(&mut env)
            .expect("total_supply query failed");
        assert_eq!(total_supply, 1000);

        // Manager check
        let manager = share.manager.get(&mut env).unwrap();
        assert_eq!(manager, AccountId::new(42));
    }

    #[test]
    fn test_transfer_sufficient_balance() {
        // manager=100, initial_holder=50 (1,000 shares), backing_asset=999
        let (share, mut env) = setup_share(100, 50, 1000, 999);

        // Let the holder(50) be the current sender
        env = env.with_sender(AccountId::new(50));

        // Transfer 300 shares from 50 -> 60
        let result = share.transfer(AccountId::new(60), 300, &mut env);
        assert!(result.is_ok(), "Transfer should succeed");

        let from_balance = share
            .get_balance(AccountId::new(50), &mut env)
            .expect("balance check failed")
            .unwrap();
        assert_eq!(from_balance, 700);

        let to_balance = share
            .get_balance(AccountId::new(60), &mut env)
            .expect("balance check failed")
            .unwrap();
        assert_eq!(to_balance, 300);
    }

    #[test]
    fn test_transfer_insufficient_balance() {
        // manager=1, initial_holder=50 (only 100 shares), backing_asset=999
        let (share, mut env) = setup_share(1, 50, 100, 999);

        // Current sender is 50
        env = env.with_sender(AccountId::new(50));

        // Attempt to transfer 999, but user only has 100
        let result = share.transfer(AccountId::new(60), 999, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_NOT_ENOUGH_BALANCE),
            "Should fail with ERR_NOT_ENOUGH_BALANCE"
        );
    }

    #[test]
    fn test_get_balance_not_set_yet() {
        let (share, mut env) = setup_share(2, 10, 100, 999);

        // Check an address that wasn't granted any initial shares
        let bal = share
            .get_balance(AccountId::new(9999), &mut env)
            .expect("call failed");
        assert_eq!(bal, None);
    }

    // -------------------------------------------------------------------------
    //  Tests for the share-specific "mint" (deposit underlying) logic
    // -------------------------------------------------------------------------
    #[test]
    fn test_mint_authorized() {
        // manager=200, initial_holder=10 (balance=500), backing_asset=999
        let (share, mut env) = setup_share(200, 10, 500, 999);

        // Let's deposit 100 of the underlying coin to mint new shares
        // The environment's sender is manager(200), which is authorized
        // Provide the deposit via `with_funds(...)`
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 100,
        }]);

        let recipient = AccountId::new(10);
        let before_balance = share.get_balance(recipient, &mut env).unwrap().unwrap();
        assert_eq!(before_balance, 500);

        let result = share.mint(recipient, &mut env);
        assert!(result.is_ok(), "Mint should succeed");

        // Now check the new share balance after ratio-based mint
        let after_balance = share.get_balance(recipient, &mut env).unwrap().unwrap();

        // For the first deposit (coin_deposited=0), minted == deposit => 100
        let expected_after = 600; // 500 + 100
        assert_eq!(after_balance, expected_after);

        // Verify coin_deposited changed from 0 -> 100
        let coin_deposited = share.coin_deposited.get(&mut env).unwrap();
        assert_eq!(coin_deposited.amount, 100);
        assert_eq!(coin_deposited.asset_id, AccountId::new(999));
    }

    #[test]
    fn test_mint_unauthorized() {
        // manager=200, initial_holder=10, backing_asset=999
        let (share, mut env) = setup_share(200, 10, 500, 999);

        // Switch env sender to 999 => not manager
        env = env.with_sender(AccountId::new(999));

        // Provide the deposit
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 50,
        }]);

        // Attempt to mint
        let result = share.mint(AccountId::new(10), &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_UNAUTHORIZED),
            "Should fail with ERR_UNAUTHORIZED"
        );
    }

    #[test]
    fn test_mint_second_deposit_ratio() {
        // manager=200, initial_holder=10 (balance=500), backing_asset=999
        let (share, mut env) = setup_share(200, 10, 500, 999);

        // First deposit: deposit 100 underlying
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 100,
        }]);
        share
            .mint(AccountId::new(10), &mut env)
            .expect("1st deposit failed");

        // Now total_supply = 600, coin_deposited = 100

        // Next deposit is 50 => new_shares = 50 * 600 / 100 = 300
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 50,
        }]);
        share
            .mint(AccountId::new(10), &mut env)
            .expect("2nd deposit failed");

        // Final balance of 10 should be 600 + 300 = 900
        let final_bal = share
            .get_balance(AccountId::new(10), &mut env)
            .unwrap()
            .unwrap();
        assert_eq!(final_bal, 900);

        let final_coin_deposited = share.coin_deposited.get(&mut env).unwrap().amount;
        // Should now be 150 total underlying
        assert_eq!(final_coin_deposited, 150);
    }

    // -------------------------------------------------------------------------
    //  Tests for burn_for_underlying (redeem) logic
    // -------------------------------------------------------------------------
    #[test]
    fn test_burn_for_underlying_basic() {
        let (share, mut env) = setup_share(42, 10, 1000, 999);

        // Manager deposits 100 underlying
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 100,
        }]);
        // Mint directly to the same holder: 10
        share
            .mint(AccountId::new(10), &mut env)
            .expect("First deposit failed");

        // Now total_supply = 1100, coin_deposited=100
        // The holder(10) tries to burn 110 shares => gets 10 underlying
        let share_id = env.whoami();
        env = env
            .with_sender(AccountId::new(10))
            .with_funds(vec![FungibleAsset {
                asset_id: share_id,
                amount: 110,
            }]);

        let before_balance = share
            .get_balance(AccountId::new(10), &mut env)
            .unwrap()
            .unwrap();
        assert_eq!(before_balance, 1100);

        let mut env = env.with_exec_handler(|account_id, req: TransferMsg, funds| {
            assert!(funds.is_empty());
            assert_eq!(account_id, AccountId::new(999));
            assert_eq!(req.amount, 10);
            assert_eq!(req.to, AccountId::new(10));
            Ok(())
        });

        share
            .burn_for_underlying(&mut env)
            .expect("burn_for_underlying failed");

        // new balance = 1100 - 110 = 990
        let after_balance = share
            .get_balance(AccountId::new(10), &mut env)
            .unwrap()
            .unwrap();
        assert_eq!(after_balance, 990);

        // coin_deposited decreased by 10 => 100 - 10 = 90
        let new_coin_deposited = share.coin_deposited.get(&mut env).unwrap().amount;
        assert_eq!(new_coin_deposited, 90);
    }

    #[test]
    fn test_burn_for_underlying_wrong_asset() {
        let (share, mut env) = setup_share(42, 10, 1000, 1234);

        // Attempt to burn but send the *underlying* asset in instead of the share
        env = env
            .with_sender(AccountId::new(10))
            .with_funds(vec![FungibleAsset {
                asset_id: AccountId::new(1234), // backing asset, not the share
                amount: 100,
            }]);

        let result = share.burn_for_underlying(&mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_INVALID_COIN),
            "Should fail with ERR_INVALID_COIN"
        );
    }

    #[test]
    fn test_burn_for_underlying_insufficient_balance() {
        let (share, mut env) = setup_share(42, 10, 100, 999);
        // The user only has 100 shares initially

        // Let the user attempt to burn 999 shares
        let share_account_id = env.whoami();
        env = env
            .with_sender(share_account_id)
            .with_funds(vec![FungibleAsset {
                asset_id: share_account_id,
                amount: 999,
            }]);

        let result = share.burn_for_underlying(&mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_NOT_ENOUGH_BALANCE),
            "Should fail with ERR_NOT_ENOUGH_BALANCE"
        );
    }

    // -------------------------------------------------------------------------
    //  Tests for slash
    // -------------------------------------------------------------------------
    #[test]
    fn test_slash_authorized() {
        let (share, mut env) = setup_share(42, 10, 1000, 999);

        // Manager deposits 500 underlying
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 500,
        }]);
        share
            .mint(AccountId::new(10), &mut env)
            .expect("mint failed");

        // underlying deposit is now 500
        let cd_before = share.coin_deposited.get(&mut env).unwrap().amount;
        assert_eq!(cd_before, 500);

        // slash 200 -> authorized: sender is manager=42
        share.slash(200, &mut env).expect("slash failed");

        let cd_after = share.coin_deposited.get(&mut env).unwrap().amount;
        assert_eq!(cd_after, 300);
    }

    #[test]
    fn test_slash_unauthorized() {
        let (share, mut env) = setup_share(42, 10, 1000, 999);

        // Switch to a non-manager sender
        env = env.with_sender(AccountId::new(123));

        let result = share.slash(999, &mut env);
        assert!(
            matches!(result, Err(e) if e == ERR_UNAUTHORIZED),
            "Should fail with ERR_UNAUTHORIZED"
        );
    }

    #[test]
    fn test_slash_to_zero() {
        let (share, mut env) = setup_share(42, 10, 1000, 999);

        // deposit 100 underlying
        env = env.with_funds(vec![FungibleAsset {
            asset_id: AccountId::new(999),
            amount: 100,
        }]);
        share
            .mint(AccountId::new(10), &mut env)
            .expect("mint failed");

        // slash entire deposit
        share.slash(100, &mut env).expect("slash failed");
        let cd_after = share.coin_deposited.get(&mut env).unwrap().amount;
        assert_eq!(cd_after, 0);

        // Additional slash calls saturate at 0, not negative
        share.slash(9999, &mut env).expect("slash failed again");
        let cd_after2 = share.coin_deposited.get(&mut env).unwrap().amount;
        assert_eq!(cd_after2, 0, "Slash saturates at zero");
    }
}
