mod checkpoint;

use crate::checkpoint::Checkpoint;
use evolve_core::{AccountId, Context, InvokeRequest, InvokeResponse, Invoker, SdkResult};
use evolve_server_core::{AccountsCodeStorage, ReadonlyKV, Transaction};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

pub enum Error {
    SdkError(evolve_core::ErrorCode),
    StorageError(),
}
pub type StfResult<T> = Result<T, Error>;

pub struct TxResult {}

pub struct Stf<Tx>(PhantomData<Tx>);

impl<T> Stf<T> {
    fn apply_tx<'a, S: ReadonlyKV, A: AccountsCodeStorage<ExecCtx<'a, S>>, Tx: Transaction>(
        storage: &'a S,
        account_storage: &mut A,
        tx: &Tx,
    ) -> SdkResult<()> {
        todo!("impl")
    }

    fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage<ExecCtx<'a, S>>>(
        storage: &'a S,
        account_storage: &mut A,
        from: AccountId,
        to: AccountId,
        req: InvokeRequest,
    ) {
        let identifier = Self::get_account_code_identifier_for_account(to, storage)
            .unwrap()
            .unwrap();
        // load account
        let account = account_storage.get(&identifier).unwrap().unwrap();

        let checkpoint = Checkpoint::new(storage);

    }

    fn get_account_code_identifier_for_account(
        account: AccountId,
        storage: impl ReadonlyKV,
    ) -> Result<Option<String>, Error> {
        todo!("impl")
    }

    fn set_account_code_identifier_for_account<S>(
        account: AccountId,
        account_identifier: &'static str,
        storage: Checkpoint<S>,
    ) -> Result<String, Error> {
    }
}

struct ExecCtx<'a, S: ReadonlyKV> {
    whoami: AccountId,
    gas_limit: u64,
    gas_used: RefCell<Rc<u64>>,
    storage: Checkpoint<'a, S>,
}

impl<S: ReadonlyKV> Invoker for ExecCtx<'_, S> {
    fn do_query(
        &self,
        ctx: &Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        todo!("impl")
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
