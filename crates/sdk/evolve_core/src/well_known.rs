use borsh::{BorshDeserialize, BorshSerialize};
use crate::{AccountId, ErrorCode, InvokeRequest, InvokeResponse, Message, ERR_ENCODING};
use crate::encoding::{Decodable, Encodable};

pub const ACCOUNT_IDENTIFIER_PREFIX: u8 = 0;
pub const ACCOUNT_IDENTIFIER_SINGLETON_PREFIX: u8 = 1;

pub const RUNTIME_ACCOUNT_ID: AccountId = AccountId(0);
pub const STORAGE_ACCOUNT_ID: AccountId = AccountId(1);

pub const RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER: u64 = 1;

pub const INIT_FUNCTION_IDENTIFIER: u64 = 0;

#[derive(BorshSerialize, BorshDeserialize)]
pub struct CreateAccountRequest {
    pub code_id: String,
    pub init_message: Message,
}

impl TryFrom<CreateAccountRequest> for InvokeRequest {
    type Error = ErrorCode;

    fn try_from(value: CreateAccountRequest) -> Result<Self, Self::Error> {
        Ok(InvokeRequest::new(
            RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER,
            Message::from(value.encode()?)
        ))
    }
}

impl TryFrom<InvokeRequest> for CreateAccountRequest {
    type Error = ErrorCode;
    fn try_from(value: InvokeRequest) -> Result<Self, Self::Error> {
        Ok(
            CreateAccountRequest::decode(value.message_bytes())?
        )
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