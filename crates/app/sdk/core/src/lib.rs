use crate::encoding::Encodable;
pub use crate::message::{InvokeRequest, InvokeResponse, Message};
use borsh::{BorshDeserialize, BorshSerialize};

// Re-export macros so modules don't need to import evolve_macros directly
pub use evolve_macros::{account_impl, skip_storage, storage, AccountState};

pub mod encoding;
pub mod error;
pub mod events_api;
pub mod fungible_asset;
pub mod low_level;
pub mod message;
pub mod runtime_api;
pub mod schema;
pub mod storage_api;

pub use error::ErrorCode;
pub use fungible_asset::FungibleAsset;

define_error!(ERR_ENCODING, 0x01, "encoding error");
define_error!(ERR_UNKNOWN_FUNCTION, 0x02, "unknown function");
define_error!(ERR_ACCOUNT_NOT_INIT, 0x03, "account not initialized");
define_error!(ERR_UNAUTHORIZED, 0x04, "unauthorized");
define_error!(ERR_NOT_PAYABLE, 0x05, "not payable");
define_error!(ERR_ONE_COIN, 0x06, "one coin");
define_error!(ERR_INCOMPATIBLE_FA, 0x07, "incompatible fungible asset");
define_error!(ERR_INSUFFICIENT_BALANCE, 0x08, "insufficient balance");
define_error!(ERR_OVERFLOW, 0x09, "amount overflow");

#[cfg(feature = "error-decode")]
pub type SdkResult<T> = Result<T, crate::error::DecodedError>;
#[cfg(not(feature = "error-decode"))]
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

    pub const fn inner(&self) -> u128 {
        self.0
    }
}

impl AccountId {
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().into()
    }
}

/// Block context available to all modules during execution.
///
/// Contains metadata about the current block. Accessed via `env.block()`.
#[derive(Copy, Clone, Debug, Default)]
pub struct BlockContext {
    pub height: u64,
    pub time: u64,
}

impl BlockContext {
    pub const fn new(height: u64, time: u64) -> Self {
        Self { height, time }
    }
}

pub trait EnvironmentQuery {
    fn whoami(&self) -> AccountId;
    fn sender(&self) -> AccountId;
    fn funds(&self) -> &[FungibleAsset];
    fn block(&self) -> BlockContext;
    fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse>;
}

pub trait Environment: EnvironmentQuery {
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse>;

    /// Emits an event with the given name and data.
    ///
    /// Events are diagnostic data that get included in transaction/block results.
    /// They do not affect state and are used for indexing and observability.
    fn emit_event(&mut self, name: &str, data: &[u8]) -> SdkResult<()>;

    /// Generates a unique 32-byte identifier.
    ///
    /// The ID is deterministic and unique within the current execution context:
    /// - In transactions: derived from tx hash + per-tx counter
    /// - In begin/end block: derived from block height + per-block counter
    ///
    /// Returns an error if called during a query (queries cannot generate unique IDs).
    fn unique_id(&mut self) -> SdkResult<[u8; 32]>;
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode: Send + Sync {
    fn identifier(&self) -> String;
    fn schema(&self) -> schema::AccountSchema;
    fn init(&self, env: &mut dyn Environment, request: &InvokeRequest)
        -> SdkResult<InvokeResponse>;
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

pub fn one_coin(env: &dyn EnvironmentQuery) -> SdkResult<FungibleAsset> {
    let funds = env.funds();
    ensure!(funds.len() == 1, ERR_ONE_COIN);
    Ok(FungibleAsset {
        asset_id: funds[0].asset_id,
        amount: funds[0].amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use borsh::{BorshDeserialize, BorshSerialize};

    #[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
    struct TestPayload {
        value: u32,
        name: String,
    }

    #[test]
    fn test_invoke_request_encode_decode() {
        let function_id: u64 = 42;
        let human_name = "test_function";
        let payload = TestPayload {
            value: 123,
            name: "test".to_string(),
        };

        // Create InvokeRequest using the public API
        let message = Message::new(&payload).expect("message creation should succeed");
        let request = InvokeRequest::new_from_message(human_name, function_id, message);

        // Verify accessors work
        assert_eq!(request.function(), function_id);
        assert_eq!(request.human_name(), human_name);

        // Verify payload can be retrieved
        let retrieved: TestPayload = request.get().expect("get should succeed");
        assert_eq!(retrieved, payload);
    }
}
