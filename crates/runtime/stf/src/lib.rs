mod checkpoint;
#[cfg(test)]
mod example_test;
#[cfg(test)]
mod mocks;
#[cfg(test)]
mod test_all;

use crate::checkpoint::ExecutionState;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::events_api::{
    EmitEventRequest, EmitEventResponse, Event, EVENT_HANDLER_ACCOUNT_ID,
};
use evolve_core::runtime_api::{
    CreateAccountRequest, CreateAccountResponse, MigrateRequest, ACCOUNT_IDENTIFIER_PREFIX,
    ACCOUNT_IDENTIFIER_SINGLETON_PREFIX, RUNTIME_ACCOUNT_ID,
};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageSetRequest,
    StorageSetResponse, STORAGE_ACCOUNT_ID,
};

use evolve_core::{
    AccountCode, AccountId, Environment, FungibleAsset, InvokableMessage, InvokeRequest,
    InvokeResponse, Message, ReadonlyKV, SdkResult, ERR_UNAUTHORIZED, ERR_UNKNOWN_FUNCTION,
};
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, StateChange, Transaction,
    TxValidator as TxValidatorTrait,
};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

pub struct TxResult {
    pub events: Vec<Event>,
    pub gas_used: u64,
    pub response: SdkResult<InvokeResponse>,
}

pub struct BlockResult {
    pub begin_block_events: Vec<Event>,
    pub state_changes: Vec<StateChange>,
    pub tx_results: Vec<TxResult>,
    pub end_block_events: Vec<Event>,
}

pub struct Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>(
    PhantomData<(Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx)>,
);

impl<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
    Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    BeginBlocker: BeginBlockerTrait<Block>,
    TxValidator: TxValidatorTrait<Tx>,
    EndBlocker: EndBlockerTrait,
    PostTx: PostTxExecution<Tx>,
{
    /// Allows the given closure to execute as the runtime.
    pub fn sudo<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        storage: &'a S,
        account_codes: &mut A,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, Vec<StateChange>)> {
        let mut invoker = Invoker::new_for_exec(
            RUNTIME_ACCOUNT_ID,
            0,
            ExecutionState::new(storage),
            account_codes,
        );

        let resp = action(&mut invoker)?;

        Ok((resp, invoker.into_state_changes()))
    }
    pub fn apply_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        storage: &'a S,
        account_codes: &mut A,
        block: &Block,
    ) -> BlockResult {
        // create invoker
        let mut invoker = Invoker::new_for_exec(
            RUNTIME_ACCOUNT_ID,
            0,
            ExecutionState::new(storage),
            account_codes,
        );

        // run begin blocker.
        BeginBlocker::begin_block(block, &mut invoker);
        let begin_block_events = invoker.storage.borrow_mut().pop_events();

        // apply txs.
        let txs = block.txs();
        let mut tx_results = Vec::with_capacity(txs.len());
        for tx in txs {
            // apply tx
            let tx_result = Self::apply_tx(&mut invoker, tx);
            tx_results.push(tx_result);
        }

        // run end blocker
        EndBlocker::end_block(&mut invoker);

        let end_block_events = invoker.storage.borrow_mut().pop_events();
        let state_changes = invoker.into_state_changes();

        BlockResult {
            begin_block_events,
            state_changes,
            tx_results,
            end_block_events,
        }
    }

    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        invoker: &mut Invoker<'a, S, A>,
        tx: &Tx,
    ) -> TxResult {
        // do tx validation; we do not swap invoker
        match TxValidator::validate_tx(tx, invoker) {
            Ok(()) => (),
            Err(err) => {
                return TxResult {
                    events: invoker.storage.borrow_mut().pop_events(),
                    gas_used: invoker.gas_used.borrow().clone(),
                    response: Err(err),
                }
            }
        }

        // exec tx
        let mut invoker = invoker.branch_for_new_exec(tx.sender());

        let response = invoker.do_exec(
            tx.recipient(),
            tx.request(),
            tx.funds().iter().cloned().collect(),
        );

        // do post tx validation
        let gas_used = invoker.gas_used.borrow().clone();
        let events = invoker.storage.borrow_mut().pop_events();

        // TODO: post tx executor. It is more nuanced.
        TxResult {
            events,
            gas_used,
            response,
        }
    }

    pub(crate) fn create_account<'a, S: ReadonlyKV, A: AccountsCodeStorage + 'a, M: Encodable>(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        code_id: String,
        init_message: M,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(CreateAccountResponse, ExecutionState<'a, S>)> {
        let init_message = Message::new(&init_message)?;
        let req = CreateAccountRequest {
            code_id,
            init_message,
        };

        let (resp, state) = Self::exec(
            storage,
            account_storage,
            from,
            RUNTIME_ACCOUNT_ID,
            &req,
            funds,
        )?;

        Ok((resp.get::<CreateAccountResponse>()?, state))
    }

    pub(crate) fn exec<'a, S: ReadonlyKV, A: AccountsCodeStorage + 'a, R: InvokableMessage>(
        storage: &'a S,
        account_storage: &'a mut A,
        from: AccountId,
        to: AccountId,
        req: &R,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(InvokeResponse, ExecutionState<'a, S>)> {
        let req = InvokeRequest::new(req)?;
        let writable_storage = ExecutionState::new(storage);
        let mut invoker = Invoker::new_for_exec(from, 0, writable_storage, account_storage);
        let resp = invoker.do_exec(to, &req, funds);
        resp.map(|resp| {
            (
                resp,
                Rc::try_unwrap(invoker.storage)
                    .unwrap_or_else(|_i| panic!("expected unwrap"))
                    .into_inner(),
            )
        })
    }

    pub(crate) fn query<
        'a,
        S: ReadonlyKV + 'a,
        A: AccountsCodeStorage + 'a,
        R: InvokableMessage,
    >(
        storage: &'a S,
        account_storage: &'a mut A,
        to: AccountId,
        req: &R,
    ) -> SdkResult<InvokeResponse> {
        let writable_storage = ExecutionState::new(storage);
        let invoker = Invoker::new_for_query(0, writable_storage, account_storage);
        invoker.do_query(to, &InvokeRequest::new(req)?)
    }
}

struct Invoker<'a, S, A> {
    whoami: AccountId,
    sender: AccountId,

    gas_limit: u64,

    funds: Vec<FungibleAsset>,

    gas_used: Rc<RefCell<u64>>,
    account_codes: Rc<RefCell<&'a mut A>>,
    storage: Rc<RefCell<ExecutionState<'a, S>>>,
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

    fn do_query(&self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
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
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        self.handle_transfers(to, funds.as_ref())?;

        let checkpoint = self.storage.borrow().checkpoint();

        let resp = match to {
            // check if system
            RUNTIME_ACCOUNT_ID => self.handle_system_exec(data, funds),
            // check if storage
            STORAGE_ACCOUNT_ID => self.handle_storage_exec(data),
            EVENT_HANDLER_ACCOUNT_ID => self.handle_event_handler_exec(data),
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
        storage: ExecutionState<'a, S>,
        account_code_storage: &'a mut A,
    ) -> Self {
        Self::new_for_exec(RUNTIME_ACCOUNT_ID, gas_limit, storage, account_code_storage)
    }
    fn new_for_exec(
        exec_source: AccountId,
        gas_limit: u64,
        storage: ExecutionState<'a, S>,
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
        request: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        match request.function() {
            CreateAccountRequest::FUNCTION_IDENTIFIER => {
                let req: CreateAccountRequest = request.get()?;

                let (new_account_id, init_response) =
                    self.create_account(&req.code_id, req.init_message, funds)?;

                let resp = CreateAccountResponse {
                    new_account_id,
                    init_response,
                };
                Ok(InvokeResponse::new(&resp)?)
            }
            MigrateRequest::FUNCTION_IDENTIFIER => {
                // exec on behalf of runtime the migration request, runtime has the money
                // so runtime needs to send the money to the account, so here we simulate that
                // we branch exec to runtime account ID, which is the one receiving the money
                let mut invoker = self.branch_exec(RUNTIME_ACCOUNT_ID, vec![]);

                let req: MigrateRequest = request.get()?;
                // TODO: enhance with more migration delegation rights
                if req.account_id != invoker.sender {
                    return Err(ERR_UNAUTHORIZED);
                }

                // set new account id
                self.set_account_code_identifier_for_account(req.account_id, &req.new_code_id)?;
                // make runtime invoke the exec msg
                invoker.do_exec(req.account_id, &req.execute_message, funds)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_system_query(&self, _request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        Err(ERR_UNKNOWN_FUNCTION)
    }

    fn handle_storage_exec(&mut self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            StorageSetRequest::FUNCTION_IDENTIFIER => {
                let storage_set: StorageSetRequest = request.get()?;

                let mut key = self.whoami.as_bytes();
                key.extend(storage_set.key);

                self.storage.borrow_mut().set(&key, storage_set.value)?;

                Ok(InvokeResponse::new(&StorageSetResponse {})?)
            }
            StorageRemoveRequest::FUNCTION_IDENTIFIER => {
                todo!("impl")
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_event_handler_exec(&mut self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            EmitEventRequest::FUNCTION_IDENTIFIER => {
                let req: EmitEventRequest = request.get()?;
                let event = Event {
                    source: self.whoami,
                    name: req.name,
                    contents: req.contents,
                };
                self.storage.borrow_mut().emit_event(event);
                Ok(InvokeResponse::new(&EmitEventResponse {})?)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_storage_query(&self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            StorageGetRequest::FUNCTION_IDENTIFIER => {
                let storage_get: StorageGetRequest = request.get()?;

                let mut key = storage_get.account_id.as_bytes();
                key.extend(storage_get.key);

                let value = self.storage.borrow().get(&key)?;

                Ok(InvokeResponse::new(&StorageGetResponse { value })?)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_transfers(&mut self, to: AccountId, funds: &[FungibleAsset]) -> SdkResult<()> {
        for asset in funds {
            // create transfer message
            let msg = evolve_fungible_asset::TransferMsg {
                to,
                amount: asset.amount,
            };

            // execute transfer
            self.do_exec(asset.asset_id, &InvokeRequest::new(&msg)?, vec![])?;
        }
        Ok(())
    }

    fn create_account(
        &mut self,
        code_id: &str,
        msg: Message,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(AccountId, Message)> {
        // get new account and associate it with new code ID
        let new_account_id = self.next_account_number()?;
        self.set_account_code_identifier_for_account(new_account_id, code_id)?;
        // prepare request params
        let req = InvokeRequest::new_from_message(0, msg);
        let mut invoker = self.branch_exec(new_account_id, funds);
        // do account init
        let init_resp =
            self.with_account(new_account_id, |code| code.init(&mut invoker, &req))??;

        Ok((new_account_id, init_resp.into_inner()))
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
            .unwrap_or(Ok(AccountId::new(u16::MAX.into())))?;

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

    /// TODO: change the name, but this is invoked when u executed another action
    fn branch_for_new_exec(&self, sender: AccountId) -> Self {
        Self {
            whoami: AccountId::new(u128::MAX),
            sender,
            gas_limit: self.gas_limit,
            funds: vec![],
            gas_used: self.gas_used.clone(),
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
        }
    }

    fn into_state_changes(self) -> Vec<StateChange> {
        Rc::try_unwrap(self.storage)
            .unwrap_or_else(|_err| panic!("expected unwrap"))
            .into_inner()
            .into_changes()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
