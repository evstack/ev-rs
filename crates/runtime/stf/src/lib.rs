mod checkpoint;
#[cfg(test)]
mod test;

use crate::checkpoint::Checkpoint;
use evolve_core::well_known::{
    CreateAccountRequest, CreateAccountResponse, ACCOUNT_IDENTIFIER_PREFIX,
    ACCOUNT_IDENTIFIER_SINGLETON_PREFIX, INIT_FUNCTION_IDENTIFIER, RUNTIME_ACCOUNT_ID,
    RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER,
};
use evolve_core::{
    AccountCode, AccountId, Context, InvokeRequest, InvokeResponse, Invoker as InvokerTrait,
    Message, ReadonlyKV, SdkResult, ERR_UNKNOWN_FUNCTION,
};
use evolve_server_core::{AccountsCodeStorage, Transaction};
use std::cell::{RefCell};
use std::marker::PhantomData;
use std::ops::Deref;
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
    fn apply_tx<
        'a,
        S: ReadonlyKV + 'a,
        A: AccountsCodeStorage<Invoker<'a, S, A>> + 'a,
        Tx: Transaction,
    >(
        storage: &'a S,
        account_codes: &mut A,
        tx: &Tx,
    ) -> StfResult<()> {
        todo!("impl")
    }

    pub(crate) fn create_account<
        'a,
        S: ReadonlyKV,
        A: AccountsCodeStorage<Invoker<'a, S, A>> + 'a,
    >(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        code_id: String,
        init_message: Message,
    ) -> SdkResult<Message> {
        let req = InvokeRequest::try_from(CreateAccountRequest{
            code_id,
            init_message,
        })?;


        Self::exec(
            storage,
            account_storage,
            from,
            RUNTIME_ACCOUNT_ID,
            req,
        )
        .map(|v| v.into_message())
    }

    pub(crate) fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage<Invoker<'a, S, A>> + 'a>(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        to: AccountId,
        req: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let writable_storage = Checkpoint::new(storage);
        let mut context = Context::new(from, to);
        let mut invoker = Invoker::new(0, writable_storage, account_storage);
        invoker.do_exec(&mut context, to, req)
    }
}

struct Invoker<'a, S, A> {
    gas_limit: u64,
    gas_used: Rc<RefCell<u64>>,
    storage: Rc<RefCell<Checkpoint<'a, S>>>,
    account_codes_storage: Rc<RefCell<&'a mut A>>,
}

impl<S: ReadonlyKV, A: AccountsCodeStorage<Self>> InvokerTrait for Invoker<'_, S, A> {
    fn do_query(
        &self,
        _ctx: &Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let ctx = Context::new(RUNTIME_ACCOUNT_ID, to);

        let invoker = self.clone();

        self.with_account(to, |code| code.query(&invoker, &ctx, data))?
    }

    fn do_exec(
        &mut self,
        ctx: &mut Context,
        to: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let checkpoint = self.storage.borrow().checkpoint();

        // check if system message
        let resp = if to == RUNTIME_ACCOUNT_ID {
            self.handle_system_request(ctx.whoami(), data)
        } else {
            let mut invoker = self.clone();
            let mut new_ctx = Context::new(ctx.whoami(), to);
            self.with_account(to, |code| code.execute(&mut invoker, &mut new_ctx, data))?
        };
        // restore checkpoint in case of failure and yield back.
        if resp.is_err() {
            self.storage.borrow_mut().restore(checkpoint);
        }
        resp
    }
}

impl<'a, S: ReadonlyKV, A: AccountsCodeStorage<Self>> Invoker<'a, S, A> {
    fn new(gas_limit: u64, storage: Checkpoint<'a, S>, account_code_storage: &'a mut A) -> Self {
        Self {
            gas_limit,
            gas_used: Rc::new(RefCell::new(0)),
            storage: Rc::new(RefCell::new(storage)),
            account_codes_storage: Rc::new(RefCell::new(account_code_storage)),
        }
    }

    fn handle_system_request(
        &mut self,
        from: AccountId,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        match request.function() {
            RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER => {
                let req = CreateAccountRequest::try_from(request)?;
                let resp = self.create_account(from, &req.code_id, req.init_message)
                    .map(|res| {
                        CreateAccountResponse {
                            new_account_id: res.0,
                            init_response: res.1.into_message(),
                        }
                    })?;
                return InvokeResponse::try_from(resp)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn create_account(
        &mut self,
        from: AccountId,
        code_id: &str,
        msg: impl Into<Message>,
    ) -> SdkResult<(AccountId, InvokeResponse)> {
        // get new account and associate it with new code ID
        let new_account_id = self.next_account_number()?;
        self.set_account_code_identifier_for_account(new_account_id, code_id)?;
        // prepare request params
        let req = InvokeRequest::new(INIT_FUNCTION_IDENTIFIER, msg.into());
        let mut invoker = self.clone();
        let mut ctx = Context::new(from, new_account_id);
        // do account init
        self.with_account(new_account_id, |code| {
            code.init(&mut invoker, &mut ctx, req)
        })?
        .map(|r| (new_account_id, r))
    }

    fn with_account<R>(
        &self,
        account: AccountId,
        f: impl FnOnce(&Box<dyn AccountCode<Self>>) -> R,
    ) -> SdkResult<R> {
        let code_id = self
            .get_account_code_identifier_for_account(account)?
            .unwrap(); // TODO

        let storage_borrow = self.account_codes_storage.borrow();

        // 3. Get a reference out of the storage
        let code = storage_borrow.get(&code_id)?.unwrap(); // TODO

        Ok(f(code))
    }
    fn get_account_code_identifier_for_account(
        &self,
        account: AccountId,
    ) -> SdkResult<Option<String>> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        Ok(self
            .storage
            .borrow()
            .get(&key)?
            .map(|e| String::from_utf8_lossy(&e).to_string())) // TODO
    }

    fn set_account_code_identifier_for_account(
        &mut self,
        account: AccountId,
        account_identifier: &str,
    ) -> SdkResult<()> {
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

    fn next_account_number(&self) -> SdkResult<AccountId> {
        let last = self
            .storage
            .borrow()
            .get(&vec![ACCOUNT_IDENTIFIER_SINGLETON_PREFIX])?
            .unwrap_or(u16::MAX.to_be_bytes().into());

        Ok(AccountId::try_from(last.as_slice())?)
    }

    fn clone(&self) -> Self {
        Self {
            gas_limit: self.gas_limit,
            gas_used: self.gas_used.clone(), // TODO charge gas
            storage: self.storage.clone(),
            account_codes_storage: self.account_codes_storage.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
