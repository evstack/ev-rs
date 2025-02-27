use crate::message::Message;
use crate::{AccountId, InvokableMessage};
use borsh::{BorshDeserialize, BorshSerialize};
pub const ACCOUNT_IDENTIFIER_PREFIX: u8 = 0;
pub const ACCOUNT_IDENTIFIER_SINGLETON_PREFIX: u8 = 1;
pub const RUNTIME_ACCOUNT_ID: AccountId = AccountId(0);

#[derive(BorshDeserialize, BorshSerialize)]
pub struct CreateAccountRequest {
    pub code_id: String,
    pub init_message: Message,
}

impl InvokableMessage for CreateAccountRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "create_account";
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct CreateAccountResponse {
    pub new_account_id: AccountId,
    pub init_response: Message,
}
