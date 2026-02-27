use evolve_core::account_impl;
pub use fa_interface::*;

#[account_impl(FungibleAssetInterface)]
mod fa_interface {
    use evolve_core::{AccountId, Environment, EnvironmentQuery, SdkResult};
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
        fn metadata(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<FungibleAssetMetadata>;
        #[query]
        fn get_balance(
            &self,
            account: AccountId,
            env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<Option<u128>>;
        #[query]
        fn total_supply(&self, env: &mut dyn EnvironmentQuery) -> SdkResult<u128>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::{
        AccountId, BlockContext, Environment, EnvironmentQuery, FungibleAsset, InvokeRequest,
        InvokeResponse, SdkResult, ERR_UNKNOWN_FUNCTION,
    };

    struct TestEnv {
        whoami: AccountId,
        sender: AccountId,
        last_query: Option<String>,
        last_exec: Option<String>,
    }

    impl TestEnv {
        fn new() -> Self {
            Self {
                whoami: AccountId::from_u64(1),
                sender: AccountId::from_u64(2),
                last_query: None,
                last_exec: None,
            }
        }
    }

    impl EnvironmentQuery for TestEnv {
        fn whoami(&self) -> AccountId {
            self.whoami
        }

        fn sender(&self) -> AccountId {
            self.sender
        }

        fn funds(&self) -> &[FungibleAsset] {
            &[]
        }

        fn block(&self) -> BlockContext {
            BlockContext::new(0, 0)
        }

        fn do_query(&mut self, _to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
            self.last_query = Some(data.human_name().to_string());
            match data.human_name() {
                "metadata" => InvokeResponse::new(&FungibleAssetMetadata {
                    name: "Token".to_string(),
                    symbol: "TOK".to_string(),
                    decimals: 6,
                    icon_url: "https://example.com/icon.png".to_string(),
                    description: "desc".to_string(),
                }),
                "get_balance" => InvokeResponse::new(&Some(123u128)),
                "total_supply" => InvokeResponse::new(&777u128),
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }
    }

    impl Environment for TestEnv {
        fn do_exec(
            &mut self,
            _to: AccountId,
            data: &InvokeRequest,
            _funds: Vec<FungibleAsset>,
        ) -> SdkResult<InvokeResponse> {
            self.last_exec = Some(data.human_name().to_string());
            match data.human_name() {
                "transfer" => InvokeResponse::new(&()),
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }

        fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
            Ok(())
        }

        fn unique_id(&mut self) -> SdkResult<[u8; 32]> {
            Ok([0u8; 32])
        }
    }

    #[test]
    fn interface_ref_queries_decode_expected_types() {
        let mut env = TestEnv::new();
        let account = AccountId::from_u64(10);
        let fa = FungibleAssetInterfaceRef::new(account);

        let metadata = fa
            .metadata(&mut env)
            .expect("metadata query should succeed");
        assert_eq!(metadata.name, "Token");
        assert_eq!(metadata.symbol, "TOK");
        assert_eq!(metadata.decimals, 6);

        let balance = fa
            .get_balance(AccountId::from_u64(999), &mut env)
            .expect("balance query should succeed");
        assert_eq!(balance, Some(123));

        let supply = fa
            .total_supply(&mut env)
            .expect("total_supply query should succeed");
        assert_eq!(supply, 777);
        assert_eq!(env.last_query.as_deref(), Some("total_supply"));
    }

    #[test]
    fn interface_ref_transfer_dispatches_exec_call() {
        let mut env = TestEnv::new();
        let account = AccountId::from_u64(10);
        let fa = FungibleAssetInterfaceRef::new(account);

        fa.transfer(AccountId::from_u64(11), 55, &mut env)
            .expect("transfer should succeed");

        assert_eq!(env.last_exec.as_deref(), Some("transfer"));
    }
}
