use evolve_macros::account_impl;
pub use fa_interface::*;

#[account_impl(FungibleAssetInterface)]
mod fa_interface {
    use evolve_core::{AccountId, Environment, ErrorCode, SdkResult};
    use evolve_macros::{exec, query};
    pub const ERR_NOT_PAYABLE: ErrorCode = 0;

    #[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
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
    }
}
