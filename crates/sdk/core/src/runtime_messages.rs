use crate::message::Message;
use crate::{AccountId, InvokableMessage};
use borsh::{BorshDeserialize, BorshSerialize};

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
