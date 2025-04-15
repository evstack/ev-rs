use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageSetRequest,
    STORAGE_ACCOUNT_ID,
};
use evolve_core::{
    AccountId, Environment, ErrorCode, FungibleAsset, InvokableMessage, InvokeRequest,
    InvokeResponse, Message, SdkResult,
};
use std::collections::HashMap;

pub struct MockEnvironment {
    account_id: AccountId,
    sender_id: AccountId,
    storage: HashMap<Vec<u8>, Message>,
    should_fail: bool, // Simulate environment failure
}

impl MockEnvironment {
    pub(crate) fn storage_remove(&mut self, key: &[u8]) {
        self.storage.remove(key);
    }
}

impl MockEnvironment {
    pub fn new(account_id: u128, sender_id: u128) -> Self {
        Self {
            account_id: AccountId::new(account_id),
            sender_id: AccountId::new(sender_id),
            storage: HashMap::new(),
            should_fail: false,
        }
    }

    pub fn with_failure() -> Self {
        let mut env = Self::new(1, 2);
        env.should_fail = true;
        env
    }
}

impl Environment for MockEnvironment {
    fn whoami(&self) -> AccountId {
        self.account_id
    }

    fn sender(&self) -> AccountId {
        self.sender_id
    }

    fn funds(&self) -> &[FungibleAsset] {
        &[]
    }

    fn do_query(&self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
        if self.should_fail {
            return Err(ErrorCode::new(99, "Simulated environment failure"));
        }
        if to != STORAGE_ACCOUNT_ID {
            return Err(ErrorCode::new(1, "Unsupported account ID"));
        }

        let request = data.get::<StorageGetRequest>()?;
        let value = self.storage.get(&request.key).cloned();
        InvokeResponse::new(&StorageGetResponse { value })
    }

    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        _funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        if self.should_fail {
            return Err(ErrorCode::new(99, "Simulated environment failure"));
        }
        if to != STORAGE_ACCOUNT_ID {
            return Err(ErrorCode::new(1, "Unsupported account ID"));
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
            _ => Err(ErrorCode::new(0, "Invalid storage function identifier")),
        }
    }
}
