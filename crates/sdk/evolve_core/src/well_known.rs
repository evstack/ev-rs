use crate::AccountId;

pub const ACCOUNT_IDENTIFIER_PREFIX: u8 = 0;
pub const ACCOUNT_IDENTIFIER_SINGLETON_PREFIX: u8 = 1;

pub const RUNTIME_ACCOUNT_ID: AccountId=  AccountId(0);
pub const STORAGE_ACCOUNT_ID: AccountId = AccountId(1);