// We implement a pool account which is also an LP token that users can transfer.

use evolve_macros::account_impl;

// Account impl macro defines who is the one implementing the AccountCode trait.
// the accountCode trait is implemented based on exec/init/query methods which are marked
// using macro attributes
#[account_impl(Account)]
pub mod pool {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_collections::item::Item;
    use evolve_core::{Environment, ErrorCode, FungibleAsset, SdkResult};
    use evolve_fungible_asset::{FungibleAssetInterfaceRef, FungibleAssetMetadata};
    use evolve_macros::{exec, init, query};
    use evolve_token::account::TokenRef;

    const ERR_INVALID_FUNDS_AMOUNT: ErrorCode = ErrorCode::new(0, "invalid funds amount");
    const ERR_INVALID_ASSET_ID: ErrorCode = ErrorCode::new(1, "invalid asset id");

    #[derive(BorshDeserialize, BorshSerialize, Clone)]
    pub struct PoolState {
        pub asset_one: FungibleAsset,
        pub asset_two: FungibleAsset,
    }

    // Define account and what it has inside of state, only collections types are allowed.
    pub struct Account {
        // a Ref it's an auto generated client to talk with an account
        // the methods of the Ref are based off the Query/EXec/Initialize methods on the account.
        lp_token: Item<TokenRef>,

        asset_one: Item<FungibleAssetInterfaceRef>,
        asset_two: Item<FungibleAssetInterfaceRef>,
    }

    impl Account {
        // this builds the account, note that the prefixes 0,1,2 must be different
        #[allow(dead_code)]
        pub const fn new() -> Self {
            Account {
                lp_token: Item::new(0),
                asset_one: Item::new(1),
                asset_two: Item::new(2),
            }
        }
        // marking method with init means this is the method used to initialize the account
        // adding the payable attribute means this can receive funds.
        // there can only be one init method.
        #[init(payable)]
        pub fn initialize(
            self: &Account,
            mut initial_funds: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if initial_funds.len() != 2 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }
            let asset_two = initial_funds.pop().unwrap();
            let asset_one = initial_funds.pop().unwrap();

            // init LP token
            let (lp_token, _) = TokenRef::initialize(
                FungibleAssetMetadata {
                    name: "lp".to_string(),
                    symbol: "LP".to_string(),
                    decimals: 0,
                    icon_url: "".to_string(),
                    description: "".to_string(),
                },
                vec![],
                Some(env.sender()),
                env,
            )?;

            // set token references.
            self.lp_token.set(&lp_token, env)?;
            self.asset_one
                .set(&FungibleAssetInterfaceRef::new(asset_one.asset_id), env)?;
            self.asset_two
                .set(&FungibleAssetInterfaceRef::new(asset_two.asset_id), env)?;

            Ok(())
        }

        // an exec method is expected to have a mut dyn environment
        // as last param, and the fact that it's payable means we expect
        // also vec<fungibleAsset> to be part of the function params.
        #[exec(payable)]
        pub fn deposit(
            self: &Account,
            mut inputs: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if inputs.len() != 2 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }

            let asset_two = inputs.pop().unwrap();
            let asset_one = inputs.pop().unwrap();

            // just assert correct asset types
            let asset_one_ref = self.asset_one.get(env)?;
            if asset_one_ref.0 != asset_one.asset_id {
                return Err(ERR_INVALID_ASSET_ID);
            }

            let asset_two_ref = self.asset_two.get(env)?;
            if asset_two_ref.0 != asset_two.asset_id {
                return Err(ERR_INVALID_ASSET_ID);
            }

            // compute how much to give to the user
            let lp_to_user: u128 = 100;

            let lp_token = self.lp_token.get(env)?;
            lp_token.mint(env.sender(), lp_to_user, env)?;

            Ok(())
        }

        #[exec(payable)]
        pub fn burn(
            self: &Account,
            mut lp_in: Vec<FungibleAsset>,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            if lp_in.len() != 1 {
                return Err(ERR_INVALID_FUNDS_AMOUNT);
            }

            let lp = lp_in.pop().unwrap();
            // since we are also a token we check that the token being transferred in is ours.
            if lp.asset_id != env.whoami() {
                return Err(ERR_INVALID_ASSET_ID);
            }

            // TODO: logic to compute how much to give out
            let asset_one_out = 100u128;
            let asset_two_out = 100u128;

            self.lp_token.get(env)?.burn(env.sender(), lp.amount, env)?;
            self.asset_one
                .get(env)?
                .transfer(env.sender(), asset_one_out, env)?;
            self.asset_two
                .get(env)?
                .transfer(env.sender(), asset_two_out, env)?;

            Ok(())
        }

        // this is how we implement a query by creating a method with &dyn Environment
        // and query attribute
        #[query]
        pub fn pool_state(self: &Account, env: &dyn Environment) -> SdkResult<PoolState> {
            let asset_one = self.asset_one.get(env)?;
            let asset_two = self.asset_two.get(env)?;

            Ok(PoolState {
                asset_one: FungibleAsset {
                    asset_id: asset_one.0,
                    amount: asset_one
                        .get_balance(env.whoami(), env)?
                        .unwrap_or_default(),
                },
                asset_two: FungibleAsset {
                    asset_id: asset_two.0,
                    amount: asset_two
                        .get_balance(env.whoami(), env)?
                        .unwrap_or_default(),
                },
            })
        }
    }
}
