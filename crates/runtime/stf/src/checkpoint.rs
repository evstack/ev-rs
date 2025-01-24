use crate::Error;
use evolve_core::ErrorCode;
use evolve_server_core::ReadonlyKV;
use std::collections::HashMap;

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
    storage: S,
}

impl<S> Checkpoint<'_, S> {
    pub(crate) fn new(storage: S) -> Self {
        Self {
            storage,
            map: HashMap::new(),
            change_list: Vec::new(),
        }
    }
}

impl<S: ReadonlyKV> Checkpoint<'_, S> {
    pub(crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.storage.get(key)
    }

    pub(crate) fn set(&mut self, key: &[u8], value: &[u8]) -> Result<(), ErrorCode> {
        todo!("impl")
    }
}
