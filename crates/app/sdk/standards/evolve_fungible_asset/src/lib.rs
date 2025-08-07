use evolve_core::account_impl;
pub use fa_interface::*;

#[account_impl(FungibleAssetInterface)]
mod fa_interface {
    use evolve_core::{AccountId, Environment, SdkResult};
    use evolve_macros::{exec, query};
    #[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone)]
    pub struct FungibleAssetMetadata {
        pub name: String,
        pub symbol: String,
        pub decimals: u8,
        pub icon_url: String,
        pub description: String,
    }
    pub trait FungibleAssetInterface {
        #[exec]
        fn transfer(&self, to: AccountId, amount: u128, env: &mut dyn Environment)
            -> SdkResult<()>;
        #[query]
        fn metadata(&self, env: &dyn Environment) -> SdkResult<FungibleAssetMetadata>;
        #[query]
        fn get_balance(&self, account: AccountId, env: &dyn Environment)
            -> SdkResult<Option<u128>>;
        #[query]
        fn total_supply(&self, env: &dyn Environment) -> SdkResult<u128>;
    }
}
