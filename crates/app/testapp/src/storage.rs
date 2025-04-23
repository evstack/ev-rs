use evolve_core::{ErrorCode, ReadonlyKV};
use evolve_server_core::{StateChange, WritableKV};
use evolve_testing::server_mocks::StorageMock;

#[derive(Default)]
pub struct Storage {
    inner: StorageMock,
    height: u64,
}

impl WritableKV for Storage {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode> {
        self.height += 1;
        self.inner.apply_changes(changes)
    }
}

impl ReadonlyKV for Storage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.inner.get(key)
    }
}

impl evolve_cometbft::consensus::Storage for Storage {
    fn get_latest_block(&self) -> u64 {
        self.height
    }
}
