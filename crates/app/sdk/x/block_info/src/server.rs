use crate::account::BlockInfoRef;
use evolve_core::{AccountId, Environment};
use evolve_server_core::{BeginBlocker, Block};
use std::marker::PhantomData;

pub struct SetTimeBeginBlocker<T> {
    block_info_id: AccountId,
    _phantom: PhantomData<T>,
}

impl<T> SetTimeBeginBlocker<T> {
    pub const fn new(block_info_id: AccountId) -> Self {
        Self {
            block_info_id,
            _phantom: PhantomData,
        }
    }
}

pub trait BlockTimeProvider {
    fn time_unix_ms(&self) -> u64;
}

// TODO: removing the phantom causes unconstrained type parameters issues
impl<B, Tx> BeginBlocker<B> for SetTimeBeginBlocker<Tx>
where
    B: Block<Tx> + BlockTimeProvider,
{
    fn begin_block(&self, block: &B, env: &mut dyn Environment) {
        BlockInfoRef::from(self.block_info_id)
            .set_block_info(block.height(), block.time_unix_ms(), env)
            .unwrap()
    }
}
