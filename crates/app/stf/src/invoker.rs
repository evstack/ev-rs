use crate::execution_scope::ExecutionScope;
use crate::execution_state::ExecutionState;
use crate::gas::GasCounter;
use crate::handlers;
use crate::validation::validate_code_id;
use evolve_core::events_api::EVENT_HANDLER_ACCOUNT_ID;
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::storage_api::STORAGE_ACCOUNT_ID;
use evolve_core::unique_api::UNIQUE_HANDLER_ACCOUNT_ID;
use evolve_core::{
    AccountCode, AccountId, Environment, FungibleAsset, InvokeRequest, InvokeResponse, Message,
    ReadonlyKV, SdkResult,
};
use evolve_gas::account::StorageGasConfig;
use evolve_server_core::{AccountsCodeStorage, Transaction};
use std::cell::RefCell;
use std::rc::Rc;

use crate::errors::{ERR_ACCOUNT_DOES_NOT_EXIST, ERR_CODE_NOT_FOUND, ERR_FUNDS_TO_SYSTEM_ACCOUNT};
use crate::runtime_api_impl;

/// Execution context for account operations and transaction processing.
///
/// The Invoker provides the execution environment for account operations,
/// managing state transitions, gas metering, and inter-account communication.
/// It implements the `Environment` trait to provide accounts with access to
/// blockchain functionality.
///
/// # Type Parameters
///
/// * `'a` - Lifetime parameter for storage references
/// * `S` - Storage backend type that implements `ReadonlyKV`
/// * `A` - Account code storage type that implements `AccountsCodeStorage`
pub struct Invoker<'a, S, A> {
    pub(crate) whoami: AccountId,
    pub(crate) sender: AccountId,
    pub(crate) funds: Vec<FungibleAsset>,
    pub(crate) account_codes: Rc<RefCell<&'a A>>,
    pub(crate) storage: Rc<RefCell<ExecutionState<'a, S>>>,
    pub(crate) gas_counter: Rc<RefCell<GasCounter>>,
    pub(crate) scope: ExecutionScope,
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
            RUNTIME_ACCOUNT_ID => handlers::handle_system_query(data),
            STORAGE_ACCOUNT_ID => handlers::handle_storage_query(self, data),
            UNIQUE_HANDLER_ACCOUNT_ID => handlers::handle_unique_query(self, data),
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

        // Helper closure to restore checkpoint on error.
        //
        // # Panics
        //
        // This closure panics if checkpoint restoration fails. This is intentional and
        // unavoidable for the following reasons:
        //
        // 1. **State corruption**: A failed checkpoint restore means the in-memory state
        //    is inconsistent - some operations succeeded, others didn't, and we cannot
        //    determine which changes were applied.
        //
        // 2. **No safe recovery**: Returning an error would allow the blockchain to continue
        //    with corrupted state, potentially causing invalid state transitions, incorrect
        //    balances, or consensus failures across nodes.
        //
        // 3. **Fail-fast principle**: It's safer to halt the node immediately than to
        //    propagate corrupted state. Operators can investigate and restart from a
        //    known-good state.
        //
        // In practice, checkpoint restoration should never fail unless there's a bug in
        // the checkpoint/restore implementation or memory corruption has occurred.
        let restore_checkpoint = |storage: &RefCell<ExecutionState<'_, S>>| {
            if let Err(restore_err) = storage.borrow_mut().restore(checkpoint) {
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
            RUNTIME_ACCOUNT_ID => handlers::handle_system_exec(self, data, funds),
            // check if storage
            STORAGE_ACCOUNT_ID => handlers::handle_storage_exec(self, data),
            EVENT_HANDLER_ACCOUNT_ID => handlers::handle_event_handler_exec(self, data),
            UNIQUE_HANDLER_ACCOUNT_ID => handlers::handle_unique_exec(self, data),
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
    pub fn new_for_end_block(
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

    pub fn new_for_begin_block(
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

    pub fn new_for_query(
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

    pub fn new_for_validate_tx(
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

    pub fn branch_query(&self, whoami: AccountId) -> Self {
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

    pub fn branch_exec(&self, to: AccountId, funds: Vec<FungibleAsset>) -> Self {
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

    pub fn into_new_exec(self, sender: AccountId) -> Self {
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
            return Err(ERR_FUNDS_TO_SYSTEM_ACCOUNT);
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

    pub fn create_account(
        &mut self,
        code_id: &str,
        msg: Message,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<(AccountId, Message)> {
        // Validate code_id
        validate_code_id(code_id)?;

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

    pub fn with_account<R>(
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

    /// Consumes the Invoker and returns the final execution results.
    ///
    /// Returns the execution state, total gas used, and emitted events.
    ///
    /// # Panics
    ///
    /// This method panics if multiple references to `ExecutionState` exist when called.
    /// This is intentional because:
    ///
    /// 1. **Ownership invariant**: The Invoker owns the execution state and must be the
    ///    sole owner when finalization occurs. Multiple references indicate a bug in
    ///    reference management (e.g., a branched Invoker was not properly dropped).
    ///
    /// 2. **State integrity**: We cannot safely extract and return the `ExecutionState`
    ///    if other references exist - they could mutate it after we've "finalized" it,
    ///    leading to inconsistent state being committed to the blockchain.
    ///
    /// 3. **No graceful fallback**: Cloning `ExecutionState` is not semantically correct
    ///    here as it would mean committing a snapshot while other code holds references
    ///    to the "real" state.
    ///
    /// If this panic occurs, it indicates a bug in the STF that must be fixed. The panic
    /// message includes diagnostic information to aid debugging.
    pub fn into_execution_results(
        self,
    ) -> (
        ExecutionState<'a, S>,
        u64,
        Vec<evolve_core::events_api::Event>,
    ) {
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
