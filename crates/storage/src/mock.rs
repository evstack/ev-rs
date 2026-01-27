use crate::{CommitHash, Operation, Storage};
use async_trait::async_trait;
use evolve_core::{ErrorCode, ReadonlyKV};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Simple in-memory storage for fast dev/test runs.
///
/// This does not provide persistence or Merkle commitments.
#[derive(Clone, Default)]
pub struct MockStorage {
    state: Arc<RwLock<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MockStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ReadonlyKV for MockStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        let guard = self.state.read().expect("mock storage read lock poisoned");
        Ok(guard.get(key).cloned())
    }
}

#[async_trait(?Send)]
impl Storage for MockStorage {
    async fn commit(&self) -> Result<CommitHash, ErrorCode> {
        Ok(CommitHash::from([0u8; 32]))
    }

    async fn batch(&self, operations: Vec<Operation>) -> Result<(), ErrorCode> {
        let mut guard = self
            .state
            .write()
            .expect("mock storage write lock poisoned");

        for op in operations {
            match op {
                Operation::Set { key, value } => {
                    guard.insert(key, value);
                }
                Operation::Remove { key } => {
                    guard.remove(&key);
                }
            }
        }

        Ok(())
    }
}
