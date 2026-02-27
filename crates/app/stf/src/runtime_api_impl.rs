use crate::execution_state::ExecutionState;
use evolve_core::runtime_api::{ACCOUNT_IDENTIFIER_PREFIX, ACCOUNT_IDENTIFIER_SINGLETON_PREFIX};
use evolve_core::{AccountId, Message, ReadonlyKV, SdkResult};

/// Initial account ID value - starts from u16::MAX to distinguish from default 0
const INITIAL_ACCOUNT_ID: u64 = u16::MAX as u64;

pub(crate) fn get_account_code_identifier_for_account<S: ReadonlyKV>(
    storage: &ExecutionState<S>,
    account: AccountId,
) -> SdkResult<Option<String>> {
    let key = get_account_code_identifier_for_account_key(account);
    let code_id = storage.get(&key)?;
    match code_id {
        None => Ok(None),
        Some(code_id) => Ok(Some(code_id.get()?)),
    }
}

pub(crate) fn set_account_code_identifier_for_account<S: ReadonlyKV>(
    storage: &mut ExecutionState<S>,
    account: AccountId,
    account_identifier: &str,
) -> SdkResult<()> {
    let key = get_account_code_identifier_for_account_key(account);
    storage.set(&key, Message::new(&account_identifier)?)
}

fn get_account_code_identifier_for_account_key(account: AccountId) -> Vec<u8> {
    let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
    key.extend_from_slice(&account.as_bytes());
    key
}

pub(crate) fn next_account_number<S: ReadonlyKV>(
    storage: &mut ExecutionState<S>,
) -> SdkResult<AccountId> {
    let key = vec![ACCOUNT_IDENTIFIER_SINGLETON_PREFIX];

    // get last
    let last = storage
        .get(&key)?
        .map_or(Ok(AccountId::new(INITIAL_ACCOUNT_ID.into())), |msg| {
            msg.get()
        })?;

    // set next
    storage.set(&key, Message::new(&last.increase())?)?;
    Ok(last)
}
