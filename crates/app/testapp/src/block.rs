use evolve_server_core::Block;

/// Simple block type for testing purposes.
pub struct TestBlock<T> {
    pub txs: Vec<T>,
    pub height: u64,
}

impl<T> TestBlock<T> {
    pub fn new(height: u64, txs: Vec<T>) -> Self {
        Self { txs, height }
    }

    pub fn make_for_testing(txs: Vec<T>) -> Self {
        Self { txs, height: 1 }
    }
}

impl<T> Block<T> for TestBlock<T> {
    fn height(&self) -> u64 {
        self.height
    }

    fn txs(&self) -> &[T] {
        &self.txs
    }
}
