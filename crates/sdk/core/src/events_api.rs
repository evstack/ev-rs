use crate::{AccountId, InvokableMessage, Message};
use borsh::{BorshDeserialize, BorshSerialize};

pub const EVENT_HANDLER_ACCOUNT_ID: AccountId = AccountId(2);

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct Event {
    pub source: AccountId,
    pub name: String,
    pub contents: Message,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct EmitEventRequest {
    pub name: String,
    pub contents: Message,
}

impl InvokableMessage for EmitEventRequest {
    const FUNCTION_IDENTIFIER: u64 = 1;
    const FUNCTION_IDENTIFIER_NAME: &'static str = "emit_event";
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct EmitEventResponse {}
