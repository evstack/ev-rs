use crate::execution_scope::ExecutionScope;
use crate::execution_state::ExecutionState;
use crate::gas::GasCounter;
use crate::handlers;
use crate::validation::{validate_code_id, validate_event};
use evolve_core::events_api::Event;
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::storage_api::STORAGE_ACCOUNT_ID;
use evolve_core::unique_api::UNIQUE_HANDLER_ACCOUNT_ID;
use evolve_core::{
    AccountCode, AccountId, BlockContext, Environment, EnvironmentQuery, FungibleAsset,
    InvokeRequest, InvokeResponse, Message, ReadonlyKV, SdkResult,
};
use evolve_stf_traits::{AccountsCodeStorage, Transaction};

use crate::errors::{
    ERR_ACCOUNT_DOES_NOT_EXIST, ERR_CALL_DEPTH_EXCEEDED, ERR_CODE_NOT_FOUND,
    ERR_FUNDS_TO_SYSTEM_ACCOUNT,
};
use crate::runtime_api_impl;

/// Maximum call depth to prevent stack overflow from deeply nested inter-account calls.
/// This is a conservative limit - typical call chains are much shorter.
const MAX_CALL_DEPTH: u16 = 64;

/// Execution context for account operations and transaction processing.
///
/// The Invoker provides the execution environment for account operations,
/// managing state transitions, gas metering, and inter-account communication.
/// It implements the `Environment` trait to provide accounts with access to
/// blockchain functionality.
pub struct Invoker<'s, 'a, S, A> {
    pub(crate) whoami: AccountId,
    pub(crate) sender: AccountId,
    pub(crate) funds: Vec<FungibleAsset>,
    pub(crate) account_codes: &'a A,
    pub(crate) storage: &'a mut ExecutionState<'s, S>,
    pub(crate) gas_counter: &'a mut GasCounter,
    pub(crate) scope: ExecutionScope,
    /// Current call depth for detecting excessive nesting.
    pub(crate) call_depth: u16,
    /// Block context (height, time, etc.).
    pub(crate) block: BlockContext,
}

impl<S: ReadonlyKV, A: AccountsCodeStorage> EnvironmentQuery for Invoker<'_, '_, S, A> {
    fn whoami(&self) -> AccountId {
        self.whoami
    }

    fn sender(&self) -> AccountId {
        self.sender
    }

    fn funds(&self) -> &[FungibleAsset] {
        &self.funds
    }

    fn block(&self) -> BlockContext {
        self.block
    }

    fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match to {
            RUNTIME_ACCOUNT_ID => handlers::handle_system_query(data),
            STORAGE_ACCOUNT_ID => handlers::handle_storage_query(self, data),
            UNIQUE_HANDLER_ACCOUNT_ID => handlers::handle_unique_query(self, data),
            _ => {
                let account_codes = self.account_codes;
                let code_id =
                    runtime_api_impl::get_account_code_identifier_for_account(self.storage, to)?
                        .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;
                let mut invoker = self.branch_query(to);
                account_codes.with_code(&code_id, |code| match code {
                    Some(code) => code.query(&mut invoker, data),
                    None => Err(ERR_CODE_NOT_FOUND),
                })?
            }
        }
    }
}

impl<S: ReadonlyKV, A: AccountsCodeStorage> Environment for Invoker<'_, '_, S, A> {
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        // Check call depth to prevent stack overflow
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(ERR_CALL_DEPTH_EXCEEDED);
        }

        // Take checkpoint before ANY state changes
        let checkpoint = self.storage.checkpoint();

        // Helper closure to restore checkpoint on error.
        let restore_checkpoint = |storage: &mut ExecutionState<'_, S>| {
            if let Err(restore_err) = storage.restore(checkpoint) {
                eprintln!("CRITICAL: Failed to restore checkpoint: {restore_err:?}");
                panic!(
                    "CRITICAL: Failed to restore checkpoint after error - blockchain state is corrupted and cannot continue safely"
                );
            }
        };

        // Handle transfers - restore checkpoint if this fails
        if let Err(transfer_err) = self.handle_transfers(to, funds.as_ref()) {
            restore_checkpoint(self.storage);
            return Err(transfer_err);
        }

        let resp = self.exec_without_transfers(to, data, funds);

        // restore checkpoint in case of failure and yield back.
        if resp.is_err() {
            restore_checkpoint(self.storage);
        }
        resp
    }

    fn emit_event(&mut self, name: &str, data: &[u8]) -> SdkResult<()> {
        validate_event(name, data)?;
        let event = Event {
            source: self.whoami,
            name: name.to_string(),
            contents: Message::from_bytes(data.to_vec()),
        };
        self.storage.emit_event(event)?;
        Ok(())
    }
}

impl<'s, 'a, S: ReadonlyKV, A: AccountsCodeStorage> Invoker<'s, 'a, S, A> {
    pub fn new_for_end_block(
        storage: &'a mut ExecutionState<'s, S>,
        account_codes: &'a A,
        gas_counter: &'a mut GasCounter,
        block: BlockContext,
    ) -> Self {
        Self {
            whoami: RUNTIME_ACCOUNT_ID,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes,
            storage,
            gas_counter,
            scope: ExecutionScope::EndBlock(block.height),
            call_depth: 0,
            block,
        }
    }

    pub fn new_for_begin_block(
        storage: &'a mut ExecutionState<'s, S>,
        account_codes: &'a A,
        gas_counter: &'a mut GasCounter,
        block: BlockContext,
    ) -> Self {
        Self {
            whoami: RUNTIME_ACCOUNT_ID,
            sender: AccountId::invalid(),
            funds: vec![],
            account_codes,
            storage,
            gas_counter,
            scope: ExecutionScope::BeginBlock(block.height),
            call_depth: 0,
            block,
        }
    }

    pub fn new_for_query(
        storage: &'a mut ExecutionState<'s, S>,
        account_codes: &'a A,
        gas_counter: &'a mut GasCounter,
        recipient: AccountId,
    ) -> Self {
        Self {
            whoami: recipient,
            sender: RUNTIME_ACCOUNT_ID,
            funds: vec![],
            account_codes,
            storage,
            gas_counter,
            scope: ExecutionScope::Query,
            call_depth: 0,
            block: BlockContext::default(),
        }
    }

    pub fn new_for_validate_tx(
        storage: &'a mut ExecutionState<'s, S>,
        account_codes: &'a A,
        gas_counter: &'a mut GasCounter,
        tx: &impl Transaction,
        block: BlockContext,
    ) -> Self {
        Self {
            whoami: AccountId::invalid(), // whoami is invalid because no one is receiving anything as of now.
            sender: RUNTIME_ACCOUNT_ID,   // sender is runtime for tx validation
            funds: vec![],                // no funds for this.
            account_codes,
            storage,
            gas_counter,
            scope: ExecutionScope::Transaction(tx.compute_identifier()),
            call_depth: 0,
            block,
        }
    }

    pub fn branch_query<'b>(&'b mut self, whoami: AccountId) -> Invoker<'s, 'b, S, A> {
        Invoker {
            whoami,
            sender: RUNTIME_ACCOUNT_ID,
            funds: vec![],
            account_codes: self.account_codes,
            storage: self.storage,
            gas_counter: self.gas_counter,
            scope: self.scope,
            call_depth: self.call_depth.saturating_add(1),
            block: self.block,
        }
    }

    pub fn branch_exec<'b>(
        &'b mut self,
        to: AccountId,
        funds: Vec<FungibleAsset>,
    ) -> Invoker<'s, 'b, S, A> {
        Invoker {
            whoami: to,
            sender: self.whoami,
            funds,
            account_codes: self.account_codes,
            storage: self.storage,
            gas_counter: self.gas_counter,
            scope: self.scope,
            call_depth: self.call_depth.saturating_add(1),
            block: self.block,
        }
    }

    pub fn into_new_exec(mut self, sender: AccountId) -> Self {
        self.whoami = sender;
        self.sender = AccountId::invalid();
        self.funds.clear();
        self
    }

    fn exec_without_transfers(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        match to {
            // check if system
            RUNTIME_ACCOUNT_ID => handlers::handle_system_exec(self, data, funds),
            // check if storage
            STORAGE_ACCOUNT_ID => handlers::handle_storage_exec(self, data),
            UNIQUE_HANDLER_ACCOUNT_ID => handlers::handle_unique_exec(self, data),
            // other account
            _ => {
                let account_codes = self.account_codes;
                let code_id =
                    runtime_api_impl::get_account_code_identifier_for_account(self.storage, to)?
                        .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;
                let mut invoker = self.branch_exec(to, funds);
                account_codes.with_code(&code_id, |code| match code {
                    Some(code) => code.execute(&mut invoker, data),
                    None => Err(ERR_CODE_NOT_FOUND),
                })?
            }
        }
    }

    fn handle_transfers(&mut self, to: AccountId, funds: &[FungibleAsset]) -> SdkResult<()> {
        // Early return if no funds to transfer
        if funds.is_empty() {
            return Ok(());
        }

        self.storage.reserve_writes(funds.len().saturating_mul(2));

        // Validate recipient is not a system account that shouldn't receive funds
        if to == STORAGE_ACCOUNT_ID || to == UNIQUE_HANDLER_ACCOUNT_ID {
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
            self.exec_without_transfers(asset.asset_id, &InvokeRequest::new(&msg)?, vec![])?;
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
        let new_account_id = runtime_api_impl::next_account_number(self.storage)?;
        runtime_api_impl::set_account_code_identifier_for_account(
            self.storage,
            new_account_id,
            code_id,
        )?;
        // prepare request params
        let req = InvokeRequest::new_from_message("init", 0, msg);
        let account_codes = self.account_codes;
        let stored_code_id = runtime_api_impl::get_account_code_identifier_for_account(
            self.storage,
            new_account_id,
        )?
        .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;
        let mut invoker = self.branch_exec(new_account_id, funds);
        // do account init
        let init_resp = account_codes.with_code(&stored_code_id, |code| match code {
            Some(code) => Ok(code.init(&mut invoker, &req)?),
            None => Err(ERR_CODE_NOT_FOUND),
        })??;

        Ok((new_account_id, init_resp.into_inner()))
    }

    /// Executes a closure with access to a specific account's code and environment.
    ///
    /// This utility method allows reading from an account's state without going through
    /// the full message dispatch flow. Useful for system-level operations that need to
    /// inspect account state directly.
    #[allow(dead_code)]
    pub fn run_as<T: AccountCode + Default, R>(
        &mut self,
        account_id: AccountId,
        handle: impl FnOnce(&T, &mut dyn EnvironmentQuery) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let mut env = self.branch_query(account_id);
        let code = T::default();
        handle(&code, &mut env)
    }

    /// Returns the current gas consumed.
    pub fn gas_used(&self) -> u64 {
        self.gas_counter.gas_used()
    }
}
