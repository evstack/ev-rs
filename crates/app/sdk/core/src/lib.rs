use crate::encoding::Encodable;
pub use crate::message::{InvokeRequest, InvokeResponse, Message};
use borsh::{BorshDeserialize, BorshSerialize};

pub mod encoding;
pub mod error;
pub mod events_api;
pub mod low_level;
pub mod message;
pub mod runtime_api;
pub mod storage_api;
pub mod unique_api;

pub use error::ErrorCode;

pub const ERR_ENCODING: ErrorCode = ErrorCode::new(0, "encoding error");
pub const ERR_UNKNOWN_FUNCTION: ErrorCode = ErrorCode::new(1, "unknown function");
pub const ERR_ACCOUNT_NOT_INITIALIZED: ErrorCode = ErrorCode::new(2, "account not initialized");
pub const ERR_UNAUTHORIZED: ErrorCode = ErrorCode::new(3, "unauthorized");
pub const ERR_NOT_PAYABLE: ErrorCode = ErrorCode::new(4, "not payable");

pub type SdkResult<T> = Result<T, ErrorCode>;

#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, BorshSerialize, BorshDeserialize,
)]
pub struct AccountId(u128);

impl AccountId {
    pub fn invalid() -> AccountId {
        AccountId(u128::MAX)
    }
}

impl AccountId {
    pub fn increase(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl AccountId {
    pub const fn new(u: u128) -> Self {
        Self(u)
    }
}

impl AccountId {
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().into()
    }
}

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, Eq, PartialEq)]
pub struct FungibleAsset {
    pub asset_id: AccountId,
    pub amount: u128,
}

pub trait Environment {
    fn whoami(&self) -> AccountId;
    fn sender(&self) -> AccountId;
    fn funds(&self) -> &[FungibleAsset];
    fn do_query(&self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse>;
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse>;
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode: Send + Sync {
    fn identifier(&self) -> String;
    fn init(&self, env: &mut dyn Environment, request: &InvokeRequest)
        -> SdkResult<InvokeResponse>;
    fn execute(
        &self,
        env: &mut dyn Environment,
        request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn query(&self, env: &dyn Environment, request: &InvokeRequest) -> SdkResult<InvokeResponse>;
}

pub trait ReadonlyKV {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode>;
}

pub trait InvokableMessage: Encodable + Clone {
    const FUNCTION_IDENTIFIER: u64;
    const FUNCTION_IDENTIFIER_NAME: &'static str;
}

/// A macro that ensures a condition holds true. If not, returns an error.
///
/// # Usage
///
/// ```rust
/// # use std::error::Error;
/// #
/// # // Suppose we have an enum for our errors:
/// #[derive(Debug, )]
/// enum MyError {
///     SomeError,
///     AnotherError,
/// }
///
/// // Return type must be Result<T, Something>
/// fn example_function(value: i32) -> Result<(), MyError> {
///     use evolve_core::ensure;
///
///     ensure!(value > 10, MyError::SomeError);
///     // Proceed if the condition is satisfied
///     Ok(())
/// }
/// #
/// # fn main() {
/// #     example_function(11).unwrap();
/// # }
/// ```
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $err:expr) => {
        if !$cond {
            return Err($err.into());
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_invoke_request_encode_decode() {}
}
