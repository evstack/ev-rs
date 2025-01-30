use crate::AccountsCodeStorage;
use evolve_core::{AccountCode, ErrorCode, Environment};
use std::collections::HashMap;

pub struct MockedAccountsCodeStorage {
    codes: HashMap<String, Box<dyn AccountCode>>,
}

impl MockedAccountsCodeStorage {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new(),
        }
    }
}

impl AccountsCodeStorage for MockedAccountsCodeStorage {
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R,
    {
        let code = self.codes.get(identifier).map(|e| e.as_ref());
        Ok(f(code))
    }

    fn add_code<T: AccountCode + 'static>(&mut self, account_code: T) -> Result<(), ErrorCode> {
        self.codes
            .insert(account_code.identifier(), Box::new(account_code));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
