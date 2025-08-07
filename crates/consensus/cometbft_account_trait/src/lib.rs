use evolve_core::account_impl;

#[account_impl(AbciValsetManagerAccount)]
pub mod consensus_account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::query;

    #[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
    pub enum Pubkey {
        Ed25519([u8; 32]),
    }

    #[derive(BorshDeserialize, BorshSerialize, Clone, Debug)]
    pub struct ValidatorUpdate {
        pub power: u32,
        pub pub_key: Pubkey,
    }

    pub trait AbciValsetManagerAccount {
        #[query]
        fn valset_changes(&self, env: &dyn Environment) -> SdkResult<Vec<ValidatorUpdate>>;
    }
}
