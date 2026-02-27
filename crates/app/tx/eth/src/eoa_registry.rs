use crate::traits::{derive_eth_eoa_account_id, derive_runtime_contract_address};
use alloy_primitives::Address;
use evolve_core::low_level::{exec_account, query_account, register_account_at_id};
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageSetRequest, StorageSetResponse,
    STORAGE_ACCOUNT_ID,
};
use evolve_core::{AccountId, Environment, EnvironmentQuery, Message, ReadonlyKV, SdkResult};

const EOA_ADDR_TO_ID_PREFIX: &[u8] = b"registry/eoa/eth/a2i/";
const EOA_ID_TO_ADDR_PREFIX: &[u8] = b"registry/eoa/eth/i2a/";
const CONTRACT_ADDR_TO_ID_PREFIX: &[u8] = b"registry/contract/runtime/a2i/";
const CONTRACT_ID_TO_ADDR_PREFIX: &[u8] = b"registry/contract/runtime/i2a/";
const ETH_EOA_CODE_ID: &str = "EthEoaAccount";

fn addr_to_id_key(address: Address) -> Vec<u8> {
    let mut key = Vec::with_capacity(EOA_ADDR_TO_ID_PREFIX.len() + 20);
    key.extend_from_slice(EOA_ADDR_TO_ID_PREFIX);
    key.extend_from_slice(address.as_slice());
    key
}

fn id_to_addr_key(account_id: AccountId) -> Vec<u8> {
    let account_bytes = account_id.as_bytes();
    let mut key = Vec::with_capacity(EOA_ID_TO_ADDR_PREFIX.len() + account_bytes.len());
    key.extend_from_slice(EOA_ID_TO_ADDR_PREFIX);
    key.extend_from_slice(&account_bytes);
    key
}

fn contract_addr_to_id_key(address: Address) -> Vec<u8> {
    let mut key = Vec::with_capacity(CONTRACT_ADDR_TO_ID_PREFIX.len() + 20);
    key.extend_from_slice(CONTRACT_ADDR_TO_ID_PREFIX);
    key.extend_from_slice(address.as_slice());
    key
}

fn contract_id_to_addr_key(account_id: AccountId) -> Vec<u8> {
    let account_bytes = account_id.as_bytes();
    let mut key = Vec::with_capacity(CONTRACT_ID_TO_ADDR_PREFIX.len() + account_bytes.len());
    key.extend_from_slice(CONTRACT_ID_TO_ADDR_PREFIX);
    key.extend_from_slice(&account_bytes);
    key
}

fn decode_account_id(message: Message) -> SdkResult<AccountId> {
    message.get::<AccountId>()
}

pub fn lookup_account_id_in_env(
    address: Address,
    env: &mut dyn EnvironmentQuery,
) -> SdkResult<Option<AccountId>> {
    let response: StorageGetResponse = query_account(
        STORAGE_ACCOUNT_ID,
        &StorageGetRequest {
            account_id: RUNTIME_ACCOUNT_ID,
            key: addr_to_id_key(address),
        },
        env,
    )?;
    match response.value {
        Some(raw) => Ok(Some(decode_account_id(raw)?)),
        None => Ok(None),
    }
}

pub fn lookup_address_in_env(
    account_id: AccountId,
    env: &mut dyn EnvironmentQuery,
) -> SdkResult<Option<Address>> {
    let response: StorageGetResponse = query_account(
        STORAGE_ACCOUNT_ID,
        &StorageGetRequest {
            account_id: RUNTIME_ACCOUNT_ID,
            key: id_to_addr_key(account_id),
        },
        env,
    )?;
    match response.value {
        Some(raw) => {
            let bytes = raw.get::<[u8; 20]>()?;
            Ok(Some(Address::from(bytes)))
        }
        None => Ok(None),
    }
}

pub fn lookup_account_id_in_storage<S: ReadonlyKV>(
    storage: &S,
    address: Address,
) -> SdkResult<Option<AccountId>> {
    let mut full_key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
    full_key.extend_from_slice(&addr_to_id_key(address));
    match storage.get(&full_key)? {
        Some(raw) => Ok(Some(decode_account_id(Message::from_bytes(raw))?)),
        None => Ok(None),
    }
}

pub fn lookup_contract_account_id_in_env(
    address: Address,
    env: &mut dyn EnvironmentQuery,
) -> SdkResult<Option<AccountId>> {
    let response: StorageGetResponse = query_account(
        STORAGE_ACCOUNT_ID,
        &StorageGetRequest {
            account_id: RUNTIME_ACCOUNT_ID,
            key: contract_addr_to_id_key(address),
        },
        env,
    )?;
    match response.value {
        Some(raw) => Ok(Some(decode_account_id(raw)?)),
        None => Ok(None),
    }
}

pub fn lookup_contract_account_id_in_storage<S: ReadonlyKV>(
    storage: &S,
    address: Address,
) -> SdkResult<Option<AccountId>> {
    let mut full_key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
    full_key.extend_from_slice(&contract_addr_to_id_key(address));
    match storage.get(&full_key)? {
        Some(raw) => Ok(Some(decode_account_id(Message::from_bytes(raw))?)),
        None => Ok(None),
    }
}

pub fn lookup_address_in_storage<S: ReadonlyKV>(
    storage: &S,
    account_id: AccountId,
) -> SdkResult<Option<Address>> {
    let mut full_key = RUNTIME_ACCOUNT_ID.as_bytes().to_vec();
    full_key.extend_from_slice(&id_to_addr_key(account_id));
    match storage.get(&full_key)? {
        Some(raw) => {
            let bytes = Message::from_bytes(raw).get::<[u8; 20]>()?;
            Ok(Some(Address::from(bytes)))
        }
        None => Ok(None),
    }
}

fn set_mapping(
    address: Address,
    account_id: AccountId,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    if let Some(existing) = lookup_account_id_in_env(address, env)? {
        if existing != account_id {
            return Err(evolve_core::ErrorCode::new(0x5A));
        }
    }
    if let Some(existing_addr) = lookup_address_in_env(account_id, env)? {
        if existing_addr != address {
            return Err(evolve_core::ErrorCode::new(0x5A));
        }
    }

    let _resp: StorageSetResponse = exec_account(
        STORAGE_ACCOUNT_ID,
        &StorageSetRequest {
            key: addr_to_id_key(address),
            value: Message::new(&account_id)?,
        },
        vec![],
        env,
    )?;
    let _resp: StorageSetResponse = exec_account(
        STORAGE_ACCOUNT_ID,
        &StorageSetRequest {
            key: id_to_addr_key(account_id),
            value: Message::new(&address.into_array())?,
        },
        vec![],
        env,
    )?;
    Ok(())
}

pub fn resolve_or_create_eoa_account(
    address: Address,
    env: &mut dyn Environment,
) -> SdkResult<AccountId> {
    if let Some(account_id) = lookup_account_id_in_env(address, env)? {
        return Ok(account_id);
    }

    let account_id = derive_eth_eoa_account_id(address);
    register_account_at_id(
        account_id,
        ETH_EOA_CODE_ID.to_string(),
        &address.into_array(),
        env,
    )?;

    set_mapping(address, account_id, env)?;
    Ok(account_id)
}

pub fn register_runtime_contract_account(
    account_id: AccountId,
    env: &mut dyn Environment,
) -> SdkResult<Address> {
    let address = derive_runtime_contract_address(account_id);
    if let Some(existing) = lookup_contract_account_id_in_env(address, env)? {
        if existing != account_id {
            return Err(evolve_core::ErrorCode::new(0x5A));
        }
    }

    let response: StorageGetResponse = query_account(
        STORAGE_ACCOUNT_ID,
        &StorageGetRequest {
            account_id: RUNTIME_ACCOUNT_ID,
            key: contract_id_to_addr_key(account_id),
        },
        env,
    )?;
    if let Some(raw) = response.value {
        let existing = raw.get::<[u8; 20]>()?;
        if existing != address.into_array() {
            return Err(evolve_core::ErrorCode::new(0x5A));
        }
    }

    let _resp: StorageSetResponse = exec_account(
        STORAGE_ACCOUNT_ID,
        &StorageSetRequest {
            key: contract_addr_to_id_key(address),
            value: Message::new(&account_id)?,
        },
        vec![],
        env,
    )?;
    let _resp: StorageSetResponse = exec_account(
        STORAGE_ACCOUNT_ID,
        &StorageSetRequest {
            key: contract_id_to_addr_key(account_id),
            value: Message::new(&address.into_array())?,
        },
        vec![],
        env,
    )?;
    Ok(address)
}
