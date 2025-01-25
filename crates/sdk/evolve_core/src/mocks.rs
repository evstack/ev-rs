use std::collections::HashMap;
use crate::{ErrorCode, ReadonlyKV};

impl ReadonlyKV for HashMap<Vec<u8>, Vec<u8>> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.get(key).map_or(Ok(None), |v| Ok(Some(v.clone())))
    }
}