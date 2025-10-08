//! # System Message Handlers
//!
//! This module contains handlers for system-level operations including:
//! - Account creation and migration
//! - Storage operations
//!
//! These handlers process messages sent to system accounts and provide
//! core blockchain functionality.

use crate::errors::{ERR_ACCOUNT_DOES_NOT_EXIST, ERR_CODE_NOT_FOUND, ERR_SAME_CODE_MIGRATION};
use crate::invoker::Invoker;
use crate::runtime_api_impl;
use evolve_core::runtime_api::{
    CreateAccountRequest, CreateAccountResponse, MigrateRequest, RUNTIME_ACCOUNT_ID,
};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageRemoveResponse,
    StorageSetRequest, StorageSetResponse,
};
use evolve_core::{
    Environment, FungibleAsset, InvokableMessage, InvokeRequest, InvokeResponse, ReadonlyKV,
    SdkResult, ERR_UNAUTHORIZED, ERR_UNKNOWN_FUNCTION,
};
use evolve_stf_traits::AccountsCodeStorage;

/// Handles execution requests sent to the system/runtime account.
///
/// This function processes system-level operations such as account creation
/// and migration. It validates the request and executes the appropriate
/// system functionality.
///
/// # Arguments
///
/// * `invoker` - Mutable reference to the execution context
/// * `request` - The system execution request
/// * `funds` - Fungible assets to transfer with the request
///
/// # Returns
///
/// Returns the response from the system operation.
pub fn handle_system_exec<S: ReadonlyKV, A: AccountsCodeStorage>(
    invoker: &mut Invoker<'_, '_, S, A>,
    request: &InvokeRequest,
    funds: Vec<FungibleAsset>,
) -> SdkResult<InvokeResponse> {
    match request.function() {
        CreateAccountRequest::FUNCTION_IDENTIFIER => {
            let req: CreateAccountRequest = request.get()?;

            let (new_account_id, init_response) =
                invoker.create_account(&req.code_id, req.init_message, funds)?;

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
            let mut invoker = invoker.branch_exec(RUNTIME_ACCOUNT_ID, vec![]);

            let req: MigrateRequest = request.get()?;
            // TODO: enhance with more migration delegation rights
            if req.account_id != invoker.sender {
                return Err(ERR_UNAUTHORIZED);
            }

            // Validate that the account exists before migration
            let current_code_id = runtime_api_impl::get_account_code_identifier_for_account(
                invoker.storage,
                req.account_id,
            )?
            .ok_or(ERR_ACCOUNT_DOES_NOT_EXIST)?;

            // Validate that the new code exists
            let code_exists = invoker
                .account_codes
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
                invoker.storage,
                req.account_id,
                &req.new_code_id,
            )?;
            // make runtime invoke the exec msg
            invoker.do_exec(req.account_id, &req.execute_message, funds)
        }
        _ => Err(ERR_UNKNOWN_FUNCTION),
    }
}

pub fn handle_system_query(_request: &InvokeRequest) -> SdkResult<InvokeResponse> {
    Err(ERR_UNKNOWN_FUNCTION)
}

pub fn handle_storage_exec<S: ReadonlyKV, A: AccountsCodeStorage>(
    invoker: &mut Invoker<'_, '_, S, A>,
    request: &InvokeRequest,
) -> SdkResult<InvokeResponse> {
    match request.function() {
        StorageSetRequest::FUNCTION_IDENTIFIER => {
            let storage_set: StorageSetRequest = request.get()?;

            let mut key = invoker.whoami.as_bytes();
            key.extend(storage_set.key);

            // increase gas costs
            invoker
                .gas_counter
                .consume_set_gas(&key, &storage_set.value)?;
            invoker.storage.set(&key, storage_set.value)?;

            Ok(InvokeResponse::new(&StorageSetResponse {})?)
        }
        StorageRemoveRequest::FUNCTION_IDENTIFIER => {
            let storage_remove: StorageRemoveRequest = request.get()?;
            let mut key = invoker.whoami.as_bytes();
            key.extend(storage_remove.key);
            invoker.gas_counter.consume_remove_gas(&key)?;
            invoker.storage.remove(&key)?;
            Ok(InvokeResponse::new(&StorageRemoveResponse {})?)
        }
        _ => Err(ERR_UNKNOWN_FUNCTION),
    }
}

pub fn handle_storage_query<S: ReadonlyKV, A: AccountsCodeStorage>(
    invoker: &mut Invoker<'_, '_, S, A>,
    request: &InvokeRequest,
) -> SdkResult<InvokeResponse> {
    match request.function() {
        StorageGetRequest::FUNCTION_IDENTIFIER => {
            let storage_get: StorageGetRequest = request.get()?;

            let mut key = storage_get.account_id.as_bytes();
            key.extend(storage_get.key);

            let value = invoker.storage.get(&key)?;

            invoker.gas_counter.consume_get_gas(&key, &value)?;

            Ok(InvokeResponse::new(&StorageGetResponse { value })?)
        }
        _ => Err(ERR_UNKNOWN_FUNCTION),
    }
}

