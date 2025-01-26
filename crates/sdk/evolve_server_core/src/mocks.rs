use std::collections::HashMap;
use evolve_core::{AccountCode, ErrorCode, Invoker};
use crate::AccountsCodeStorage;

pub struct MockedAccountsCodeStorage<I: Invoker> {
    codes: HashMap<String, Box<dyn AccountCode<I>>>
}

impl<I: Invoker> MockedAccountsCodeStorage<I> {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new()
        }
    }
}

impl<I: Invoker> AccountsCodeStorage<I> for MockedAccountsCodeStorage<I> {
    fn get<'a>(&'a self, identifier: &str) -> Result<Option<&'a Box<dyn AccountCode<I>>>, ErrorCode> {
        Ok(self.codes.get(identifier))
    }
    fn add_code<T: AccountCode<I> + 'static>(&mut self, account_code: T) -> Result<(), ErrorCode> {
        self.codes.insert(account_code.identifier(), Box::new(account_code));
        Ok(())
    }
}
