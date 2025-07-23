//! # State Transition Function (STF) Module
//!
//! This module provides the core state transition functionality for the Evolve blockchain.
//! It manages transaction execution, block processing, and state management.
//!
//! ## Core Components
//!
//! - `Stf`: Main state transition function that processes blocks and transactions
//! - `Invoker`: Execution context for account operations
//! - Error handling and validation
//! - Gas metering and resource management

mod execution_scope;
pub mod execution_state;
pub mod gas;
mod handlers;
mod invoker;
pub mod results;
mod runtime_api_impl;
mod unique_api_impl;
mod validation;

use crate::execution_state::ExecutionState;
use crate::gas::GasCounter;
use crate::invoker::Invoker;
use crate::results::{BlockResult, TxResult};
use evolve_core::events_api::Event;
use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, InvokableMessage, InvokeRequest,
    InvokeResponse, ReadonlyKV, SdkResult,
};
use evolve_gas::account::{GasService, StorageGasConfig};
use evolve_ns::{resolve_name, GLOBAL_NAME_SERVICE_REF};
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, Transaction, TxValidator as TxValidatorTrait,
};
use std::marker::PhantomData;

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

/// The main State Transition Function (STF) for the Evolve blockchain.
///
/// This struct orchestrates the execution of blocks and transactions, managing
/// the state transitions in a deterministic manner. It coordinates between
/// different components like transaction validation, block processing, and
/// post-transaction handling.
///
/// # Type Parameters
///
/// * `Tx` - Transaction type that implements the `Transaction` trait
/// * `Block` - Block type that implements the `BlockTrait`
/// * `BeginBlocker` - Handler for begin-block logic
/// * `TxValidator` - Transaction validator
/// * `EndBlocker` - Handler for end-block logic
/// * `PostTx` - Post-transaction execution handler
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
    /// Creates a new STF instance with the provided components.
    ///
    /// # Arguments
    ///
    /// * `begin_blocker` - Handler for begin-block processing
    /// * `end_blocker` - Handler for end-block processing
    /// * `tx_validator` - Transaction validator
    /// * `post_tx_handler` - Post-transaction execution handler
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
    /// Executes the given closure with runtime privileges.
    ///
    /// This method provides a way to execute operations with system-level privileges,
    /// typically used for administrative tasks or system initialization.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `execution_height` - Block height for execution context
    /// * `action` - Closure to execute with runtime privileges
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the result of the action and the execution state.
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
    /// Executes the given closure impersonating the specified account.
    ///
    /// This method allows executing operations as if they were initiated by a specific account.
    /// It's only available in test builds or with the testing feature enabled for security reasons.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `execution_height` - Block height for execution context
    /// * `impersonate` - Account ID to impersonate
    /// * `action` - Closure to execute with the specified account privileges
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the result of the action and the execution state.
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
    /// Applies a complete block to the current state.
    ///
    /// This is the main entry point for block processing. It executes the complete
    /// block lifecycle: begin-block, transaction processing, and end-block.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `block` - Block to process
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the block execution results and the updated execution state.
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

    /// Executes a query against a specific account.
    ///
    /// Queries are read-only operations that don't modify state. They can be used
    /// to retrieve information from accounts without consuming gas or affecting
    /// the blockchain state.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `to` - Target account ID for the query
    /// * `req` - Query request message
    /// * `gc` - Gas counter for query execution
    ///
    /// # Returns
    ///
    /// Returns the query response from the target account.
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
    /// Executes a closure with read-only access to a specific account's code.
    ///
    /// This method provides a developer-friendly way to extract data from account state
    /// after execution. It's particularly useful for consensus engines that need to
    /// extract validator set changes or other state information after block execution.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with account code access
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
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

    /// Executes a closure with a reference-based account handle.
    ///
    /// Similar to `run_with_code`, but uses a reference type that can be constructed
    /// from the account ID. This is useful for account handles that don't need
    /// to maintain state but need access to the account ID.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with account reference
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
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

    /// Executes a closure with read-only access to an account's environment.
    ///
    /// This is the base method for executing read-only operations against accounts.
    /// It provides an environment interface that can be used to query account state
    /// and perform other read-only operations.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_id` - Target account ID
    /// * `handle` - Closure to execute with environment access
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
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

    /// Resolves an account name and executes a closure with account access.
    ///
    /// This method combines name resolution with account execution, allowing
    /// operations to be performed using human-readable account names instead
    /// of raw account IDs.
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_storage` - Account code storage
    /// * `account_name` - Human-readable account name to resolve
    /// * `handle` - Closure to execute with resolved account access
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution.
    ///
    /// # Errors
    ///
    /// Returns an error if the account name cannot be resolved.
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
