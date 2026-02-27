use crate::{AccountId, InvokableMessage, Message};
use borsh::{BorshDeserialize, BorshSerialize};

pub const STORAGE_ACCOUNT_ID: AccountId = AccountId::from_u64(1);

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageGetRequest {
    pub account_id: AccountId,
    pub key: Vec<u8>,
}

impl InvokableMessage for StorageGetRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_get";
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageGetResponse {
    pub value: Option<Message>,
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageSetRequest {
    pub key: Vec<u8>,
    pub value: Message,
}

impl InvokableMessage for StorageSetRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_set";
}
#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageSetResponse {}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageRemoveRequest {
    pub key: Vec<u8>,
}

impl InvokableMessage for StorageRemoveRequest {
    const FUNCTION_IDENTIFIER: u64 = 2;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_remove";
}

#[derive(BorshDeserialize, BorshSerialize, Clone)]
pub struct StorageRemoveResponse {}
