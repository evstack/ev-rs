use borsh::{BorshDeserialize, BorshSerialize};
use crate::encoding::{Decodable, Encodable};

pub mod well_known;
pub mod mocks; // TODO: make test
pub mod encoding;

pub type ErrorCode = u64;
pub const ERR_ENCODING: ErrorCode = 1;
pub const ERR_UNKNOWN_FUNCTION: ErrorCode = 2;

pub type SdkResult<T> = Result<T, ErrorCode>;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, BorshSerialize, BorshDeserialize)]
pub struct AccountId(u128);

impl AccountId {
    pub fn new(u: impl Into<u128>) -> Self {
        Self(u.into())
    }
}

impl TryFrom<&[u8]> for AccountId {
    type Error = ErrorCode;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(AccountId(u128::from_be_bytes(value.try_into().map_err(|_| ERR_ENCODING)?)))
    }
}

impl AccountId {
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().into()
    }
}
#[derive(BorshSerialize, BorshDeserialize)]
enum InnerMessage {
    OwnedBytes(Vec<u8>),
}

impl InnerMessage {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            InnerMessage::OwnedBytes(b) => b.as_slice(),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
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
    pub fn decode<T: Decodable>(&self) -> SdkResult<T> {
        T::decode(self.message_bytes())
    }
}

impl InvokeRequest {
    pub fn function(&self) -> u64 {
        self.function_identifier
    }

    pub fn message_bytes(&self) -> &[u8] {
        self.message.inner.as_bytes()
    }
}

impl InvokeRequest {
    pub fn new(function_identifier: u64, message: Message) -> Self {
        InvokeRequest {
            function_identifier,
            message,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        self.message.inner.as_bytes()
    }
}

/// Defines the response of an [`InvokeRequest`]
pub struct InvokeResponse {
    response: Message,
}

impl InvokeResponse {
    pub(crate) fn response_bytes(&self) -> &[u8] {
        self.response.inner.as_bytes()
    }
}

impl InvokeResponse {
    pub fn into_message(self) -> Message {
        self.response
    }
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

    pub fn whoami(&self) -> AccountId {
        self.whoami
    }

    pub fn sender(&self) -> AccountId {
        self.sender
    }
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode<I: Invoker> {
    fn identifier(&self) -> String;
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

pub trait ReadonlyKV {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode>;
}