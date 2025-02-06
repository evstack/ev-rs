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
            todo!()
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
        pub fn get_balance(&self, env: &dyn Environment, account_id: AccountId) -> SdkResult<u128> {
            todo!()
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
    #[test]
    fn test() {
        let account = Asset::new();
    }
}
