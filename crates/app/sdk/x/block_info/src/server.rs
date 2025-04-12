use crate::account::BlockInfoRef;
use evolve_core::Environment;
use evolve_ns::resolve_as_ref;
use evolve_server_core::{BeginBlocker, Block};
use std::marker::PhantomData;

pub struct SetTimeBeginBlocker<T>(PhantomData<T>);

pub trait BlockTimeProvider {
    fn time_unix_ms(&self) -> u64;
}

// TODO: removing the phantom causes unconstrained type parameters issues
impl<B, Tx> BeginBlocker<B> for SetTimeBeginBlocker<Tx>
where
    B: Block<Tx> + BlockTimeProvider,
{
    fn begin_block(block: &B, env: &mut dyn Environment) {
        resolve_as_ref::<BlockInfoRef>("block_info".to_string(), env)
            .unwrap()
            .unwrap()
            .set_block_info(block.height(), block.time_unix_ms(), env)
            .unwrap()
    }
}
