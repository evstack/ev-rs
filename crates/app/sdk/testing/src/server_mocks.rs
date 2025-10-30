use evolve_core::{AccountCode, ErrorCode, ReadonlyKV};
use evolve_stf_traits::{
    AccountsCodeStorage, StateChange, WritableAccountsCodeStorage, WritableKV,
};
use std::collections::HashMap;

pub struct AccountStorageMock {
    codes: HashMap<String, Box<dyn AccountCode>>,
}

impl AccountStorageMock {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new(),
        }
    }
}

impl Default for AccountStorageMock {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountsCodeStorage for AccountStorageMock {
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R,
    {
        let code = self.codes.get(identifier).map(|e| e.as_ref());
        Ok(f(code))
    }

    fn list_identifiers(&self) -> Vec<String> {
        self.codes.keys().cloned().collect()
    }
}

impl WritableAccountsCodeStorage for AccountStorageMock {
    fn add_code(&mut self, code: impl AccountCode + 'static) -> Result<(), ErrorCode> {
        self.codes.insert(code.identifier(), Box::new(code));
        Ok(())
    }
}

pub struct StorageMock {
    state: HashMap<Vec<u8>, Vec<u8>>,
}

impl StorageMock {
    pub fn new() -> Self {
        Self {
            state: HashMap::new(),
        }
    }
}

impl Default for StorageMock {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadonlyKV for StorageMock {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        Ok(self.state.get(key).cloned())
    }
}

impl WritableKV for StorageMock {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        for state in changes {
            match state {
                StateChange::Set { key, value } => {
                    self.state.insert(key, value);
                }
                StateChange::Remove { key } => {
                    self.state.remove(&key);
                }
            }
        }

        Ok(())
    }
}
