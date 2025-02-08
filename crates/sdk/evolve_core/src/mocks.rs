use crate::{ErrorCode, ReadonlyKV};
use std::collections::HashMap;

impl ReadonlyKV for HashMap<Vec<u8>, Vec<u8>> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.get(key).map_or(Ok(None), |v| Ok(Some(v.clone())))
    }
}
