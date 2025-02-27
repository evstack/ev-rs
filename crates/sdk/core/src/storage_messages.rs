use crate::{AccountId, InvokableMessage};
use borsh::{BorshDeserialize, BorshSerialize};

pub const STORAGE_ACCOUNT_ID: AccountId = AccountId(1);

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageGetRequest {
    pub account_id: AccountId,
    pub key: Vec<u8>,
}

impl InvokableMessage for StorageGetRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_get";
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageGetResponse {
    pub value: Option<Vec<u8>>,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageSetRequest {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl InvokableMessage for StorageSetRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_set";
}
#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageSetResponse {}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageRemoveRequest {
    pub key: Vec<u8>,
}

impl InvokableMessage for StorageRemoveRequest {
    const FUNCTION_IDENTIFIER: u64 = 2;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "storage_remove";
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StorageRemoveResponse {}
