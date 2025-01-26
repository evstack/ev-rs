use borsh::{BorshDeserialize, BorshSerialize};
use crate::{AccountId, ErrorCode, InvokeRequest, InvokeResponse, Message, ERR_ENCODING};

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

#[derive(BorshDeserialize, BorshSerialize)]
pub struct CreateAccountResponse {
    pub new_account_id: AccountId,
    pub init_response: Message,
}
