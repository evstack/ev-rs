pub mod execution_state;
pub mod gas;
pub mod results;
mod runtime_api_impl;

use crate::execution_state::ExecutionState;
use crate::results::BlockResult;
use evolve_core::events_api::{
    EmitEventRequest, EmitEventResponse, Event, EVENT_HANDLER_ACCOUNT_ID,
};
use evolve_core::runtime_api::{
    CreateAccountRequest, CreateAccountResponse, MigrateRequest, RUNTIME_ACCOUNT_ID,
};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageRemoveResponse,
    StorageSetRequest, StorageSetResponse, STORAGE_ACCOUNT_ID,
};

use crate::gas::GasCounter;
use crate::results::TxResult;
use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, FungibleAsset, InvokableMessage, InvokeRequest,
    InvokeResponse, Message, ReadonlyKV, SdkResult, ERR_UNAUTHORIZED, ERR_UNKNOWN_FUNCTION,
};
use evolve_gas::account::{GasService, StorageGasConfig};
use evolve_ns::resolve_name;
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, Transaction, TxValidator as TxValidatorTrait,
};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

pub const ERR_ACCOUNT_DOES_NOT_EXIST: ErrorCode = ErrorCode::new(404, "account does not exist");
pub const ERR_CODE_NOT_FOUND: ErrorCode = ErrorCode::new(401, "account code not found");

pub struct Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx> {
    begin_blocker: BeginBlocker,
    end_blocker: EndBlocker,
    tx_validator: TxValidator,
    post_tx_handler: PostTx,
    _phantoms: PhantomData<(Tx, Block)>,
}

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
    pub const fn new(
        begin_blocker: BeginBlocker,
        end_blocker: EndBlocker,
        tx_validator: TxValidator,
        post_tx_handler: PostTx,
    ) -> Self {
        Self {
            begin_blocker,
            end_blocker,
            tx_validator,
            post_tx_handler,
            _phantoms: PhantomData,
        }
    }
    /// Allows the given closure to execute as the runtime.
    pub fn sudo<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let mut invoker = Invoker::new_block_state(
            RUNTIME_ACCOUNT_ID,
            ExecutionState::new(storage),
            account_codes,
            GasCounter::infinite(),
        );

        let resp = action(&mut invoker)?;

        Ok((resp, invoker.into_execution_state()))
    }
    pub fn apply_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: &Block,
    ) -> (BlockResult, ExecutionState<'a, S>) {
        // create invoker
        let mut block_state = Invoker::new_block_state(
            RUNTIME_ACCOUNT_ID,
            ExecutionState::new(storage),
            account_codes,
            GasCounter::infinite(),
        );

        // run begin blocker.
        self.do_begin_block(block, &mut block_state);
        let begin_block_events = block_state.storage.borrow_mut().pop_events();

        // find gas service account: TODO maybe better way to do it
        let gas_service_account = resolve_name("gas".to_string(), &block_state)
            .unwrap()
            .unwrap();

        // get system gas config
        let gas_config = block_state
            .run_as(
                gas_service_account,
                |account: &GasService, env: &dyn Environment| {
                    account.storage_gas_config.may_get(env)
                },
            )
            .unwrap()
            .unwrap();

        // apply txs.
        let txs = block.txs();
        let mut tx_results = Vec::with_capacity(txs.len());
        for tx in txs {
            // apply tx
            let tx_result = self.apply_tx(&block_state, tx, gas_config.clone());
            tx_results.push(tx_result);
        }

        let end_block_events = self.do_end_block(&mut block_state);
        let new_state = block_state.into_execution_state();

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
            },
            new_state,
        )
    }

    fn do_begin_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        block: &Block,
        block_state: &mut Invoker<'a, S, A>,
    ) -> Vec<Event> {
        let mut invoker = block_state.clone_with_gas(GasCounter::infinite());
        self.begin_blocker.begin_block(block, &mut invoker);

        let events = invoker.storage.borrow_mut().pop_events();
        events
    }

    fn do_end_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        block_state: &mut Invoker<'a, S, A>,
    ) -> Vec<Event> {
        let mut invoker = block_state.clone_with_gas(GasCounter::infinite());
        self.end_blocker.end_block(&mut invoker);

        let events = invoker.storage.borrow_mut().pop_events();
        events
    }

    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        parent: &Invoker<'a, S, A>,
        tx: &Tx,
        gas_config: StorageGasConfig,
    ) -> TxResult {
        // add gas
        let mut validate_ctx =
            parent.clone_with_gas(GasCounter::finite(tx.gas_limit(), gas_config));
        // do tx validation; we do not swap invoker
        match self.tx_validator.validate_tx(tx, &mut validate_ctx) {
            Ok(()) => (),
            Err(err) => {
                return TxResult {
                    events: validate_ctx.storage.borrow_mut().pop_events(),
                    gas_used: validate_ctx.gas_counter.borrow().gas_used(),
                    response: Err(err),
                }
            }
        }

        // exec tx
        let mut exec_ctx = validate_ctx.branch_for_new_exec(tx.sender());

        let response = exec_ctx.do_exec(tx.recipient(), tx.request(), tx.funds().to_vec());

        // do post tx validation
        let gas_used = exec_ctx.gas_counter.borrow().gas_used();
        let events = exec_ctx.storage.borrow_mut().pop_events();

        // TODO: post tx executor. It is more nuanced.
        TxResult {
            events,
            gas_used,
            response,
        }
    }

    pub fn query<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R: InvokableMessage>(
        &self,
        storage: &'a S,
        account_storage: &'a mut A,
        to: AccountId,
        req: &R,
        gc: GasCounter,
    ) -> SdkResult<InvokeResponse> {
        let writable_storage = ExecutionState::new(storage);
        let invoker = Invoker::new_for_query(writable_storage, account_storage, gc);
        invoker.do_query(to, &InvokeRequest::new(req)?)
    }
    /// Allows to execute in readonly mode over the given account ID with the provided
    /// AccountCode handle.
    /// It can be used to extract data from state, after execution.
    /// For example a consensus engine wanting to extract the validator set changes after block
    /// execution.
    /// Such things could be done by reading into state keys manually but it's less developer friendly.
    pub fn run_with_code<T: AccountCode + Default, R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(&T, &dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        self.run(storage, account_storage, account_id, move |t| {
            handle(&T::default(), t)
        })
    }

    pub fn run_with_ref<T: From<AccountId>, R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(T, &dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        self.run(storage, account_storage, account_id, move |t| {
            handle(T::from(account_id), t)
        })
    }

    pub fn run<R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_id: AccountId,
        handle: impl FnOnce(&dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let exec_state = ExecutionState::new(storage);
        let invoker = Invoker::new_for_query(exec_state, account_storage, GasCounter::infinite());
        let account_id_invoker = invoker.branch_query(account_id);
        handle(&account_id_invoker)
    }

    pub fn resolve_and_run_as_ref<T: From<AccountId>, R, S: ReadonlyKV, A: AccountsCodeStorage>(
        &self,
        storage: &S,
        account_storage: &A,
        account_name: String,
        handle: impl FnOnce(T, &dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let exec_state = ExecutionState::new(storage);
        let invoker = Invoker::new_for_query(exec_state, account_storage, GasCounter::infinite());
        let account_id = resolve_name(account_name, &invoker)?.ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;
        let account_id_invoker = invoker.branch_query(account_id);
        handle(T::from(account_id), &account_id_invoker)
    }
}

struct Invoker<'a, S, A> {
    whoami: AccountId,
    sender: AccountId,

    funds: Vec<FungibleAsset>,
    account_codes: Rc<RefCell<&'a A>>,
    storage: Rc<RefCell<ExecutionState<'a, S>>>,

    gas_counter: Rc<RefCell<GasCounter>>,
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
        storage: ExecutionState<'a, S>,
        account_code_storage: &'a A,
        gas_counter: GasCounter,
    ) -> Self {
        Self::new_block_state(
            RUNTIME_ACCOUNT_ID,
            storage,
            account_code_storage,
            gas_counter,
        )
    }
    fn new_block_state(
        exec_source: AccountId,
        storage: ExecutionState<'a, S>,
        account_code_storage: &'a A,
        gas_counter: GasCounter,
    ) -> Self {
        Self {
            whoami: exec_source,
            sender: AccountId::new(u128::MAX),

            funds: vec![],
            account_codes: Rc::new(RefCell::new(account_code_storage)),
            storage: Rc::new(RefCell::new(storage)),
            gas_counter: Rc::new(RefCell::new(gas_counter)),
        }
    }

    fn branch_query(&self, whoami: AccountId) -> Self {
        Self {
            whoami,
            sender: RUNTIME_ACCOUNT_ID,
            funds: vec![],
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
            gas_counter: self.gas_counter.clone(),
        }
    }

    fn branch_exec(&self, to: AccountId, funds: Vec<FungibleAsset>) -> Self {
        Self {
            whoami: to,
            sender: self.whoami,
            funds,
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
            gas_counter: self.gas_counter.clone(),
        }
    }

    /// TODO: change the name, but this is invoked when u executed another action
    fn branch_for_new_exec(&self, sender: AccountId) -> Self {
        Self {
            whoami: sender,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
            gas_counter: self.gas_counter.clone(),
        }
    }

    fn clone_with_gas(&self, gas_counter: GasCounter) -> Self {
        Self {
            whoami: self.whoami,
            sender: self.sender,
            funds: vec![],
            account_codes: self.account_codes.clone(),
            storage: self.storage.clone(),
            gas_counter: Rc::new(RefCell::new(gas_counter)),
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
                runtime_api_impl::set_account_code_identifier_for_account(
                    &mut self.storage.borrow_mut(),
                    req.account_id,
                    &req.new_code_id,
                )?;
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

                // increase gas costs
                self.gas_counter
                    .borrow_mut()
                    .consume_set_gas(&key, &storage_set.value)?;
                self.storage.borrow_mut().set(&key, storage_set.value)?;

                Ok(InvokeResponse::new(&StorageSetResponse {})?)
            }
            StorageRemoveRequest::FUNCTION_IDENTIFIER => {
                let storage_remove: StorageRemoveRequest = request.get()?;
                let mut key = self.whoami.as_bytes();
                key.extend(storage_remove.key);
                self.gas_counter.borrow_mut().consume_remove_gas(&key)?;
                self.storage.borrow_mut().remove(&key)?;
                Ok(InvokeResponse::new(&StorageRemoveResponse {})?)
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

                self.gas_counter
                    .borrow_mut()
                    .consume_get_gas(&key, &value)?;

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
        let new_account_id = runtime_api_impl::next_account_number(&mut self.storage.borrow_mut())?;
        runtime_api_impl::set_account_code_identifier_for_account(
            &mut self.storage.borrow_mut(),
            new_account_id,
            code_id,
        )?;
        // prepare request params
        let req = InvokeRequest::new_from_message("init", 0, msg);
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
        let code_id = runtime_api_impl::get_account_code_identifier_for_account(
            &self.storage.borrow(),
            account,
        )?
        .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;

        let account_storage = self.account_codes.borrow();

        // 3. Get a reference out of the storage
        account_storage.with_code(&code_id, |code| match code {
            Some(code) => Ok(f(code)),
            None => Err(ERR_CODE_NOT_FOUND),
        })?
    }

    pub fn run_as<T: AccountCode + Default, R>(
        &self,
        account_id: AccountId,
        handle: impl FnOnce(&T, &dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let env = self.branch_query(account_id);
        let code = T::default();
        handle(&code, &env)
    }

    fn into_execution_state(self) -> ExecutionState<'a, S> {
        Rc::try_unwrap(self.storage)
            .unwrap_or_else(|_| panic!("expected unwrap"))
            .into_inner()
    }
}

impl<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx> Clone
    for Stf<Tx, Block, BeginBlocker, TxValidator, EndBlocker, PostTx>
where
    BeginBlocker: Clone,
    TxValidator: Clone,
    EndBlocker: Clone,
    PostTx: Clone,
{
    fn clone(&self) -> Self {
        Self {
            begin_blocker: self.begin_blocker.clone(),
            end_blocker: self.end_blocker.clone(),
            tx_validator: self.tx_validator.clone(),
            post_tx_handler: self.post_tx_handler.clone(),
            _phantoms: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
