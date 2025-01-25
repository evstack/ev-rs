use std::collections::HashMap;
use evolve_core::{AccountCode, ErrorCode, Invoker};
use crate::AccountsCodeStorage;

impl<I: Invoker> AccountsCodeStorage<I> for HashMap<String, Box<dyn AccountCode<I>>> {
    fn get<'a>(&'a self, identifier: &str) -> Result<Option<&'a Box<dyn AccountCode<I>>>, ErrorCode> {
        Ok(self.get(identifier))
    }

    fn add_code(&mut self, account_code: I) -> Result<(), ErrorCode> {
        todo!()
    }
}