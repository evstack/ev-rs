mod checkpoint;

use crate::checkpoint::Checkpoint;
use evolve_core::well_known::{
    ACCOUNT_IDENTIFIER_PREFIX, ACCOUNT_IDENTIFIER_SINGLETON_PREFIX, RUNTIME_ACCOUNT_ID,
};
use evolve_core::{
    AccountCode, AccountId, Context, InvokeRequest, InvokeResponse, Invoker as InvokerTrait,
    Message, SdkResult,
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
        S: ReadonlyKV + 'a,
        A: AccountsCodeStorage<Invoker<'a, S, A>>,
        Tx: Transaction,
    >(
        checkpoint: &'a mut Checkpoint<'a, S>,
        account_codes: &'a mut A,
        from: AccountId,
        identifier: &str,
        msg: impl Into<Message>,
    ) -> Result<(AccountId, InvokeResponse), Error> {
        /*
        let request = InvokeRequest::new(0, Message::from(msg.into()));
        let account = account_codes.get(identifier).unwrap().unwrap();
        let new_id = Self::next_account_number(checkpoint)?;
        let mut ctx = Context::new(from, new_id);

        let mut invoker = Invoker {
            gas_limit: 0,
            gas_used: RefCell::new(Rc::new(0)),
            storage: checkpoint,
            account_codes_storage: account_codes,
        };

        Ok(account
            .execute(&mut invoker, &mut ctx, request)
            .map(|v| (new_id, v))?)

         */
        todo!("impl")
    }
    fn apply_tx<
        'a,
        S: ReadonlyKV + 'a,
        A: AccountsCodeStorage<Invoker<'a, S, A>> + 'a,
        Tx: Transaction,
    >(
        storage: &'a S,
        account_codes: &mut A,
        tx: &Tx,
    ) -> SdkResult<()> {
        todo!("impl")
    }

    fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage<Invoker<'a, S, A>> + 'a>(
        storage: &'a S,
        account_storage: &mut A,
        from: AccountId,
        to: AccountId,
        req: InvokeRequest,
    ) {
        todo!("impl")
    }
}

struct Invoker<'a, S, A> {
    gas_limit: u64,
    gas_used: RefCell<Rc<u64>>,
    storage: RefCell<Rc<Checkpoint<'a, S>>>,
    account_codes_storage: RefCell<Rc<&'a mut A>>,
}

impl<S: ReadonlyKV, A: AccountsCodeStorage<Self>> InvokerTrait for Invoker<'_, S, A> {
    fn do_query(
        &self,
        _ctx: &Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let account = self.load_account(to)?;
        let ctx = Context::new(RUNTIME_ACCOUNT_ID, to);

        let invoker = Invoker {
            gas_limit: self.gas_limit,
            gas_used: self.gas_used.clone(), // TODO charge gas
            storage: self.storage.clone(),
            account_codes_storage: self.account_codes_storage.clone(),
        };

        // TODO: use checkpoint to revert on error

        Ok(account.query(&invoker, &ctx, data)?)
    }

    fn do_exec(
        &mut self,
        ctx: &mut Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        // TODO: use checkpoint to revert on error
        todo!()
    }
}

impl<'a, S: ReadonlyKV, A: AccountsCodeStorage<Self>> Invoker<'a, S, A> {
    fn new(gas_limit: u64, storage: Checkpoint<'a, S>, account_code_storage: &'a mut A) -> Self {
        Self {
            gas_limit,
            gas_used: RefCell::new(Rc::new(0)),
            storage: RefCell::new(Rc::new(storage)),
            account_codes_storage: RefCell::new(Rc::new(account_code_storage)),
        }
    }

    fn load_account(&self, account: AccountId) -> SdkResult<Box<&dyn AccountCode<Self>>> {
        let code_id = self
            .get_account_code_identifier_for_account(account)?
            .unwrap(); // TODO unwrap
        Ok(self.account_codes_storage.borrow().get(&code_id)?.unwrap()) // TODO unwrap
    }
    fn get_account_code_identifier_for_account(
        &self,
        account: AccountId,
    ) -> StfResult<Option<String>> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        Ok(self
            .storage
            .borrow()
            .get(&key)?
            .map(|e| String::from_utf8_lossy(&e).to_string())?) // TODO
    }

    fn set_account_code_identifier_for_account(
        &mut self,
        account: AccountId,
        account_identifier: &'static str,
    ) -> Result<(), Error> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        Ok(self
            .storage
            .borrow_mut()
            .set(&key, account_identifier.as_bytes().to_vec())?)
    }

    fn get_account_code_identifier_for_account_key(account: AccountId) -> Vec<u8> {
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account.as_bytes());
        key
    }

    fn next_account_number(&self) -> StfResult<AccountId> {
        let last = self
            .storage
            .borrow()
            .get(&vec![ACCOUNT_IDENTIFIER_SINGLETON_PREFIX])?
            .unwrap_or(u16::MAX.to_be_bytes().into());

        Ok(AccountId::try_from(last.as_slice())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
