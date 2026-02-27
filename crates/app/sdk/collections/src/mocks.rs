//! Mock environment for testing collections.

// Testing code - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]

use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageSetRequest,
    STORAGE_ACCOUNT_ID,
};
use evolve_core::{
    AccountId, BlockContext, Environment, EnvironmentQuery, ErrorCode, FungibleAsset,
    InvokableMessage, InvokeRequest, InvokeResponse, Message, SdkResult,
};
use std::collections::HashMap;

pub struct MockEnvironment {
    account_id: AccountId,
    sender_id: AccountId,
    storage: HashMap<Vec<u8>, Message>,
    should_fail: bool, // Simulate environment failure
    unique_counter: u64,
}

impl MockEnvironment {
    pub(crate) fn storage_remove(&mut self, key: &[u8]) {
        self.storage.remove(key);
    }
}

impl MockEnvironment {
    pub fn new(account_id: u64, sender_id: u64) -> Self {
        Self {
            account_id: AccountId::from_u64(account_id),
            sender_id: AccountId::from_u64(sender_id),
            storage: HashMap::new(),
            should_fail: false,
            unique_counter: 0,
        }
    }

    pub fn with_failure() -> Self {
        let mut env = Self::new(1, 2);
        env.should_fail = true;
        env
    }
}

impl EnvironmentQuery for MockEnvironment {
    fn whoami(&self) -> AccountId {
        self.account_id
    }

    fn sender(&self) -> AccountId {
        self.sender_id
    }

    fn funds(&self) -> &[FungibleAsset] {
        &[]
    }

    fn block(&self) -> BlockContext {
        BlockContext::default()
    }

    fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
        if self.should_fail {
            return Err(ErrorCode::new(99));
        }
        if to != STORAGE_ACCOUNT_ID {
            return Err(ErrorCode::new(1));
        }

        let request = data.get::<StorageGetRequest>()?;
        let value = self.storage.get(&request.key).cloned();
        InvokeResponse::new(&StorageGetResponse { value })
    }
}

impl Environment for MockEnvironment {
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        _funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        if self.should_fail {
            return Err(ErrorCode::new(99));
        }
        if to != STORAGE_ACCOUNT_ID {
            return Err(ErrorCode::new(1));
        }

        match data.function() {
            StorageSetRequest::FUNCTION_IDENTIFIER => {
                // Parse StorageSetRequest
                let request = data.get::<StorageSetRequest>()?;
                // Insert or overwrite the key in our mock storage
                self.storage.insert(request.key, request.value);
                Ok(InvokeResponse::new(&())?)
            }
            StorageRemoveRequest::FUNCTION_IDENTIFIER => {
                // Parse StorageRemoveRequest
                let request = data.get::<StorageRemoveRequest>()?;
                // Remove the key from our mock storage (if it exists)
                self.storage.remove(&request.key);
                Ok(InvokeResponse::new(&())?)
            }
            _ => Err(ErrorCode::new(0)),
        }
    }

    fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
        // Mock: events are discarded in test environment
        Ok(())
    }

    fn unique_id(&mut self) -> SdkResult<[u8; 32]> {
        // Mock: return a simple incrementing ID
        self.unique_counter = self.unique_counter.wrapping_add(1);
        let mut id = [0u8; 32];
        id[0..8].copy_from_slice(&self.unique_counter.to_be_bytes());
        Ok(id)
    }
}
