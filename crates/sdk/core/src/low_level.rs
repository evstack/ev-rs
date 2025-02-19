use crate::encoding::{Decodable, Encodable};
use crate::well_known::{
    CreateAccountRequest, CreateAccountResponse, RUNTIME_ACCOUNT_ID,
    RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER,
};
use crate::{AccountId, Environment, FungibleAsset, InvokeRequest, Message, SdkResult};

pub fn create_account<Req: Encodable, Resp: Decodable>(
    code_id: String,
    init_msg: &Req,
    funds: Vec<FungibleAsset>,
    env: &mut dyn Environment,
) -> SdkResult<(AccountId, Resp)> {
    let runtime_response: CreateAccountResponse = exec_account(
        RUNTIME_ACCOUNT_ID,
        RUNTIME_CREATE_ACCOUNT_FUNCTION_IDENTIFIER,
        &CreateAccountRequest {
            code_id,
            init_message: Message::from(init_msg.encode()?),
        },
        funds,
        env,
    )?;

    let account_response = Resp::decode(runtime_response.init_response.inner.as_bytes())?;

    Ok((runtime_response.new_account_id, account_response))
}

pub fn exec_account<Req: Encodable, Resp: Decodable>(
    target: AccountId,
    function_identifier: u64,
    request: &Req,
    funds: Vec<FungibleAsset>,
    env: &mut dyn Environment,
) -> SdkResult<Resp> {
    let invoke_request = InvokeRequest::new(function_identifier, Message::from(request.encode()?));
    env.do_exec(target, invoke_request, funds)?
        .try_into_decodable()
}

pub fn query_account<Req: Encodable, Resp: Decodable>(
    target: AccountId,
    function_identifier: u64,
    request: &Req,
    env: &dyn Environment,
) -> SdkResult<Resp> {
    let invoke_request = InvokeRequest::new(function_identifier, Message::from(request.encode()?));
    env.do_query(target, invoke_request)?.try_into_decodable()
}
