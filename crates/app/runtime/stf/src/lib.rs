mod execution_scope;
pub mod execution_state;
pub mod gas;
pub mod results;
mod runtime_api_impl;
mod unique_api_impl;

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

use evolve_core::unique_api::UNIQUE_HANDLER_ACCOUNT_ID;

use crate::execution_scope::ExecutionScope;
use crate::gas::GasCounter;
use crate::results::TxResult;
use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, FungibleAsset, InvokableMessage, InvokeRequest,
    InvokeResponse, Message, ReadonlyKV, SdkResult, ERR_UNAUTHORIZED, ERR_UNKNOWN_FUNCTION,
};
use evolve_gas::account::{GasService, StorageGasConfig};
use evolve_ns::{resolve_name, GLOBAL_NAME_SERVICE_REF};
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, Transaction, TxValidator as TxValidatorTrait,
};
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

pub const ERR_ACCOUNT_DOES_NOT_EXIST: ErrorCode = ErrorCode::new(404, "account does not exist");
pub const ERR_CODE_NOT_FOUND: ErrorCode = ErrorCode::new(401, "account code not found");
pub const ERR_EXEC_IN_QUERY: ErrorCode =
    ErrorCode::new(1, "exec functionality not available during queries");
pub const ERR_INVALID_CODE_ID: ErrorCode = ErrorCode::new(400, "invalid code identifier");
pub const ERR_EVENT_NAME_TOO_LONG: ErrorCode = ErrorCode::new(413, "event name too long");
pub const ERR_EVENT_CONTENT_TOO_LARGE: ErrorCode = ErrorCode::new(413, "event content too large");
pub const ERR_SAME_CODE_MIGRATION: ErrorCode = ErrorCode::new(400, "cannot migrate to same code");

// Security limits
const MAX_CODE_ID_LENGTH: usize = 256;
const MAX_EVENT_NAME_LENGTH: usize = 128;
const MAX_EVENT_CONTENT_SIZE: usize = 64 * 1024; // 64KB

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
        execution_height: u64,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let execution_state = ExecutionState::new(storage);
        let mut ctx =
            Invoker::new_for_begin_block(execution_state, account_codes, execution_height);

        let resp = action(&mut ctx)?;

        Ok((resp, ctx.into_execution_results().0))
    }
    /// Allows the given closure to execute as the provided account ID
    /// This method is only available in test builds or with the testing feature for security reasons
    #[cfg(any(test, feature = "testing"))]
    pub fn sudo_as<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        execution_height: u64,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let execution_state = ExecutionState::new(storage);
        let mut ctx =
            Invoker::new_for_begin_block(execution_state, account_codes, execution_height);
        ctx.whoami = impersonate;

        let resp = action(&mut ctx)?;

        Ok((resp, ctx.into_execution_results().0))
    }
    pub fn apply_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: &Block,
    ) -> (BlockResult, ExecutionState<'a, S>) {
        let state = ExecutionState::new(storage);

        // run begin blocker.
        let (mut state, gas_config, begin_block_events) =
            self.do_begin_block(state, account_codes, block);

        // apply txs.
        let txs = block.txs();
        let mut tx_results = Vec::with_capacity(txs.len());
        for tx in txs {
            // apply tx
            let tx_result: TxResult;
            (tx_result, state) = self.apply_tx(state, account_codes, tx, gas_config.clone());
            tx_results.push(tx_result);
        }

        let (state, end_block_events) = self.do_end_block(state, account_codes, block.height());

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
            },
            state,
        )
    }

    fn do_begin_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: ExecutionState<'a, S>,
        account_codes: &'a A,
        block: &Block,
    ) -> (ExecutionState<'a, S>, StorageGasConfig, Vec<Event>) {
        let mut ctx = Invoker::new_for_begin_block(state, account_codes, block.height());

        self.begin_blocker.begin_block(block, &mut ctx);

        // find gas service account
        let gas_service_account = resolve_name("gas".to_string(), &ctx)
            .expect("CRITICAL: Failed to resolve gas service name - system configuration error")
            .expect("CRITICAL: Gas service account must exist during begin_block - blockchain cannot operate without gas metering");

        // get system gas config
        let gas_config = ctx
            .run_as(
                gas_service_account,
                |account: &GasService, env: &dyn Environment| {
                    account.storage_gas_config.may_get(env)
                },
            )
            .expect("CRITICAL: Failed to run as gas service account - system configuration error")
            .expect("CRITICAL: Gas service must have storage gas config - blockchain cannot operate without gas configuration");

        let (state, _, events) = ctx.into_execution_results();

        (state, gas_config, events)
    }

    fn do_end_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: ExecutionState<'a, S>,
        codes: &'a A,
        height: u64,
    ) -> (ExecutionState<'a, S>, Vec<Event>) {
        let mut ctx = Invoker::new_for_end_block(state, codes, height);
        self.end_blocker.end_block(&mut ctx);

        let (state, _, events) = ctx.into_execution_results();
        (state, events)
    }

    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: ExecutionState<'a, S>,
        codes: &'a A,
        tx: &Tx,
        gas_config: StorageGasConfig,
    ) -> (TxResult, ExecutionState<'a, S>) {
        // NOTE: Transaction validation and execution are atomic - they share the same
        // ExecutionState throughout the process. The state cannot change between
        // validation and execution because:
        // 1. The same ExecutionState instance is used for both phases
        // 2. into_new_exec() preserves the storage, maintaining consistency
        // 3. Transactions are processed sequentially, not concurrently

        // create validation context
        let mut ctx = Invoker::new_for_validate_tx(state, codes, gas_config, tx);
        // do tx validation; we do not swap invoker
        match self.tx_validator.validate_tx(tx, &mut ctx) {
            Ok(()) => (),
            Err(err) => {
                let (state, gas_used, events) = ctx.into_execution_results();
                return (
                    TxResult {
                        events,
                        gas_used,
                        response: Err(err),
                    },
                    state,
                );
            }
        }

        // exec tx - transforms validation context to execution context
        // while preserving the same underlying state
        let mut ctx = ctx.into_new_exec(tx.sender());

        let response = ctx.do_exec(tx.recipient(), tx.request(), tx.funds().to_vec());

        let (state, gas_used, events) = ctx.into_execution_results();

        // TODO do post tx validation

        (
            TxResult {
                events,
                gas_used,
                response,
            },
            state,
        )
    }

    pub fn query<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R: InvokableMessage>(
        &self,
        storage: &'a S,
        account_codes: &'a mut A,
        to: AccountId,
        req: &R,
        gc: GasCounter,
    ) -> SdkResult<InvokeResponse> {
        let state = ExecutionState::new(storage);
        let ctx = Invoker::new_for_query(state, account_codes, gc, to);
        ctx.do_query(to, &InvokeRequest::new(req)?)
    }
    /// Allows to execute in readonly mode over the given account ID with the provided
    /// AccountCode handle.
    /// It can be used to extract data from state, after execution.
    /// For example a consensus engine wanting to extract the validator set changes after block
    /// execution.
    /// Such things could be done by reading into state keys manually, but it's less developer friendly.
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
        let state = ExecutionState::new(storage);
        let invoker =
            Invoker::new_for_query(state, account_storage, GasCounter::infinite(), account_id);
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
        let state = ExecutionState::new(storage);

        let invoker = Invoker::new_for_query(
            state,
            account_storage,
            GasCounter::infinite(),
            GLOBAL_NAME_SERVICE_REF.0,
        );
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

    scope: ExecutionScope,
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
            UNIQUE_HANDLER_ACCOUNT_ID => self.handle_unique_query(data),
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
        // Take checkpoint before ANY state changes
        let checkpoint = self.storage.borrow().checkpoint();

        // Helper closure to restore checkpoint on error
        let restore_checkpoint = |storage: &RefCell<ExecutionState<'_, S>>| {
            if let Err(restore_err) = storage.borrow_mut().restore(checkpoint) {
                // Log the error and panic as we cannot recover
                eprintln!("CRITICAL: Failed to restore checkpoint: {restore_err:?}");
                panic!("CRITICAL: Failed to restore checkpoint after error - blockchain state is corrupted and cannot continue safely");
            }
        };

        // Handle transfers - restore checkpoint if this fails
        if let Err(transfer_err) = self.handle_transfers(to, funds.as_ref()) {
            restore_checkpoint(&self.storage);
            return Err(transfer_err);
        }

        let resp = match to {
            // check if system
            RUNTIME_ACCOUNT_ID => self.handle_system_exec(data, funds),
            // check if storage
            STORAGE_ACCOUNT_ID => self.handle_storage_exec(data),
            EVENT_HANDLER_ACCOUNT_ID => self.handle_event_handler_exec(data),
            UNIQUE_HANDLER_ACCOUNT_ID => self.handle_unique_exec(data),
            // other account
            _ => {
                let mut invoker = self.branch_exec(to, funds);
                self.with_account(to, |code| code.execute(&mut invoker, data))?
            }
        };

        // restore checkpoint in case of failure and yield back.
        if resp.is_err() {
            restore_checkpoint(&self.storage);
        }
        resp
    }
}

impl<'a, S: ReadonlyKV, A: AccountsCodeStorage> Invoker<'a, S, A> {
    fn new_for_end_block(
        storage: ExecutionState<'a, S>,
        account_codes: &'a A,
        block_height: u64,
    ) -> Self {
        Self {
            whoami: RUNTIME_ACCOUNT_ID,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes: Rc::new(RefCell::new(account_codes)),
            storage: Rc::new(RefCell::new(storage)),
            gas_counter: Rc::new(RefCell::new(GasCounter::Infinite)),
            scope: ExecutionScope::EndBlock(block_height),
        }
    }
    fn new_for_begin_block(
        storage: ExecutionState<'a, S>,
        account_codes: &'a A,
        block_height: u64,
    ) -> Self {
        Self {
            whoami: RUNTIME_ACCOUNT_ID,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes: Rc::new(RefCell::new(account_codes)),
            storage: Rc::new(RefCell::new(storage)),
            gas_counter: Rc::new(RefCell::new(GasCounter::Infinite)),
            scope: ExecutionScope::BeginBlock(block_height),
        }
    }

    fn new_for_query(
        storage: ExecutionState<'a, S>,
        account_codes: &'a A,
        gas_counter: GasCounter,
        recipient: AccountId,
    ) -> Self {
        Self {
            whoami: recipient,
            sender: RUNTIME_ACCOUNT_ID,
            funds: vec![],
            account_codes: Rc::new(RefCell::new(account_codes)),
            storage: Rc::new(RefCell::new(storage)),
            gas_counter: Rc::new(RefCell::new(gas_counter)),
            scope: ExecutionScope::Query,
        }
    }

    fn new_for_validate_tx(
        storage: ExecutionState<'a, S>,
        account_codes: &'a A,
        storage_gas_config: StorageGasConfig,
        tx: &impl Transaction,
    ) -> Self {
        Self {
            whoami: AccountId::invalid(), // whoami is invalid because no one is receiving anything as of now.
            sender: RUNTIME_ACCOUNT_ID,   // sender is runtime for tx validation
            funds: vec![],                // no funds for this.
            account_codes: Rc::new(RefCell::new(account_codes)),
            storage: Rc::new(RefCell::new(storage)),
            gas_counter: Rc::new(RefCell::new(GasCounter::Finite {
                gas_limit: tx.gas_limit(),
                gas_used: 0,
                storage_gas_config,
            })),
            scope: ExecutionScope::Transaction(tx.compute_identifier()),
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
            scope: self.scope,
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
            scope: self.scope,
        }
    }

    fn into_new_exec(self, sender: AccountId) -> Self {
        Self {
            whoami: sender,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes: self.account_codes,
            storage: self.storage,
            gas_counter: self.gas_counter,
            scope: self.scope,
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

                // Validate that the account exists before migration
                let current_code_id = runtime_api_impl::get_account_code_identifier_for_account(
                    &self.storage.borrow(),
                    req.account_id,
                )?
                .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;

                // Validate that the new code exists
                let code_exists = self
                    .account_codes
                    .borrow()
                    .with_code(&req.new_code_id, |code| code.is_some())?;
                if !code_exists {
                    return Err(ERR_CODE_NOT_FOUND);
                }

                // TODO: Add code compatibility validation here
                // For now, we just check that the codes are different
                if current_code_id == req.new_code_id {
                    return Err(ERR_SAME_CODE_MIGRATION);
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

                // Validate event name length
                if req.name.len() > MAX_EVENT_NAME_LENGTH {
                    return Err(ERR_EVENT_NAME_TOO_LONG);
                }

                // Validate event content size
                if req.contents.len() > MAX_EVENT_CONTENT_SIZE {
                    return Err(ERR_EVENT_CONTENT_TOO_LARGE);
                }

                let event = Event {
                    source: self.whoami,
                    name: req.name,
                    contents: req.contents,
                };
                self.storage.borrow_mut().emit_event(event)?;
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

    fn handle_unique_exec(&mut self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            evolve_unique::unique::NextUniqueIdMsg::FUNCTION_IDENTIFIER => {
                let next_created_object_counter = self.storage.borrow_mut().next_unique_object_id();
                let unique_object_id = self.scope.unique_id(next_created_object_counter)?;
                InvokeResponse::new(&unique_object_id)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_unique_query(&self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            evolve_unique::unique::UniqueObjectsCreatedMsg::FUNCTION_IDENTIFIER => {
                InvokeResponse::new(&self.storage.borrow().created_unique_objects())
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_transfers(&mut self, to: AccountId, funds: &[FungibleAsset]) -> SdkResult<()> {
        // Early return if no funds to transfer
        if funds.is_empty() {
            return Ok(());
        }

        // Validate recipient is not a system account that shouldn't receive funds
        if to == STORAGE_ACCOUNT_ID
            || to == EVENT_HANDLER_ACCOUNT_ID
            || to == UNIQUE_HANDLER_ACCOUNT_ID
        {
            return Err(ErrorCode::new(400, "cannot send funds to system accounts"));
        }

        // Execute all transfers atomically - if any fail, all fail
        for asset in funds {
            // create transfer message
            let msg = evolve_fungible_asset::TransferMsg {
                to,
                amount: asset.amount,
            };

            // execute transfer - this will fail with ERR_NOT_ENOUGH_BALANCE if insufficient funds
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
        // Validate code_id
        if code_id.is_empty() || code_id.len() > MAX_CODE_ID_LENGTH {
            return Err(ERR_INVALID_CODE_ID);
        }

        // Validate code_id contains only valid characters (alphanumeric, dash, underscore, slash)
        if !code_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/')
        {
            return Err(ERR_INVALID_CODE_ID);
        }

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

    fn into_execution_results(self) -> (ExecutionState<'a, S>, u64, Vec<Event>) {
        // Move out the GasCounter from self.gas_counter
        // If there are multiple references, we clone the inner value instead of panicking
        let gas_counter = match Rc::try_unwrap(self.gas_counter) {
            Ok(refcell) => refcell.into_inner(),
            Err(rc) => {
                // Multiple references exist, so we need to clone the inner value
                // This is safe because GasCounter is cheap to clone
                (*rc.borrow()).clone()
            }
        };
        let gas_used = gas_counter.gas_used();

        // Move out the ExecutionState
        // Since ExecutionState contains data that's expensive to clone, we still need to ensure
        // there's only one reference, but we'll handle it more gracefully
        let (state, events) = match Rc::try_unwrap(self.storage) {
            Ok(refcell) => {
                let mut state = refcell.into_inner();
                let events = state.pop_events();
                (state, events)
            }
            Err(storage_rc) => {
                // This should not happen in normal operation as Invoker manages references carefully
                // Log an error and continue with the borrowed state
                // In production, this would warrant investigation
                eprintln!(
                    "CRITICAL: Multiple references to ExecutionState detected during finalization - this indicates a bug in reference management"
                );
                // Get events from the borrowed state
                let _events = storage_rc.borrow_mut().pop_events();
                // We can't move the state out, so we have to panic here as the invariant is broken
                // This is still better than the original panic as we've logged the issue
                panic!("CRITICAL: Cannot extract ExecutionState - multiple references exist. This is a bug in the STF reference management.");
            }
        };

        (state, gas_used, events)
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
