//! Testing utilities for Evolve SDK.

// Testing code - determinism requirements do not apply.
#![allow(clippy::disallowed_types)]

pub mod server_mocks;

use evolve_core::encoding::{Decodable, Encodable};
use evolve_core::storage_api::{
    StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageRemoveResponse,
    StorageSetRequest, StorageSetResponse, STORAGE_ACCOUNT_ID,
};
use evolve_core::{
    AccountId, BlockContext, Environment, EnvironmentQuery, FungibleAsset, InvokableMessage,
    InvokeRequest, InvokeResponse, Message, SdkResult, ERR_UNKNOWN_FUNCTION,
};
use std::collections::HashMap;

type QueryHandler = Box<dyn Fn(AccountId, &InvokeRequest) -> SdkResult<InvokeResponse>>;
type ExecHandler =
    Box<dyn Fn(AccountId, &InvokeRequest, Vec<FungibleAsset>) -> SdkResult<InvokeResponse>>;

pub struct MockEnv {
    whoami: AccountId,
    sender: AccountId,
    funds: Vec<FungibleAsset>,
    state: HashMap<Vec<u8>, Vec<u8>>,
    query_handlers: HashMap<u64, QueryHandler>,
    exec_handlers: HashMap<u64, ExecHandler>,
    block_height: u64,
    block_time: u64,
    unique_counter: u64,
}

impl MockEnv {
    pub fn new(whoami: AccountId, sender: AccountId) -> Self {
        MockEnv {
            whoami,
            sender,
            funds: vec![],
            state: HashMap::new(),
            query_handlers: HashMap::new(),
            exec_handlers: HashMap::new(),
            block_height: 0,
            block_time: 0,
            unique_counter: 0,
        }
    }

    pub fn with_block_height(self, block_height: u64) -> Self {
        Self {
            block_height,
            ..self
        }
    }

    pub fn with_block_time(self, block_time: u64) -> Self {
        Self { block_time, ..self }
    }

    pub fn with_sender(self, sender: AccountId) -> Self {
        Self { sender, ..self }
    }

    pub fn with_funds(self, funds: Vec<FungibleAsset>) -> Self {
        Self { funds, ..self }
    }

    pub fn with_whoami(self, whoami: AccountId) -> Self {
        Self { whoami, ..self }
    }

    pub fn with_query_handler<Req: InvokableMessage + Decodable, Resp: Encodable + Decodable>(
        mut self,
        handler: impl Fn(AccountId, Req) -> SdkResult<Resp> + 'static,
    ) -> Self {
        let wrapped = Box::new(
            move |sender: AccountId, raw_req: &InvokeRequest| -> SdkResult<InvokeResponse> {
                let req = raw_req.get()?;
                let resp = handler(sender, req)?;
                InvokeResponse::new(&resp)
            },
        );

        self.query_handlers
            .insert(Req::FUNCTION_IDENTIFIER, wrapped);
        self
    }

    pub fn with_exec_handler<Req: InvokableMessage + Decodable, Resp: Encodable + Decodable>(
        mut self,
        handler: impl Fn(AccountId, Req, Vec<FungibleAsset>) -> SdkResult<Resp> + 'static,
    ) -> Self {
        let wrapped = Box::new(
            move |sender: AccountId,
                  raw_req: &InvokeRequest,
                  funds: Vec<FungibleAsset>|
                  -> SdkResult<InvokeResponse> {
                let req = raw_req.get()?;
                let resp = handler(sender, req, funds)?;
                InvokeResponse::new(&resp)
            },
        );

        self.exec_handlers.insert(Req::FUNCTION_IDENTIFIER, wrapped);
        self
    }

    fn handle_storage_exec(&mut self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            StorageSetRequest::FUNCTION_IDENTIFIER => {
                let storage_set: StorageSetRequest = request.get()?;

                let mut key = self.whoami.as_bytes();
                key.extend(storage_set.key);

                self.state.insert(key, storage_set.value.as_vec()?);

                Ok(InvokeResponse::new(&StorageSetResponse {})?)
            }
            StorageRemoveRequest::FUNCTION_IDENTIFIER => {
                let storage_remove: StorageRemoveRequest = request.get()?;
                let mut key = self.whoami.as_bytes();
                key.extend(storage_remove.key);
                self.state.remove(&key);
                Ok(InvokeResponse::new(&StorageRemoveResponse {})?)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn handle_storage_query(&mut self, request: &InvokeRequest) -> SdkResult<InvokeResponse> {
        match request.function() {
            StorageGetRequest::FUNCTION_IDENTIFIER => {
                let storage_get: StorageGetRequest = request.get()?;

                let mut key = storage_get.account_id.as_bytes();
                key.extend(storage_get.key);

                let value = self.state.get(&key).cloned();

                Ok(InvokeResponse::new(&StorageGetResponse {
                    value: value.map(Message::from_bytes),
                })?)
            }
            _ => Err(ERR_UNKNOWN_FUNCTION),
        }
    }
}

impl EnvironmentQuery for MockEnv {
    fn whoami(&self) -> AccountId {
        self.whoami
    }

    fn sender(&self) -> AccountId {
        self.sender
    }

    fn funds(&self) -> &[FungibleAsset] {
        self.funds.as_ref()
    }

    fn block(&self) -> BlockContext {
        BlockContext::new(self.block_height, self.block_time)
    }

    fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse> {
        if to == STORAGE_ACCOUNT_ID {
            return self.handle_storage_query(data);
        };

        // find handler for the given msg function
        let function = data.function();
        let handler = self.query_handlers.get(&function);
        match handler {
            Some(handler) => handler(to, data),
            None => Err(ERR_UNKNOWN_FUNCTION),
        }
    }
}

impl Environment for MockEnv {
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse> {
        if to == STORAGE_ACCOUNT_ID {
            return self.handle_storage_exec(data);
        };

        // find handler for the given msg function
        let function = data.function();
        let handler = self.exec_handlers.get(&function);
        match handler {
            Some(handler) => handler(to, data, funds),
            None => Err(ERR_UNKNOWN_FUNCTION),
        }
    }

    fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
        // Mock: events are discarded in test environment
        Ok(())
    }

    fn unique_id(&mut self) -> SdkResult<[u8; 32]> {
        self.unique_counter = self.unique_counter.wrapping_add(1);
        let mut id = [0u8; 32];
        id[0..8].copy_from_slice(&self.unique_counter.to_be_bytes());
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::encoding::{Decodable, Encodable};
    use evolve_core::storage_api::{
        StorageGetRequest, StorageGetResponse, StorageRemoveRequest, StorageRemoveResponse,
        StorageSetRequest, StorageSetResponse, STORAGE_ACCOUNT_ID,
    };
    use evolve_core::{AccountId, Environment, FungibleAsset, InvokeRequest, Message, SdkResult};

    use evolve_core::InvokableMessage;

    /// Helper: Create an `InvokeRequest` from a message.
    fn make_invoke_request<T: InvokableMessage + Encodable>(msg: &T) -> SdkResult<InvokeRequest> {
        InvokeRequest::new(msg)
    }

    #[test]
    fn test_env_configuration() {
        // Create numeric account IDs:
        let whoami = AccountId::new(1234_u128);
        let sender = AccountId::new(5678_u128);

        let funds = vec![
            FungibleAsset {
                asset_id: AccountId::new(55000),
                amount: 123,
            },
            FungibleAsset {
                asset_id: AccountId::new(55100),
                amount: 456,
            },
        ];

        let env = MockEnv::new(whoami, sender).with_funds(funds.clone());

        // Check that the environment is configured correctly
        assert_eq!(
            env.whoami(),
            whoami,
            "The whoami() should match our contract's address."
        );
        assert_eq!(
            env.sender(),
            sender,
            "The sender() should match the address we configured."
        );
        assert_eq!(env.funds(), funds, "Funds should match those we provided.");
    }

    #[test]
    fn test_storage_set_and_get() {
        let whoami = AccountId::new(100_u128);
        let sender = AccountId::new(200_u128);
        let mut env = MockEnv::new(whoami, sender);

        // 1) Prepare a StorageSetRequest
        let set_request = StorageSetRequest {
            key: b"my_key".to_vec(),
            value: Message::from_bytes(b"my_value".to_vec()),
        };

        // 2) Create an InvokeRequest from our message
        let invoke_req =
            make_invoke_request(&set_request).expect("Failed to create invoke request");

        // 3) Execute it against the storage account
        let result = env.do_exec(STORAGE_ACCOUNT_ID, &invoke_req, vec![]);
        assert!(result.is_ok(), "StorageSet should succeed.");
        let _set_response: StorageSetResponse = result.unwrap().get().expect("Decode error");
        // 4) Now, get it back with StorageGetRequest
        let get_request = StorageGetRequest {
            account_id: whoami,
            key: b"my_key".to_vec(),
        };
        let get_invoke_req =
            make_invoke_request(&get_request).expect("Failed to create invoke request");

        let get_result = env.do_query(STORAGE_ACCOUNT_ID, &get_invoke_req);
        assert!(get_result.is_ok(), "StorageGet should succeed.");
        let get_response: StorageGetResponse = get_result.unwrap().get().expect("Decode error");

        // 5) Confirm the retrieved value matches what we stored
        let retrieved = get_response.value.expect("Value should not be None.");
        assert_eq!(
            retrieved.as_bytes().unwrap(),
            b"my_value",
            "Retrieved value should match 'my_value'."
        );
    }

    #[test]
    fn test_storage_remove() {
        let whoami = AccountId::new(300_u128);
        let sender = AccountId::new(400_u128);
        let mut env = MockEnv::new(whoami, sender);

        // First, set something
        let set_request = StorageSetRequest {
            key: b"remove_key".to_vec(),
            value: Message::from_bytes(b"some_value".to_vec()),
        };
        let set_invoke = make_invoke_request(&set_request).unwrap();
        let _ = env
            .do_exec(STORAGE_ACCOUNT_ID, &set_invoke, vec![])
            .expect("set should succeed");

        // Remove it
        let remove_request = StorageRemoveRequest {
            key: b"remove_key".to_vec(),
        };
        let remove_invoke = make_invoke_request(&remove_request).unwrap();
        let remove_result = env.do_exec(STORAGE_ACCOUNT_ID, &remove_invoke, vec![]);
        assert!(remove_result.is_ok(), "StorageRemove should succeed.");
        let _remove_resp: StorageRemoveResponse = remove_result
            .unwrap()
            .get()
            .expect("Decode error for remove response");

        // Confirm it's gone
        let get_request = StorageGetRequest {
            account_id: whoami,
            key: b"remove_key".to_vec(),
        };
        let get_invoke = make_invoke_request(&get_request).unwrap();
        let get_result = env.do_query(STORAGE_ACCOUNT_ID, &get_invoke);
        assert!(
            get_result.is_ok(),
            "StorageGet after removal should succeed."
        );
        let get_response: StorageGetResponse = get_result.unwrap().get().expect("Decode error");
        assert!(
            get_response.value.is_none(),
            "Value should be None after removal."
        );
    }

    #[test]
    fn test_with_query_handler() {
        let whoami = AccountId::new(700_u128);
        let sender = AccountId::new(800_u128);

        // This is a custom request type with a unique FUNCTION_IDENTIFIER
        #[derive(Clone, Debug)]
        struct CustomQueryReq {
            pub data: String,
        }

        impl Encodable for CustomQueryReq {
            fn encode(&self) -> SdkResult<Vec<u8>> {
                Ok(self.data.as_bytes().to_vec())
            }
        }
        impl Decodable for CustomQueryReq {
            fn decode(bytes: &[u8]) -> SdkResult<Self> {
                Ok(CustomQueryReq {
                    data: String::from_utf8(bytes.to_vec()).unwrap_or_default(),
                })
            }
        }
        impl InvokableMessage for CustomQueryReq {
            const FUNCTION_IDENTIFIER: u64 = 12345;
            const FUNCTION_IDENTIFIER_NAME: &'static str = "custom_query_req";
        }

        #[derive(Clone, Debug)]
        struct CustomQueryResp {
            pub data: String,
        }

        impl Encodable for CustomQueryResp {
            fn encode(&self) -> SdkResult<Vec<u8>> {
                Ok(self.data.as_bytes().to_vec())
            }
        }
        impl Decodable for CustomQueryResp {
            fn decode(bytes: &[u8]) -> SdkResult<Self> {
                Ok(CustomQueryResp {
                    data: String::from_utf8(bytes.to_vec()).unwrap_or_default(),
                })
            }
        }

        // Create environment with a custom query handler
        let mut env = MockEnv::new(whoami, sender).with_query_handler(
            // The closure must accept (AccountId, Req) -> SdkResult<Resp>
            |sender_id: AccountId, req: CustomQueryReq| -> SdkResult<CustomQueryResp> {
                // We can do something interesting here, e.g. check the `sender_id` or the `req`.
                let reply = format!(
                    "Hello from {:?}, you said: {}",
                    sender_id.as_bytes(),
                    req.data
                );
                Ok(CustomQueryResp { data: reply })
            },
        );

        // Make a request
        let query = CustomQueryReq {
            data: "this is a query".to_string(),
        };

        let invoke_req = make_invoke_request(&query).unwrap();

        // Perform the query
        let result = env.do_query(AccountId::new(9999_u128), &invoke_req);
        assert!(result.is_ok(), "Query handler should succeed.");

        let response: CustomQueryResp = result.unwrap().get().expect("Decode error");
        assert!(
            response.data.contains("this is a query"),
            "Response should contain the original data."
        );
    }

    #[test]
    fn test_with_exec_handler() {
        let whoami = AccountId::new(900_u128);
        let sender = AccountId::new(1000_u128);

        // We'll reuse the same approach, but define a unique function ID
        #[derive(Clone, Debug)]
        struct CustomExecReq {
            pub val: u64,
        }
        impl Encodable for CustomExecReq {
            fn encode(&self) -> SdkResult<Vec<u8>> {
                Ok(self.val.to_le_bytes().to_vec())
            }
        }
        impl Decodable for CustomExecReq {
            fn decode(bytes: &[u8]) -> SdkResult<Self> {
                let mut arr = [0u8; 8];
                arr.copy_from_slice(bytes);
                let val = u64::from_le_bytes(arr);
                Ok(CustomExecReq { val })
            }
        }
        impl InvokableMessage for CustomExecReq {
            const FUNCTION_IDENTIFIER: u64 = 9999;
            const FUNCTION_IDENTIFIER_NAME: &'static str = "custom_exec_req";
        }

        #[derive(Clone, Debug)]
        struct CustomExecResp {
            pub message: String,
        }
        impl Encodable for CustomExecResp {
            fn encode(&self) -> SdkResult<Vec<u8>> {
                Ok(self.message.as_bytes().to_vec())
            }
        }
        impl Decodable for CustomExecResp {
            fn decode(bytes: &[u8]) -> SdkResult<Self> {
                Ok(CustomExecResp {
                    message: String::from_utf8(bytes.to_vec()).unwrap_or_default(),
                })
            }
        }

        let mut env = MockEnv::new(whoami, sender).with_exec_handler(
            move |sender_id: AccountId, req: CustomExecReq, funds: Vec<FungibleAsset>| {
                // We'll check that we've received the request and funds
                let total_funds: u128 = funds.iter().map(|f| f.amount).sum();
                let response_str = format!(
                    "Got val = {}, from sender = {:?}, total_funds = {}",
                    req.val,
                    sender_id.as_bytes(),
                    total_funds
                );
                Ok(CustomExecResp {
                    message: response_str,
                })
            },
        );

        let custom_exec_req = CustomExecReq { val: 42 };
        let invoke_req = make_invoke_request(&custom_exec_req).unwrap();

        let funds = vec![FungibleAsset {
            asset_id: AccountId::new(9999_u128),
            amount: 100,
        }];

        let result = env.do_exec(AccountId::new(1111_u128), &invoke_req, funds);
        assert!(result.is_ok(), "Execution should succeed.");
        let response: CustomExecResp = result.unwrap().get().expect("Decode error");
        assert!(
            response.message.contains("val = 42"),
            "Response should contain val = 42"
        );
        assert!(
            response.message.contains("total_funds = 100"),
            "Response should mention total_funds = 100"
        );
    }

    #[test]
    fn test_missing_handlers_return_unknown_function() {
        #[derive(Clone, Debug)]
        struct MissingReq;
        impl Encodable for MissingReq {
            fn encode(&self) -> SdkResult<Vec<u8>> {
                Ok(Vec::new())
            }
        }
        impl Decodable for MissingReq {
            fn decode(_bytes: &[u8]) -> SdkResult<Self> {
                Ok(Self)
            }
        }
        impl InvokableMessage for MissingReq {
            const FUNCTION_IDENTIFIER: u64 = 0xDEAD_BEEF;
            const FUNCTION_IDENTIFIER_NAME: &'static str = "missing_req";
        }

        let whoami = AccountId::new(1);
        let sender = AccountId::new(2);
        let mut env = MockEnv::new(whoami, sender);
        let req = make_invoke_request(&MissingReq).unwrap();

        let query_result = env.do_query(AccountId::new(3), &req);
        assert!(matches!(query_result, Err(e) if e == ERR_UNKNOWN_FUNCTION));

        let exec_result = env.do_exec(AccountId::new(3), &req, Vec::new());
        assert!(matches!(exec_result, Err(e) if e == ERR_UNKNOWN_FUNCTION));
    }

    #[test]
    fn test_storage_set_is_namespaced_by_whoami() {
        let owner_a = AccountId::new(10);
        let owner_b = AccountId::new(11);
        let sender = AccountId::new(20);
        let mut env = MockEnv::new(owner_a, sender);

        let set_request = StorageSetRequest {
            key: b"shared_key".to_vec(),
            value: Message::from_bytes(b"owner_a_value".to_vec()),
        };
        let set_invoke = make_invoke_request(&set_request).unwrap();
        env.do_exec(STORAGE_ACCOUNT_ID, &set_invoke, Vec::new())
            .unwrap();

        let get_for_a = StorageGetRequest {
            account_id: owner_a,
            key: b"shared_key".to_vec(),
        };
        let get_for_b = StorageGetRequest {
            account_id: owner_b,
            key: b"shared_key".to_vec(),
        };

        let resp_a: StorageGetResponse = env
            .do_query(
                STORAGE_ACCOUNT_ID,
                &make_invoke_request(&get_for_a).unwrap(),
            )
            .unwrap()
            .get()
            .unwrap();
        let resp_b: StorageGetResponse = env
            .do_query(
                STORAGE_ACCOUNT_ID,
                &make_invoke_request(&get_for_b).unwrap(),
            )
            .unwrap()
            .get()
            .unwrap();

        assert_eq!(resp_a.value.unwrap().as_bytes().unwrap(), b"owner_a_value");
        assert!(resp_b.value.is_none());
    }

    #[test]
    fn test_unique_id_monotonic_counter_prefix() {
        let whoami = AccountId::new(100);
        let sender = AccountId::new(200);
        let mut env = MockEnv::new(whoami, sender);

        let id1 = env.unique_id().unwrap();
        let id2 = env.unique_id().unwrap();

        assert_eq!(&id1[0..8], &1_u64.to_be_bytes());
        assert_eq!(&id2[0..8], &2_u64.to_be_bytes());
        assert!(id1[8..].iter().all(|b| *b == 0));
        assert!(id2[8..].iter().all(|b| *b == 0));
    }
}
