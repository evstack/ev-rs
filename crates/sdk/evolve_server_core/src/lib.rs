pub mod mocks;

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
    fn get<'a, 'b>(&'a self, identifier: &'b str) -> Result<Option<&'a Box<dyn AccountCode<I>>>, ErrorCode>;
    fn add_code(&mut self, account_code: I) -> Result<(), ErrorCode>;
}
