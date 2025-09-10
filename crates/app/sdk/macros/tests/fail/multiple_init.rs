// Mock necessary items from evolve_core that the macro depends on
pub mod evolve_core {
    pub struct AccountId(pub [u8; 32]);
    pub struct InvokeRequest;
    pub struct InvokeResponse;
    pub trait EnvironmentQuery {}
    pub trait Environment: EnvironmentQuery {}
    pub struct FungibleAsset;
    pub struct SdkError;
    pub type SdkResult<T> = Result<T, SdkError>;
    pub const ERR_UNKNOWN_FUNCTION: SdkError = SdkError;
    pub const ERR_NOT_PAYABLE: SdkError = SdkError;

    impl InvokeRequest {
        pub fn function(&self) -> u64 {
            0
        }
        pub fn get<T>(&self) -> SdkResult<T> {
            unimplemented!()
        }
    }

    impl InvokeResponse {
        pub fn new<T>(_: &T) -> Self {
            Self
        }
    }

    pub trait InvokableMessage {
        const FUNCTION_IDENTIFIER: u64;
        const FUNCTION_IDENTIFIER_NAME: &'static str;
    }

    pub trait AccountCode {
        fn identifier(&self) -> String;
        fn init(
            &self,
            env: &mut dyn EnvironmentQuery,
            request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse>;
        fn execute(
            &self,
            env: &mut dyn Environment,
            request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse>;
        fn query(
            &self,
            env: &mut dyn EnvironmentQuery,
            request: &InvokeRequest,
        ) -> SdkResult<InvokeResponse>;
    }

    pub mod low_level {
        use super::*;

        pub fn create_account<T>(
            _name: String,
            _msg: &T,
            _funds: Vec<FungibleAsset>,
            _env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<(AccountId, T)> {
            unimplemented!()
        }

        pub fn exec_account<T, R>(
            _account_id: AccountId,
            _msg: &T,
            _funds: Vec<FungibleAsset>,
            _env: &mut dyn Environment,
        ) -> SdkResult<R> {
            unimplemented!()
        }

        pub fn query_account<T, R>(
            _account_id: AccountId,
            _msg: &T,
            _env: &mut dyn EnvironmentQuery,
        ) -> SdkResult<R> {
            unimplemented!()
        }
    }
}

// Import the macros we want to test
use evolve_macros::{account_impl, init};

// Mock borsh for serialization (the macro adds derives for borsh)
pub mod borsh {
    pub use ::borsh::*;
}

#[account_impl(MultipleInitAccount)]
mod multiple_init_account {
    use crate::evolve_core::*;

    pub struct MultipleInitAccount {
        balance: u64,
    }

    impl MultipleInitAccount {
        // First init function
        #[init]
        fn initialize(&self, initial_balance: u64, env: &mut dyn Environment) -> SdkResult<()> {
            Ok(())
        }

        // Second init function - this should cause a compile error
        #[init]
        fn another_initialize(
            &self,
            initial_value: u32,
            env: &mut dyn Environment,
        ) -> SdkResult<()> {
            Ok(())
        }
    }
}

fn main() {
    // The compile test expects this code to fail compilation
    println!("Test compiled successfully");
}
