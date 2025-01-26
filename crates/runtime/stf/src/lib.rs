mod checkpoint;
#[cfg(test)]
mod test;

use crate::checkpoint::Checkpoint;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::well_known::{
    CreateAccountRequest, CreateAccountResponse, EmptyResponse, StorageGetRequest,
    StorageGetResponse, StorageSetRequest, ACCOUNT_IDENTIFIER_PREFIX,
    ACCOUNT_IDENTIFIER_SINGLETON_PREFIX, INIT_FUNCTION_IDENTIFIER, RUNTIME_ACCOUNT_ID,
    RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER, STORAGE_ACCOUNT_ID,
};
use evolve_core::{
    AccountCode, AccountId, Context, InvokeRequest, InvokeResponse, Invoker as InvokerTrait,
    Message, ReadonlyKV, SdkResult, ERR_UNKNOWN_FUNCTION,
};
use evolve_server_core::{AccountsCodeStorage, Transaction};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::Rc;

pub struct TxResult {}

pub struct Stf<Tx>(PhantomData<Tx>);

impl<T> Stf<T> {
    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, Tx: Transaction>(
        storage: &'a S,
        account_codes: &mut A,
        tx: &Tx,
    ) {
        todo!("impl")
    }

    pub(crate) fn create_account<'a, S: ReadonlyKV, A: AccountsCodeStorage + 'a>(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        code_id: String,
        init_message: Message,
    ) -> SdkResult<CreateAccountResponse> {
        let req = InvokeRequest::try_from(CreateAccountRequest {
            code_id,
            init_message,
        })?;

        let resp = Self::exec(storage, account_storage, from, RUNTIME_ACCOUNT_ID, req)?;

        Ok(CreateAccountResponse::try_from(resp)?)
    }

    pub(crate) fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage + 'a>(
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

impl<S: ReadonlyKV, A: AccountsCodeStorage> InvokerTrait for Invoker<'_, S, A> {
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

        let resp = match to {
            // check if system
            RUNTIME_ACCOUNT_ID => self.handle_system_exec(ctx.whoami(), data),
            // check if storage
            STORAGE_ACCOUNT_ID => self.handle_storage_exec(ctx.whoami(), data),
            // other account
            _ => {
                let mut invoker = self.clone();
                let mut new_ctx = Context::new(ctx.whoami(), to);
                self.with_account(to, |code| code.execute(&mut invoker, &mut new_ctx, data))?
            }
        };

        // restore checkpoint in case of failure and yield back.
        if resp.is_err() {
            self.storage.borrow_mut().restore(checkpoint);
        }
        resp
    }
}

impl<'a, S: ReadonlyKV, A: AccountsCodeStorage> Invoker<'a, S, A> {
    fn new(gas_limit: u64, storage: Checkpoint<'a, S>, account_code_storage: &'a mut A) -> Self {
        Self {
            gas_limit,
            gas_used: Rc::new(RefCell::new(0)),
            storage: Rc::new(RefCell::new(storage)),
            account_codes_storage: Rc::new(RefCell::new(account_code_storage)),
        }
    }

    fn handle_system_exec(
        &mut self,
        from: AccountId,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        match request.function() {
            RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER => {
                let req = CreateAccountRequest::try_from(request)?;
                let resp = self
                    .create_account(from, &req.code_id, req.init_message)
                    .map(|res| CreateAccountResponse {
                        new_account_id: res.0,
                        init_response: res.1.into_message(),
                    })?;
                InvokeResponse::try_from(resp)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_storage_exec(
        &mut self,
        from: AccountId,
        data: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let storage_set = StorageSetRequest::try_from(data)?;

        let mut key = from.as_bytes();
        key.extend(storage_set.key);

        self.storage.borrow_mut().set(&key, storage_set.value)?;

        InvokeResponse::try_from(EmptyResponse {})
    }

    fn handle_storage_query(
        &mut self,
        from: AccountId,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let storage_get = StorageGetRequest::try_from(request)?;

        let mut key = from.as_bytes();
        key.extend(storage_get.key);

        InvokeResponse::try_from(StorageGetResponse {
            value: self.storage.borrow().get(&key)?,
        })
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
        f: impl FnOnce(&dyn AccountCode) -> R,
    ) -> SdkResult<R> {
        let code_id = self
            .get_account_code_identifier_for_account(account)?
            .unwrap(); // TODO

        let storage_borrow = self.account_codes_storage.borrow();

        // 3. Get a reference out of the storage
        storage_borrow.with_code(&code_id, |code| f(code.unwrap())) // todo remove unwrap
    }
    fn get_account_code_identifier_for_account(
        &self,
        account: AccountId,
    ) -> SdkResult<Option<String>> {
        let key = Self::get_account_code_identifier_for_account_key(account);
        let code_id = self.storage.borrow().get(&key)?;

        Ok(code_id.map(|e| String::from_utf8(e).unwrap())) // TODO
    }

    fn set_account_code_identifier_for_account(
        &mut self,
        account: AccountId,
        account_identifier: &str,
    ) -> SdkResult<()> {
        println!("setting: {:?} {:?}", account, account_identifier);
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

    fn next_account_number(&mut self) -> SdkResult<AccountId> {
        let key = vec![ACCOUNT_IDENTIFIER_SINGLETON_PREFIX];

        // get last
        let last = self
            .storage
            .borrow()
            .get(&key)?
            .map(|bytes| AccountId::decode(&bytes))
            .unwrap_or(Ok(AccountId::new(u16::MAX)))?;

        // set next
        self.storage
            .borrow_mut()
            .set(&key, last.increase().encode()?)?;

        Ok(last)
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
