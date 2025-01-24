use evolve_core::{AccountCode, AccountId, ErrorCode, Invoker, Message};

pub trait Transaction {
    fn sender(&self) -> AccountId;
    fn recipient(&self) -> AccountId;
    fn message(&self) -> Message;
    fn gas_limit(&self) -> u64;
}

/// Stores account code.
pub trait AccountsCodeStorage<I: Invoker> {
    /// TODO: this probably needs to consume gas, and should accept gas.
    fn get<'a>(&self, identifier: &str) -> Result<Option<Box<&'a dyn AccountCode<I>>>, ErrorCode>;
    fn add_account(&mut self, account_code: I) -> Result<(), ErrorCode>;
}

pub trait ReadonlyKV {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode>;
}