use evolve_core::{AccountCode, AccountId, Context, InvokeRequest, InvokeResponse, Invoker, Message, SdkResult};
use evolve_server_core::Transaction;
use crate::Stf;

struct Echo;

impl<I: Invoker> AccountCode<I> for Echo {
    fn identifier(&self) -> String {
        "Echo".to_string()
    }

    fn init(
        &self,
        _invoker: &mut I,
        _ctx: &mut Context,
        _request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let resp = "echo_init".to_string().as_bytes().to_vec();
        Ok(InvokeResponse::new(Message::from(resp)))
    }

    fn execute(
        &self,
        invoker: &mut I,
        ctx: &mut Context,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let mut resp = "echo_execute".to_string().as_bytes().to_vec();
        resp.extend_from_slice(request.bytes());
        Ok(InvokeResponse::new(Message::from(resp)))
    }

    fn query(
        &self,
        invoker: &I,
        ctx: &Context,
        request: InvokeRequest,
    ) -> SdkResult<InvokeResponse> {
        let mut resp = "echo_response".to_string().as_bytes().to_vec();
        resp.extend_from_slice(request.bytes());
        Ok(InvokeResponse::new(Message::from(resp)))
    }
}

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

#[test]
fn success() {

}