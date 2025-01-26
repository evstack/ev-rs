use crate::test::echo_account::{Echo, InitRequest};
use crate::Stf;
use evolve_core::encoding::Encodable;
use evolve_core::{AccountCode, AccountId, Invoker, Message};
use evolve_server_core::mocks::MockedAccountsCodeStorage;
use evolve_server_core::{AccountsCodeStorage, Transaction};
use std::collections::HashMap;

struct MockTx;

impl Transaction for MockTx {
    fn sender(&self) -> AccountId {
        todo!()
    }

    fn recipient(&self) -> AccountId {
        todo!()
    }

    fn message(&self) -> Message {
        todo!()
    }

    fn gas_limit(&self) -> u64 {
        todo!()
    }
}

type TestStf = Stf<MockTx>;

mod echo_account {
    use borsh::{BorshDeserialize, BorshSerialize};
    use evolve_core::encoding::Encodable;
    use evolve_core::{
        AccountCode, Context, InvokeRequest, InvokeResponse, Invoker, Message, SdkResult,
    };

    pub(crate) struct Echo {
        map: evolve_collections::Map<(), String>,
    }

    impl Echo {
        pub(crate) fn new() -> Self {
            Echo {
                map: evolve_collections::Map::new(0),
            }
        }
    }

    #[derive(BorshDeserialize, BorshSerialize)]
    pub(crate) struct InitRequest {
        pub(crate) echo_msg: String,
    }

    #[derive(BorshSerialize, BorshDeserialize)]
    pub(crate) struct InitResponse {
        pub(crate) echo_msg: String,
    }

    impl AccountCode for Echo {
        fn identifier(&self) -> String {
            "echo".to_string()
        }

        fn init(
            &self,
            invoker: &mut dyn Invoker,
            ctx: &mut Context,
            request: InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            let request: InitRequest = request.decode()?;
            let response = Message::from(
                InitResponse {
                    echo_msg: request.echo_msg,
                }
                .encode()?,
            );

            self.map.set(ctx, invoker, (), "hh".to_string())?;

            Ok(InvokeResponse::new(response))
        }

        fn execute(
            &self,
            invoker: &mut dyn Invoker,
            ctx: &mut Context,
            request: InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            todo!()
        }

        fn query(
            &self,
            invoker: &dyn Invoker,
            ctx: &Context,
            request: InvokeRequest,
        ) -> SdkResult<InvokeResponse> {
            todo!()
        }
    }
}

#[test]
fn success() {
    let mut account_codes = MockedAccountsCodeStorage::new();

    let echo = Echo::new();

    account_codes.add_code(echo).unwrap();

    let mut storage: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();

    let echo_id = TestStf::create_account(
        &mut storage,
        &mut account_codes,
        AccountId::new(100u128),
        "echo".to_string(),
        Message::from(
            InitRequest {
                echo_msg: "hi".to_string(),
            }
            .encode()
            .unwrap(),
        ),
    )
    .unwrap();

    println!("{:?}", echo_id)
}
