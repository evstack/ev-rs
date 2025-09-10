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

pub mod errors;
mod execution_scope;
pub mod execution_state;
pub mod gas;
mod handlers;
mod invoker;
pub mod metrics;
pub mod results;
mod runtime_api_impl;
mod unique_api_impl;
mod validation;

use crate::execution_state::ExecutionState;
use crate::gas::GasCounter;
use crate::invoker::Invoker;
use crate::metrics::{BlockExecutionMetrics, TxExecutionMetrics};
use crate::results::{BlockResult, TxResult};
use evolve_core::events_api::Event;
use evolve_core::{
    AccountCode, AccountId, Environment, EnvironmentQuery, InvokableMessage, InvokeRequest,
    InvokeResponse, ReadonlyKV, SdkResult,
};
use evolve_gas::account::{GasService, StorageGasConfig};
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker as BeginBlockerTrait, Block as BlockTrait,
    EndBlocker as EndBlockerTrait, PostTxExecution, Transaction, TxValidator as TxValidatorTrait,
};
use std::marker::PhantomData;
use std::time::Instant;

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
    system_accounts: SystemAccounts,
    _phantoms: PhantomData<(Tx, Block)>,
}

#[derive(Clone, Copy, Debug)]
pub struct SystemAccounts {
    pub gas_service: AccountId,
}

impl SystemAccounts {
    pub const fn new(gas_service: AccountId) -> Self {
        Self { gas_service }
    }

    pub const fn placeholder() -> Self {
        Self {
            gas_service: AccountId::new(u128::MAX),
        }
    }
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
        system_accounts: SystemAccounts,
    ) -> Self {
        Self {
            begin_blocker,
            end_blocker,
            tx_validator,
            post_tx_handler,
            system_accounts,
            _phantoms: PhantomData,
        }
    }
    /// Executes the given closure with system-level privileges.
    ///
    /// This method is intended for genesis initialization and system-level operations
    /// that need to bypass normal transaction flow. Unlike a traditional "sudo" model,
    /// this is not accessible during normal runtime - it can only be invoked by the
    /// consensus engine during specific lifecycle events (genesis, upgrades).
    ///
    /// # Use Cases
    ///
    /// - Genesis account creation and initialization
    /// - System parameter configuration
    /// - Protocol upgrades
    ///
    /// # Arguments
    ///
    /// * `storage` - Read-only storage backend
    /// * `account_codes` - Account code storage
    /// * `execution_height` - Block height for execution context
    /// * `action` - Closure to execute with system privileges
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the result of the action and the execution state.
    pub fn system_exec<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        execution_height: u64,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let mut execution_state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let resp = {
            let mut ctx = Invoker::new_for_begin_block(
                &mut execution_state,
                account_codes,
                &mut gas_counter,
                execution_height,
            );
            action(&mut ctx)?
        };
        execution_state.pop_events();
        Ok((resp, execution_state))
    }
    /// Executes the given closure impersonating the specified account with system privileges.
    ///
    /// This method allows executing operations as if they were initiated by a specific account,
    /// bypassing normal authentication. It is only available in test builds or with the
    /// `testing` feature enabled for security reasons.
    ///
    /// # Security Note
    ///
    /// This is a testing utility and should never be exposed in production builds.
    /// It exists to facilitate integration testing where simulating user actions
    /// is necessary without going through full transaction signing.
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
    pub fn system_exec_as<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a, R>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        execution_height: u64,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<(R, ExecutionState<'a, S>)> {
        let mut execution_state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let resp = {
            let mut ctx = Invoker::new_for_begin_block(
                &mut execution_state,
                account_codes,
                &mut gas_counter,
                execution_height,
            );
            ctx.whoami = impersonate;
            action(&mut ctx)?
        };
        execution_state.pop_events();
        Ok((resp, execution_state))
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
        let mut state = ExecutionState::new(storage);

        // run begin blocker.
        let (gas_config, begin_block_events) =
            self.do_begin_block(&mut state, account_codes, block);

        // apply txs.
        let txs = block.txs();
        let mut tx_results = Vec::with_capacity(txs.len());
        for tx in txs {
            // apply tx
            let tx_result: TxResult =
                self.apply_tx(&mut state, account_codes, tx, gas_config.clone());
            tx_results.push(tx_result);
        }

        let end_block_events = self.do_end_block(&mut state, account_codes, block.height());

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
            },
            state,
        )
    }

    pub fn apply_block_with_metrics<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        storage: &'a S,
        account_codes: &'a A,
        block: &Block,
    ) -> (BlockResult, ExecutionState<'a, S>, BlockExecutionMetrics) {
        let mut state = ExecutionState::new(storage);
        let block_start_metrics = state.metrics_snapshot();
        let block_timer = Instant::now();

        let begin_timer = Instant::now();
        let (gas_config, begin_block_events) =
            self.do_begin_block(&mut state, account_codes, block);
        let begin_block_ns = begin_timer.elapsed().as_nanos() as u64;

        let txs = block.txs();
        let mut tx_results = Vec::with_capacity(txs.len());
        let mut tx_metrics = Vec::with_capacity(txs.len());

        for (index, tx) in txs.iter().enumerate() {
            let tx_start_metrics = state.metrics_snapshot();
            let tx_timer = Instant::now();
            let tx_result = self.apply_tx(&mut state, account_codes, tx, gas_config.clone());
            let duration_ns = tx_timer.elapsed().as_nanos() as u64;
            let tx_end_metrics = state.metrics_snapshot();
            let delta = tx_end_metrics.delta(tx_start_metrics);
            tx_metrics.push(TxExecutionMetrics::from_delta(
                index as u64,
                duration_ns,
                tx_result.gas_used,
                delta,
            ));
            tx_results.push(tx_result);
        }

        let end_timer = Instant::now();
        let end_block_events = self.do_end_block(&mut state, account_codes, block.height());
        let end_block_ns = end_timer.elapsed().as_nanos() as u64;

        let total_ns = block_timer.elapsed().as_nanos() as u64;
        let block_end_metrics = state.metrics_snapshot();
        let delta = block_end_metrics.delta(block_start_metrics);
        let metrics = BlockExecutionMetrics::from_delta(
            begin_block_ns,
            end_block_ns,
            total_ns,
            delta,
            tx_metrics,
        );

        (
            BlockResult {
                begin_block_events,
                tx_results,
                end_block_events,
            },
            state,
            metrics,
        )
    }

    fn do_begin_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        account_codes: &'a A,
        block: &Block,
    ) -> (StorageGasConfig, Vec<Event>) {
        let mut gas_counter = GasCounter::Infinite;
        let gas_config = {
            let mut ctx = Invoker::new_for_begin_block(
                state,
                account_codes,
                &mut gas_counter,
                block.height(),
            );

            self.begin_blocker.begin_block(block, &mut ctx);

            // get system gas config
            ctx.run_as(
                self.system_accounts.gas_service,
                |account: &GasService, env: &mut dyn EnvironmentQuery| {
                    account.storage_gas_config.may_get(env)
                },
            )
            .expect("CRITICAL: Failed to run as gas service account - system configuration error")
            .expect("CRITICAL: Gas service must have storage gas config - blockchain cannot operate without gas configuration")
        };

        let events = state.pop_events();
        (gas_config, events)
    }

    fn do_end_block<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        codes: &'a A,
        height: u64,
    ) -> Vec<Event> {
        let mut gas_counter = GasCounter::Infinite;
        {
            let mut ctx = Invoker::new_for_end_block(state, codes, &mut gas_counter, height);
            self.end_blocker.end_block(&mut ctx);
        }
        state.pop_events()
    }

    fn apply_tx<'a, S: ReadonlyKV + 'a, A: AccountsCodeStorage + 'a>(
        &self,
        state: &mut ExecutionState<'a, S>,
        codes: &'a A,
        tx: &Tx,
        gas_config: StorageGasConfig,
    ) -> TxResult {
        // NOTE: Transaction validation and execution are atomic - they share the same
        // ExecutionState throughout the process. The state cannot change between
        // validation and execution because:
        // 1. The same ExecutionState instance is used for both phases
        // 2. into_new_exec() preserves the storage, maintaining consistency
        // 3. Transactions are processed sequentially, not concurrently

        // create validation context
        let mut gas_counter = GasCounter::Finite {
            gas_limit: tx.gas_limit(),
            gas_used: 0,
            storage_gas_config: gas_config,
        };
        let mut ctx = Invoker::new_for_validate_tx(state, codes, &mut gas_counter, tx);
        // do tx validation; we do not swap invoker
        match self.tx_validator.validate_tx(tx, &mut ctx) {
            Ok(()) => (),
            Err(err) => {
                drop(ctx);
                let gas_used = gas_counter.gas_used();
                let events = state.pop_events();
                return TxResult {
                    events,
                    gas_used,
                    response: Err(err),
                };
            }
        }

        // exec tx - transforms validation context to execution context
        // while preserving the same underlying state
        let mut ctx = ctx.into_new_exec(tx.sender());

        let response = ctx.do_exec(tx.recipient(), tx.request(), tx.funds().to_vec());

        drop(ctx);
        let gas_used = gas_counter.gas_used();
        let events = state.pop_events();

        // TODO do post tx validation

        TxResult {
            events,
            gas_used,
            response,
        }
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
        account_codes: &'a A,
        to: AccountId,
        req: &R,
        gc: GasCounter,
    ) -> SdkResult<InvokeResponse> {
        let mut state = ExecutionState::new(storage);
        let mut gas_counter = gc;
        let mut ctx = Invoker::new_for_query(&mut state, account_codes, &mut gas_counter, to);
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
        handle: impl FnOnce(&T, &mut dyn EnvironmentQuery) -> SdkResult<R>,
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
        handle: impl FnOnce(T, &mut dyn EnvironmentQuery) -> SdkResult<R>,
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
        handle: impl FnOnce(&mut dyn EnvironmentQuery) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let mut state = ExecutionState::new(storage);
        let mut gas_counter = GasCounter::Infinite;
        let mut invoker =
            Invoker::new_for_query(&mut state, account_storage, &mut gas_counter, account_id);
        handle(&mut invoker)
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
            system_accounts: self.system_accounts,
            _phantoms: PhantomData,
        }
    }
}
