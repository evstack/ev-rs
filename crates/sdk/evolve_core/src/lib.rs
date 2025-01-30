use crate::encoding::{Decodable, Encodable};
use borsh::{BorshDeserialize, BorshSerialize};

pub mod encoding;
pub mod mocks; // TODO: make test
pub mod well_known;

pub type ErrorCode = u64;
pub const ERR_ENCODING: ErrorCode = 1;
pub const ERR_UNKNOWN_FUNCTION: ErrorCode = 2;

pub type SdkResult<T> = Result<T, ErrorCode>;

#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, BorshSerialize, BorshDeserialize,
)]
pub struct AccountId(u128);

impl AccountId {
    pub fn increase(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl AccountId {
    pub fn new(u: impl Into<u128>) -> Self {
        Self(u.into())
    }
}

impl AccountId {
    pub fn as_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().into()
    }
}
#[derive(BorshSerialize, BorshDeserialize, Debug)]
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

#[derive(BorshSerialize, BorshDeserialize, Debug)]
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

pub trait Environment {
    fn whoami(&self) -> AccountId;
    fn sender(&self) -> AccountId;
    fn do_query(
        &self,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn do_exec(
        &mut self,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode {
    fn identifier(&self) -> String;
    fn init(
        &self,
        env: &mut dyn Environment,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn execute(
        &self,
        env: &mut dyn Environment,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn query(
        &self,
        env: &dyn Environment,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
}

pub trait ReadonlyKV {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode>;
}
