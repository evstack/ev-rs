pub mod mocks; // TODO: make test

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
    fn get<'a>(
        &'a self,
        identifier: &str,
    ) -> Result<Option<&'a Box<dyn AccountCode<I>>>, ErrorCode>;
    fn add_code<T: AccountCode<I> +'static>(&mut self, account_code: T) -> Result<(), ErrorCode>;
}
