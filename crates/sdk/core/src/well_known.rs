use crate::encoding::{Decodable, Encodable};
use crate::{AccountId, ErrorCode, InvokableMessage, InvokeRequest, InvokeResponse, Message};
use borsh::{BorshDeserialize, BorshSerialize};

pub const ACCOUNT_IDENTIFIER_PREFIX: u8 = 0;
pub const ACCOUNT_IDENTIFIER_SINGLETON_PREFIX: u8 = 1;

pub const RUNTIME_ACCOUNT_ID: AccountId = AccountId(0);
pub const STORAGE_ACCOUNT_ID: AccountId = AccountId(1);
pub const STORAGE_ACCOUNT_SET_FUNCTION_IDENTIFIER: u64 = 1;
pub const STORAGE_ACCOUNT_GET_FUNCTION_IDENTIFIER: u64 = 1;

pub const RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER: u64 = 1;

pub const INIT_FUNCTION_IDENTIFIER: u64 = 0;

#[derive(BorshSerialize, BorshDeserialize)]
pub struct CreateAccountRequest {
    pub code_id: String,
    pub init_message: Message,
}

impl InvokableMessage for CreateAccountRequest {
    const FUNCTION_IDENTIFIER: u64 = RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "create_account";
}
impl TryFrom<CreateAccountRequest> for InvokeRequest {
    type Error = ErrorCode;

    fn try_from(value: CreateAccountRequest) -> Result<Self, Self::Error> {
        Ok(InvokeRequest::new(
            RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER,
            Message::from(value.encode()?),
        ))
    }
}

impl TryFrom<&InvokeRequest> for CreateAccountRequest {
    type Error = ErrorCode;
    fn try_from(value: &InvokeRequest) -> Result<Self, Self::Error> {
        Ok(CreateAccountRequest::decode(value.message_bytes())?)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct CreateAccountResponse {
    pub new_account_id: AccountId,
    pub init_response: Message,
}

impl TryFrom<CreateAccountResponse> for InvokeResponse {
    type Error = ErrorCode;
    fn try_from(value: CreateAccountResponse) -> Result<Self, Self::Error> {
        Ok(InvokeResponse::new(Message::from(value.encode()?)))
    }
}

impl TryFrom<InvokeResponse> for CreateAccountResponse {
    type Error = ErrorCode;
    fn try_from(value: InvokeResponse) -> Result<Self, Self::Error> {
        Ok(CreateAccountResponse::decode(value.response_bytes())?)
    }
}
#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct StorageSetRequest {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl TryFrom<StorageSetRequest> for InvokeRequest {
    type Error = ErrorCode;
    fn try_from(value: StorageSetRequest) -> Result<Self, Self::Error> {
        Ok(InvokeRequest::new(
            STORAGE_ACCOUNT_SET_FUNCTION_IDENTIFIER,
            Message::from(value.encode()?),
        ))
    }
}

impl TryFrom<&InvokeRequest> for StorageSetRequest {
    type Error = ErrorCode;
    fn try_from(value: &InvokeRequest) -> Result<Self, Self::Error> {
        Ok(StorageSetRequest::decode(value.message_bytes())?)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct EmptyResponse {}

impl TryFrom<EmptyResponse> for InvokeResponse {
    type Error = ErrorCode;
    fn try_from(value: EmptyResponse) -> Result<Self, Self::Error> {
        Ok(InvokeResponse::new(Message::from(value.encode()?)))
    }
}

impl TryFrom<InvokeResponse> for EmptyResponse {
    type Error = ErrorCode;
    fn try_from(value: InvokeResponse) -> Result<Self, Self::Error> {
        Ok(EmptyResponse::decode(value.response_bytes())?)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct StorageGetRequest {
    pub account_id: AccountId,
    pub key: Vec<u8>,
}

impl TryFrom<StorageGetRequest> for InvokeRequest {
    type Error = ErrorCode;
    fn try_from(value: StorageGetRequest) -> Result<Self, Self::Error> {
        Ok(InvokeRequest::new(
            STORAGE_ACCOUNT_GET_FUNCTION_IDENTIFIER,
            Message::from(value.encode()?),
        ))
    }
}

impl TryFrom<&InvokeRequest> for StorageGetRequest {
    type Error = ErrorCode;
    fn try_from(value: &InvokeRequest) -> Result<Self, Self::Error> {
        Ok(StorageGetRequest::decode(value.message_bytes())?)
    }
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct StorageGetResponse {
    pub value: Option<Vec<u8>>,
}

impl TryFrom<StorageGetResponse> for InvokeResponse {
    type Error = ErrorCode;
    fn try_from(value: StorageGetResponse) -> Result<Self, Self::Error> {
        Ok(InvokeResponse::new(Message::from(value.encode()?)))
    }
}

impl TryFrom<InvokeResponse> for StorageGetResponse {
    type Error = ErrorCode;
    fn try_from(value: InvokeResponse) -> Result<Self, Self::Error> {
        StorageGetResponse::decode(value.response_bytes())
    }
}
