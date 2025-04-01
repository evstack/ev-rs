use crate::{AccountsCodeStorage, StateChange, WritableKV};
use evolve_core::{AccountCode, ErrorCode};
use std::collections::HashMap;

pub struct MockedAccountsCodeStorage {
    codes: HashMap<String, Box<dyn AccountCode>>,
}

impl Default for MockedAccountsCodeStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockedAccountsCodeStorage {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new(),
        }
    }

    pub fn add_code<T: AccountCode + 'static>(&mut self, account_code: T) -> Result<(), ErrorCode> {
        self.codes
            .insert(account_code.identifier(), Box::new(account_code));
        Ok(())
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
}

impl WritableKV for HashMap<Vec<u8>, Vec<u8>> {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        for state in changes {
            match state {
                StateChange::Set { key, value } => {
                    self.insert(key, value);
                }
                StateChange::Remove { key } => {
                    self.remove(&key);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
