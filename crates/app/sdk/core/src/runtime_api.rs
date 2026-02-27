use crate::message::Message;
use crate::{AccountId, InvokableMessage, InvokeRequest, InvokeResponse};
use borsh::{BorshDeserialize, BorshSerialize};
pub const ACCOUNT_IDENTIFIER_PREFIX: u8 = 0;
pub const ACCOUNT_IDENTIFIER_SINGLETON_PREFIX: u8 = 1;
pub const RUNTIME_ACCOUNT_ID: AccountId = AccountId::from_bytes([0u8; 32]);

/// Storage key for consensus parameters.
/// This is a well-known key that nodes read to validate blocks during sync.
pub const CONSENSUS_PARAMS_KEY: &[u8] = b"consensus_params";

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct CreateAccountRequest {
    pub code_id: String,
    pub init_message: Message,
}

impl InvokableMessage for CreateAccountRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "create_account";
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct CreateAccountResponse {
    pub new_account_id: AccountId,
    pub init_response: Message,
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct RegisterAccountAtIdRequest {
    pub account_id: AccountId,
    pub code_id: String,
    pub init_message: Message,
}

impl InvokableMessage for RegisterAccountAtIdRequest {
    const FUNCTION_IDENTIFIER: u64 = 2;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "register_account_at_id";
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct RegisterAccountAtIdResponse {}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct MigrateRequest {
    pub account_id: AccountId,
    pub new_code_id: String,
    pub execute_message: InvokeRequest,
}

impl InvokableMessage for MigrateRequest {
    const FUNCTION_IDENTIFIER: u64 = 3;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "migrate";
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct MigrateResponse {
    pub migrate_response: InvokeResponse,
}
