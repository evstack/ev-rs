mod checkpoint;
mod example_test;
#[cfg(test)]
mod mocks;
mod test_all;

use crate::checkpoint::Checkpoint;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::well_known::{
    CreateAccountRequest, CreateAccountResponse, EmptyResponse, StorageGetRequest,
    StorageGetResponse, StorageSetRequest, ACCOUNT_IDENTIFIER_PREFIX,
    ACCOUNT_IDENTIFIER_SINGLETON_PREFIX, INIT_FUNCTION_IDENTIFIER, RUNTIME_ACCOUNT_ID,
    RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER, STORAGE_ACCOUNT_ID,
};
use evolve_core::{
    AccountCode, AccountId, Environment, FungibleAsset, InvokeRequest, InvokeResponse, Message,
    ReadonlyKV, SdkResult, ERR_UNKNOWN_FUNCTION,
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
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(CreateAccountResponse, Checkpoint<'a, S>)> {
        let req = InvokeRequest::try_from(CreateAccountRequest {
            code_id,
            init_message,
        })?;

        let resp = Self::exec(
            storage,
            account_storage,
            from,
            RUNTIME_ACCOUNT_ID,
            req,
            funds,
        )?;
        Ok((CreateAccountResponse::try_from(resp.0)?, resp.1))
    }

    pub(crate) fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage + 'a>(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        to: AccountId,
        req: InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(InvokeResponse, Checkpoint<'a, S>)> {
        let writable_storage = Checkpoint::new(storage);
        let mut invoker = Invoker::new_for_exec(from, 0, writable_storage, account_storage);
        let resp = invoker.do_exec(to, req, funds);
        resp.map(|resp| {
            (
                resp,
                Rc::try_unwrap(invoker.storage)
                    .unwrap_or_else(|e| panic!("expected unwrap"))
                    .into_inner(),
            )
        })
    }

    pub(crate) fn query<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        storage: &'a S,
        account_storage: &'a mut A,
        to: AccountId,
        req: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let writable_storage = Checkpoint::new(storage);
        let mut invoker = Invoker::new_for_query(0, writable_storage, account_storage);
        invoker.do_query(to, req)
    }
}

struct Invoker<'a, S, A> {
    whoami: AccountId,
    sender: AccountId,

    gas_limit: u64,

    funds: Vec<FungibleAsset>,

    gas_used: Rc<RefCell<u64>>,
    account_codes: Rc<RefCell<&'a mut A>>,
    storage: Rc<RefCell<Checkpoint<'a, S>>>,
}

impl<S: ReadonlyKV, A: AccountsCodeStorage> Environment for Invoker<'_, S, A> {
    fn whoami(&self) -> AccountId {
        self.whoami
    }

    fn sender(&self) -> AccountId {
        self.sender
    }

    fn funds(&self) -> &[FungibleAsset] {
        &self.funds
    }

    fn do_query(&self, to: AccountId, data: InvokeRequest) -> SdkResult<InvokeResponse> {
        match to {
            RUNTIME_ACCOUNT_ID => self.handle_system_query(data),
            STORAGE_ACCOUNT_ID => self.handle_storage_query(data),
            _ => {
                let invoker = self.branch_query(to);
                self.with_account(to, |code| code.query(&invoker, data))?
            }
        }
    }

    fn do_exec(
        &mut self,
        to: AccountId,
        data: InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        self.handle_transfers(to, funds.as_ref())?;

        let checkpoint = self.storage.borrow().checkpoint();

        let resp = match to {
            // check if system
            RUNTIME_ACCOUNT_ID => self.handle_system_exec(data, funds),
            // check if storage
            STORAGE_ACCOUNT_ID => self.handle_storage_exec(data),
            // other account
            _ => {
                let mut invoker = self.branch_exec(to, funds);
                self.with_account(to, |code| code.execute(&mut invoker, data))?
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
    fn new_for_query(
        gas_limit: u64,
        storage: Checkpoint<'a, S>,
        account_code_storage: &'a mut A,
    ) -> Self {
        Self::new_for_exec(RUNTIME_ACCOUNT_ID, gas_limit, storage, account_code_storage)
    }
    fn new_for_exec(
        exec_source: AccountId,
        gas_limit: u64,
        storage: Checkpoint<'a, S>,
        account_code_storage: &'a mut A,
    ) -> Self {
        Self {
            whoami: exec_source,
            sender: AccountId::new(u128::MAX),
            gas_limit,

            funds: vec![],
            gas_used: Rc::new(RefCell::new(0)),
            account_codes: Rc::new(RefCell::new(account_code_storage)),
            storage: Rc::new(RefCell::new(storage)),
        }
    }

    fn handle_system_exec(
        &mut self,
        request: InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        match request.function() {
            RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER => {
                let req = CreateAccountRequest::try_from(request)?;
                let resp = self
                    .create_account(&req.code_id, req.init_message, funds)
                    .map(|res| CreateAccountResponse {
                        new_account_id: res.0,
                        init_response: res.1.into_message(),
                    })?;
                InvokeResponse::try_from(resp)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_system_query(&self, request: InvokeRequest) -> SdkResult<InvokeResponse> {
        Err(ERR_UNKNOWN_FUNCTION)
    }

    fn handle_storage_exec(&mut self, data: InvokeRequest) -> SdkResult<InvokeResponse> {
        // TODO: manage all
        let storage_set = StorageSetRequest::try_from(data)?;

        let mut key = self.whoami.as_bytes();
        key.extend(storage_set.key);

        self.storage.borrow_mut().set(&key, storage_set.value)?;

        InvokeResponse::try_from(EmptyResponse {})
    }

    fn handle_storage_query(&self, request: InvokeRequest) -> SdkResult<InvokeResponse> {
        let storage_get = StorageGetRequest::try_from(request)?;

        let mut key = storage_get.account_id.as_bytes();
        key.extend(storage_get.key);

        InvokeResponse::try_from(StorageGetResponse {
            value: self.storage.borrow().get(&key)?,
        })
    }

    fn handle_transfers(&mut self, to: AccountId, funds: &[FungibleAsset]) -> SdkResult<()> {
        for asset in funds {
            // create transfer message
            let msg = evolve_fungible_asset::TransferMsg {
                to,
                amount: asset.amount,
            };

            // execute transfer
            self.do_exec(
                asset.asset_id,
                InvokeRequest::new(
                    evolve_fungible_asset::TransferMsg::FUNCTION_IDENTIFIER,
                    Message::from(msg.encode()?),
                ),
                vec![],
            )?;
        }
        Ok(())
    }

    fn create_account(
        &mut self,
        code_id: &str,
        msg: impl Into<Message>,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(AccountId, InvokeResponse)> {
        // get new account and associate it with new code ID
        let new_account_id = self.next_account_number()?;
        self.set_account_code_identifier_for_account(new_account_id, code_id)?;
        // prepare request params
        let req = InvokeRequest::new(INIT_FUNCTION_IDENTIFIER, msg.into());
        let mut invoker = self.branch_exec(new_account_id, funds);
        // do account init
        self.with_account(new_account_id, |code| code.init(&mut invoker, req))?
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

        let account_storage = self.account_codes.borrow();

        // 3. Get a reference out of the storage
        account_storage.with_code(&code_id, |code| f(code.unwrap())) // todo remove unwrap
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
        let mut storage = self.storage.borrow_mut();
        let last = storage
            .get(&key)?
            .map(|bytes| AccountId::decode(&bytes))
            .unwrap_or(Ok(AccountId::new(u16::MAX)))?;

        // set next
        storage.set(&key, last.increase().encode()?)?;
        Ok(last)
    }

    fn branch_query(&self, whoami: AccountId) -> Self {
        Self {
            whoami,
            sender: RUNTIME_ACCOUNT_ID,
            gas_limit: self.gas_limit,
            funds: vec![],
            gas_used: self.gas_used.clone(),
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
        }
    }

    fn branch_exec(&self, to: AccountId, funds: Vec<FungibleAsset>) -> Self {
        Self {
            whoami: to,
            sender: self.whoami,
            gas_limit: self.gas_limit,
            funds,
            gas_used: self.gas_used.clone(),
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
