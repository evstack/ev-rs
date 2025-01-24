use evolve_server_core::ReadonlyKV;

pub (crate) struct Checkpoint<S> {
    storage: S
}

impl<S> Checkpoint<S> {
    pub (crate) fn new(storage: S) -> Self {
        Self { storage, }
    }
}

impl <S: ReadonlyKV> Checkpoint<S> {
    pub (crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, S::Error> {
        self.storage.get(key)
    }
}