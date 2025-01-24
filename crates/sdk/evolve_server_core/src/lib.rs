use evolve_core::{AccountCode, AccountId, Invoker, Message};

pub trait Transaction {
    fn sender(&self) -> AccountId;
    fn recipient(&self) -> AccountId;
    fn message(&self) -> Message;
    fn gas_limit(&self) -> u64;
}

/// Stores account code.
pub trait AccountsCodeStorage<I: Invoker> {
    type Error;
    /// TODO: this probably needs to consume gas, and should accept gas.
    fn get(&self, identifier: &str) -> Result<Option<Box<dyn AccountCode<I>>>, Self::Error>;
    fn add_account(&mut self, account_code: I) -> Result<(), Self::Error>;
}

pub trait ReadonlyKV {
    type Error;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error>;
}