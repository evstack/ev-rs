mod checkpoint;

use crate::checkpoint::Checkpoint;
use evolve_core::well_known::{ACCOUNT_IDENTIFIER_PREFIX, ACCOUNT_IDENTIFIER_SINGLETON_PREFIX};
use evolve_core::{
    AccountId, Context, InvokeRequest, InvokeResponse, Invoker as InvokerTrait, Message, SdkResult,
};
use evolve_server_core::{AccountsCodeStorage, ReadonlyKV, Transaction};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(Debug)]
pub enum Error {
    SdkError(evolve_core::ErrorCode),
    StorageError(),
}

impl From<evolve_core::ErrorCode> for Error {
    fn from(code: evolve_core::ErrorCode) -> Self {
        Error::SdkError(code)
    }
}

pub type StfResult<T> = Result<T, Error>;

pub struct TxResult {}

pub struct Stf<Tx>(PhantomData<Tx>);

impl<T> Stf<T> {
    pub fn create_account<
        'a,
        S: ReadonlyKV,
        A: AccountsCodeStorage<Invoker<'a, S>>,
        Tx: Transaction,
    >(
        checkpoint: &mut Checkpoint<S>,
        account_codes: &'a A,
        from: AccountId,
        identifier: &str,
        msg: impl Into<Message>,
    ) -> Result<(AccountId, InvokeResponse), Error> {
        let request = InvokeRequest::new(0, Message::from(msg.into()));
        let account = account_codes.get(identifier).unwrap().unwrap();
        let new_id = Self::next_account_number(checkpoint)?;
        let mut ctx = Context::new(from, new_id);

        let mut invoker = Invoker {
            gas_limit: 0,
            gas_used: RefCell::new(Rc::new(0)),
            storage: checkpoint,
        };

        Ok(account
            .execute(&mut invoker, &mut ctx, request)
            .map(|v| (new_id, v))?)
    }
    fn apply_tx<'a, S: ReadonlyKV, A: AccountsCodeStorage<Invoker<'a, S>>, Tx: Transaction>(
        storage: &'a S,
        account_codes: &mut A,
        tx: &Tx,
    ) -> SdkResult<()> {
        todo!("impl")
    }

    fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage<Invoker<'a, S>>>(
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

        // make ctx
        let ctx = Context::new(from, to);
    }

    fn get_account_code_identifier_for_account<S: ReadonlyKV>(
        account: AccountId,
        storage: &S,
    ) -> Result<Option<String>, Error> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        Ok(storage
            .get(&key)?
            .map(|e| String::from_utf8_lossy(&e).to_string())) // TODO
    }

    fn set_account_code_identifier_for_account<S: ReadonlyKV>(
        account: AccountId,
        account_identifier: &'static str,
        storage: &mut Checkpoint<S>,
    ) -> Result<(), Error> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        Ok(storage.set(&key, account_identifier.as_bytes().to_vec())?)
    }

    fn get_account_code_identifier_for_account_key(account: AccountId) -> Vec<u8> {
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account.as_bytes());
        key
    }

    fn next_account_number<S: ReadonlyKV>(storage: &mut Checkpoint<S>) -> StfResult<AccountId> {
        let last = storage
            .get(&vec![ACCOUNT_IDENTIFIER_SINGLETON_PREFIX])?
            .unwrap_or(u16::MAX.to_be_bytes().into());

        Ok(AccountId::try_from(last.as_slice())?)
    }
}

struct Invoker<'a, S: ReadonlyKV> {
    gas_limit: u64,
    gas_used: RefCell<Rc<u64>>,
    storage: &'a mut Checkpoint<'a, S>,
}

impl<S: ReadonlyKV> InvokerTrait for Invoker<'_, S> {
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
