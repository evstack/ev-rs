pub mod mocks; // TODO: make test

use evolve_core::{AccountCode, AccountId, ErrorCode, Invoker, Message};

pub trait Transaction {
    fn sender(&self) -> AccountId;
    fn recipient(&self) -> AccountId;
    fn message(&self) -> Message;
    fn gas_limit(&self) -> u64;
}

/// Stores account code.
pub trait AccountsCodeStorage {
    /// TODO: this probably needs to consume gas, and should accept gas.
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R;
    fn add_code<T: AccountCode + 'static>(&mut self, account_code: T) -> Result<(), ErrorCode>;
}
