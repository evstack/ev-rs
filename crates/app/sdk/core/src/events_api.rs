use crate::{AccountId, Message};
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshDeserialize, BorshSerialize, Debug)]
pub struct Event {
    pub source: AccountId,
    pub name: String,
    pub contents: Message,
}
