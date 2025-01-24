mod checkpoint;

use std::cell::RefCell;
use evolve_core::{AccountId, Context, InvokeRequest, InvokeResponse, Invoker, SdkResult};
use evolve_server_core::{AccountsCodeStorage, ReadonlyKV, Transaction};
use std::marker::PhantomData;
use std::rc::Rc;

pub enum Error {
    SdkError(evolve_core::ErrorCode),
    StorageError()
}
pub type StfResult<T> = Result<T, Error>;

pub struct TxResult {}

pub struct Stf<Tx>(PhantomData<Tx>);

impl<T> Stf<T> {
    fn apply_tx<S: ReadonlyKV, A: AccountsCodeStorage<ExecCtx>, Tx: Transaction>(
        account_storage: &mut A,
        tx: &Tx,
    ) -> SdkResult<()> {
        todo!("impl")
    }
}

struct ExecCtx {
    whoami: AccountId,
    gas_limit: u64,
    gas_used: RefCell<Rc<u64>>,
}

impl Invoker for ExecCtx {
    fn do_query(
        &self,
        ctx: &Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        todo!()
    }

    fn do_exec(
        &mut self,
        ctx: &mut Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
