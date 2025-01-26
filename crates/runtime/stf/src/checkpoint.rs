use evolve_core::{ErrorCode, ReadonlyKV};
use std::collections::HashMap;

// TODO: make this work

enum StateChange<'a> {
    Set {
        key: &'a [u8],
        previous_value: Option<Vec<u8>>,
    },
    Remove {
        key: &'a [u8],
        previous_value: Option<Vec<u8>>,
    },
}

impl StateChange<'_> {
    fn revert(self, map: &mut HashMap<Vec<u8>, Vec<u8>>) {
        match self {
            StateChange::Set {
                key,
                previous_value,
            } => match previous_value {
                Some(previous_value) => {
                    map.insert(key.to_vec(), previous_value);
                }
                None => {
                    map.remove(key);
                }
            },
            StateChange::Remove {
                key,
                previous_value,
            } => {
                match previous_value {
                    Some(previous_value) => {
                        map.insert(key.to_vec(), previous_value);
                    }
                    None => {
                        // no op
                    }
                }
            }
        }
    }
}

pub(crate) struct Checkpoint<'a, S> {
    map: HashMap<Vec<u8>, Vec<u8>>,
    change_list: Vec<StateChange<'a>>,
    storage: &'a S,
}

impl<'a, S> Checkpoint<'a, S> {
    pub(crate) fn new(storage: &'a S) -> Self {
        Self {
            storage,
            map: HashMap::new(),
            change_list: Vec::new(),
        }
    }
}

impl<'a, S: ReadonlyKV> Checkpoint<'a, S> {
    pub(crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        match self.map.get(key) {
            Some(value) => Ok(Some(value.clone())),
            None => self.storage.get(key),
        }
    }

    pub(crate) fn set(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), ErrorCode> {
        self.map.insert(key.to_vec(), value);
        Ok(())
    }

    pub(crate) fn remove(&mut self, key: &[u8]) -> Result<(), ErrorCode> {
        self.map.remove(key);
        Ok(())
    }
    pub(crate) fn checkpoint(&self) -> u64 {
        0
    }

    pub(crate) fn restore(&mut self, checkpoint: u64) {}
}
