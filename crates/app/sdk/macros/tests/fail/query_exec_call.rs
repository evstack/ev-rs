// Minimal compile-fail test: query context should not allow exec.
pub mod evolve_core {
    pub struct AccountId(pub [u8; 32]);
    pub struct InvokeRequest;
    pub struct InvokeResponse;
    pub struct FungibleAsset;
    pub struct SdkError;
    pub type SdkResult<T> = Result<T, SdkError>;

    pub trait EnvironmentQuery {
        fn do_query(&mut self, _to: AccountId, _data: &InvokeRequest) -> SdkResult<InvokeResponse>;
    }
}

use evolve_core::{AccountId, EnvironmentQuery, InvokeRequest, SdkResult};

fn bad_query(env: &mut dyn EnvironmentQuery) -> SdkResult<()> {
    let dummy = InvokeRequest;
    env.do_exec(AccountId([0u8; 32]), &dummy, vec![])?;
    Ok(())
}

fn main() {
    println!("Test compiled successfully");
}
