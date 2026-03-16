use crate::error::ERR_ADDRESS_ACCOUNT_CONFLICT;
use crate::traits::{derive_eth_eoa_account_id, derive_runtime_contract_address};
use alloy_primitives::Address;
use evolve_core::low_level::{exec_account, query_account, register_account_at_id};
use evolve_core::runtime_api::{ACCOUNT_STORAGE_PREFIX, RUNTIME_ACCOUNT_ID};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageSetRequest, StorageSetResponse,
    STORAGE_ACCOUNT_ID,
};
use evolve_core::{AccountId, Environment, EnvironmentQuery, Message, ReadonlyKV, SdkResult};

const EOA_ADDR_TO_ID_PREFIX: &[u8] = b"registry/eoa/eth/a2i/";
const EOA_ID_TO_ADDR_PREFIX: &[u8] = b"registry/eoa/eth/i2a/";
const CONTRACT_ADDR_TO_ID_PREFIX: &[u8] = b"registry/contract/runtime/a2i/";
const CONTRACT_ID_TO_ADDR_PREFIX: &[u8] = b"registry/contract/runtime/i2a/";
/// Code identifier for Ethereum externally owned accounts.
pub const ETH_EOA_CODE_ID: &str = "EthEoaAccount";

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
    let mut full_key = vec![ACCOUNT_STORAGE_PREFIX];
    full_key.extend_from_slice(&RUNTIME_ACCOUNT_ID.as_bytes());
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
    let mut full_key = vec![ACCOUNT_STORAGE_PREFIX];
    full_key.extend_from_slice(&RUNTIME_ACCOUNT_ID.as_bytes());
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
    let mut full_key = vec![ACCOUNT_STORAGE_PREFIX];
    full_key.extend_from_slice(&RUNTIME_ACCOUNT_ID.as_bytes());
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
    let (forward, reverse) = ensure_mapping_conflict_free(address, account_id, env)?;
    if forward.is_some() && reverse.is_some() {
        return Ok(());
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

fn ensure_mapping_conflict_free(
    address: Address,
    account_id: AccountId,
    env: &mut dyn EnvironmentQuery,
) -> SdkResult<(Option<AccountId>, Option<Address>)> {
    let forward = lookup_account_id_in_env(address, env)?;
    let reverse = lookup_address_in_env(account_id, env)?;

    if let Some(existing) = forward {
        if existing != account_id {
            return Err(ERR_ADDRESS_ACCOUNT_CONFLICT);
        }
    }

    if let Some(existing_addr) = reverse {
        if existing_addr != address {
            return Err(ERR_ADDRESS_ACCOUNT_CONFLICT);
        }
    }

    Ok((forward, reverse))
}

/// Ensure address/account mapping exists and is conflict-free.
///
/// This is idempotent for already consistent mappings.
pub fn ensure_eoa_mapping(
    address: Address,
    account_id: AccountId,
    env: &mut dyn Environment,
) -> SdkResult<()> {
    let (forward, reverse) = ensure_mapping_conflict_free(address, account_id, env)?;

    if forward.is_some() && reverse.is_some() {
        return Ok(());
    }

    set_mapping(address, account_id, env)
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

    ensure_eoa_mapping(address, account_id, env)?;
    Ok(account_id)
}

pub fn register_runtime_contract_account(
    account_id: AccountId,
    env: &mut dyn Environment,
) -> SdkResult<Address> {
    let address = derive_runtime_contract_address(account_id);
    if let Some(existing) = lookup_contract_account_id_in_env(address, env)? {
        if existing != account_id {
            return Err(ERR_ADDRESS_ACCOUNT_CONFLICT);
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
            return Err(ERR_ADDRESS_ACCOUNT_CONFLICT);
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

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
    use evolve_core::storage_api::{
        StorageGetRequest, StorageGetResponse, StorageSetRequest, StorageSetResponse,
        STORAGE_ACCOUNT_ID,
    };
    use evolve_core::{
        BlockContext, Environment, ErrorCode, FungibleAsset, InvokableMessage, InvokeRequest,
        InvokeResponse, ERR_UNKNOWN_FUNCTION,
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MockEnv {
        storage: BTreeMap<Vec<u8>, Vec<u8>>,
    }

    impl MockEnv {
        fn account_scoped_key(account_id: AccountId, key: &[u8]) -> Vec<u8> {
            let mut full = account_id.as_bytes().to_vec();
            full.extend_from_slice(key);
            full
        }
    }

    impl EnvironmentQuery for MockEnv {
        fn whoami(&self) -> AccountId {
            RUNTIME_ACCOUNT_ID
        }

        fn sender(&self) -> AccountId {
            AccountId::invalid()
        }

        fn funds(&self) -> &[FungibleAsset] {
            &[]
        }

        fn block(&self) -> BlockContext {
            BlockContext::default()
        }

        fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
            if to != STORAGE_ACCOUNT_ID {
                return Err(ERR_UNKNOWN_FUNCTION);
            }

            match data.function() {
                StorageGetRequest::FUNCTION_IDENTIFIER => {
                    let req: StorageGetRequest = data.get()?;
                    let key = Self::account_scoped_key(req.account_id, &req.key);
                    let value = self.storage.get(&key).cloned().map(Message::from_bytes);
                    InvokeResponse::new(&StorageGetResponse { value })
                }
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }
    }

    impl Environment for MockEnv {
        fn do_exec(
            &mut self,
            to: AccountId,
            data: &InvokeRequest,
            _funds: Vec<FungibleAsset>,
        ) -> SdkResult<InvokeResponse> {
            if to != STORAGE_ACCOUNT_ID {
                return Err(ERR_UNKNOWN_FUNCTION);
            }

            match data.function() {
                StorageSetRequest::FUNCTION_IDENTIFIER => {
                    let req: StorageSetRequest = data.get()?;
                    let key = Self::account_scoped_key(RUNTIME_ACCOUNT_ID, &req.key);
                    self.storage.insert(key, req.value.into_bytes()?);
                    InvokeResponse::new(&StorageSetResponse {})
                }
                _ => Err(ERR_UNKNOWN_FUNCTION),
            }
        }

        fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
            Ok(())
        }

        fn unique_id(&mut self) -> Result<[u8; 32], ErrorCode> {
            Ok([0u8; 32])
        }
    }

    fn address(byte: u8) -> Address {
        Address::repeat_byte(byte)
    }

    fn account_id(value: u64) -> AccountId {
        AccountId::from_u64(value)
    }

    #[test]
    fn ensure_eoa_mapping_is_idempotent_for_existing_pair() {
        let mut env = MockEnv::default();
        let address = address(0x11);
        let account_id = account_id(101);

        set_mapping(address, account_id, &mut env).expect("seed mapping");
        ensure_eoa_mapping(address, account_id, &mut env).expect("idempotent mapping");

        assert_eq!(
            lookup_account_id_in_env(address, &mut env).expect("forward lookup"),
            Some(account_id)
        );
        assert_eq!(
            lookup_address_in_env(account_id, &mut env).expect("reverse lookup"),
            Some(address)
        );
    }

    #[test]
    fn ensure_eoa_mapping_repairs_missing_reverse_entry() {
        let mut env = MockEnv::default();
        let address = address(0x22);
        let account_id = account_id(202);

        let key = MockEnv::account_scoped_key(RUNTIME_ACCOUNT_ID, &addr_to_id_key(address));
        env.storage.insert(
            key,
            Message::new(&account_id).unwrap().into_bytes().unwrap(),
        );

        ensure_eoa_mapping(address, account_id, &mut env).expect("repair reverse mapping");

        assert_eq!(
            lookup_account_id_in_env(address, &mut env).expect("forward lookup"),
            Some(account_id)
        );
        assert_eq!(
            lookup_address_in_env(account_id, &mut env).expect("reverse lookup"),
            Some(address)
        );
    }

    #[test]
    fn ensure_eoa_mapping_repairs_missing_forward_entry() {
        let mut env = MockEnv::default();
        let address = address(0x33);
        let account_id = account_id(303);

        let key = MockEnv::account_scoped_key(RUNTIME_ACCOUNT_ID, &id_to_addr_key(account_id));
        env.storage.insert(
            key,
            Message::new(&address.into_array())
                .unwrap()
                .into_bytes()
                .unwrap(),
        );

        ensure_eoa_mapping(address, account_id, &mut env).expect("repair forward mapping");

        assert_eq!(
            lookup_account_id_in_env(address, &mut env).expect("forward lookup"),
            Some(account_id)
        );
        assert_eq!(
            lookup_address_in_env(account_id, &mut env).expect("reverse lookup"),
            Some(address)
        );
    }

    #[test]
    fn ensure_eoa_mapping_rejects_forward_conflict() {
        let mut env = MockEnv::default();
        let address = address(0x44);
        let original_account = account_id(404);
        let conflicting_account = account_id(405);

        set_mapping(address, original_account, &mut env).expect("seed mapping");

        let err = ensure_eoa_mapping(address, conflicting_account, &mut env)
            .expect_err("conflicting forward mapping should fail");
        assert_eq!(err, ERR_ADDRESS_ACCOUNT_CONFLICT);
    }

    #[test]
    fn ensure_eoa_mapping_rejects_reverse_conflict() {
        let mut env = MockEnv::default();
        let account_id = account_id(505);
        let original_address = address(0x55);
        let conflicting_address = address(0x56);

        set_mapping(original_address, account_id, &mut env).expect("seed mapping");

        let err = ensure_eoa_mapping(conflicting_address, account_id, &mut env)
            .expect_err("conflicting reverse mapping should fail");
        assert_eq!(err, ERR_ADDRESS_ACCOUNT_CONFLICT);
    }
}
