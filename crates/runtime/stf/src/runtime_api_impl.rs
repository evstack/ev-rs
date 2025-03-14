use crate::execution_state::ExecutionState;
use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::runtime_api::{ACCOUNT_IDENTIFIER_PREFIX, ACCOUNT_IDENTIFIER_SINGLETON_PREFIX};
use evolve_core::{AccountId, ReadonlyKV, SdkResult};

pub(crate) fn get_account_code_identifier_for_account<S: ReadonlyKV>(
    storage: &ExecutionState<S>,
    account: AccountId,
) -> SdkResult<Option<String>> {
    let key = get_account_code_identifier_for_account_key(account);
    let code_id = storage.get(&key)?;

    Ok(code_id.map(|e| String::from_utf8(e).unwrap())) // TODO
}

pub(crate) fn set_account_code_identifier_for_account<S: ReadonlyKV>(
    storage: &mut ExecutionState<S>,
    account: AccountId,
    account_identifier: &str,
) -> SdkResult<()> {
    let key = get_account_code_identifier_for_account_key(account);
    storage.set(&key, account_identifier.as_bytes().to_vec())
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
        .map(|bytes| AccountId::decode(&bytes))
        .unwrap_or(Ok(AccountId::new(u16::MAX.into())))?;

    // set next
    storage.set(&key, last.increase().encode()?)?;
    Ok(last)
}
