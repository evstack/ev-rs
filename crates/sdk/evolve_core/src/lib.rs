use std::fmt::Error;

pub mod well_known;

pub type ErrorCode = u64;
pub const ENCODING_ERROR: ErrorCode = 1;

pub type SdkResult<T> = Result<T, ErrorCode>;

#[derive(Copy, Clone)]
pub struct AccountId(u128);

impl TryFrom<&[u8]> for AccountId {
    type Error = ErrorCode;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(AccountId(u128::from_be_bytes(value.try_into().map_err(|_| ENCODING_ERROR)?)))
    }
}

impl AccountId {
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().into()
    }
}

enum InnerMessage {
    OwnedBytes(Vec<u8>),
}

/// Defines a message, the internals of this type are hidden such that we can improve them later
/// for performance.
pub struct Message {
    inner: InnerMessage,
}

impl From<Vec<u8>> for Message {
    fn from(bytes: Vec<u8>) -> Self {
        Message {
            inner: InnerMessage::OwnedBytes(bytes),
        }
    }
}

/// Defines a request to invoke a method in an account.
pub struct InvokeRequest {
    /// Defines the identifier of the function.
    function_identifier: u64,
    /// Defines the message argument of the function.
    message: Message,
}

impl InvokeRequest {
    pub fn new(function_identifier: u64, message: Message) -> Self {
        InvokeRequest {
            function_identifier,
            message,
        }
    }
}

/// Defines the response of an [`InvokeRequest`]
pub struct InvokeResponse {
    response: Message,
}

impl InvokeResponse {
    pub fn new(response: Message) -> Self {
        InvokeResponse { response }
    }
}

pub trait Invoker {
    fn do_query(
        &self,
        ctx: &Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn do_exec(
        &mut self,
        ctx: &mut Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
}

/// Defines the execution context.
pub struct Context {
    whoami: AccountId,
    sender: AccountId,
}

impl Context {
    pub fn new(sender: AccountId, whoami: AccountId) -> Self {
        Context { whoami, sender }
    }
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode<I: Invoker> {
    fn identifier(&self) -> &'static str;
    fn init(
        &self,
        invoker: &mut I,
        ctx: &mut Context,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn execute(
        &self,
        invoker: &mut I,
        ctx: &mut Context,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn query(
        &self,
        invoker: &I,
        ctx: &Context,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
}
