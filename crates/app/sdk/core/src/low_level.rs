use crate::encoding::{Decodable, Encodable};
use crate::runtime_api::{
    CreateAccountRequest, CreateAccountResponse, MigrateRequest, RUNTIME_ACCOUNT_ID,
};
use crate::{
    AccountId, Environment, FungibleAsset, InvokableMessage, InvokeRequest, Message, SdkResult,
};

pub fn create_account<Req: Encodable, Resp: Decodable>(
    code_id: String,
    init_msg: &Req,
    funds: Vec<FungibleAsset>,
    env: &mut dyn Environment,
) -> SdkResult<(AccountId, Resp)> {
    let runtime_response: CreateAccountResponse = exec_account(
        RUNTIME_ACCOUNT_ID,
        &CreateAccountRequest {
            code_id,
            init_message: Message::new(init_msg)?,
        },
        funds,
        env,
    )?;

    Ok((
        runtime_response.new_account_id,
        runtime_response.init_response.get()?,
    ))
}

pub fn exec_account<Req: InvokableMessage, Resp: Decodable>(
    target: AccountId,
    request: &Req,
    funds: Vec<FungibleAsset>,
    env: &mut dyn Environment,
) -> SdkResult<Resp> {
    let invoke_request = InvokeRequest::new(request)?;
    env.do_exec(target, &invoke_request, funds)?.get()
}

pub fn query_account<Req: InvokableMessage, Resp: Decodable>(
    target: AccountId,
    request: &Req,
    env: &dyn Environment,
) -> SdkResult<Resp> {
    let invoke_request = InvokeRequest::new(request)?;
    env.do_query(target, &invoke_request)?.get()
}

pub fn migrate_account<Req: InvokableMessage, Resp: Decodable>(
    target: AccountId,
    new_account_code: String,
    migrate_request: &Req,
    funds: Vec<FungibleAsset>,
    env: &mut dyn Environment,
) -> SdkResult<Resp> {
    let invoke_request = InvokeRequest::new(&MigrateRequest {
        account_id: target,
        new_code_id: new_account_code,
        execute_message: InvokeRequest::new(migrate_request)?,
    })?;

    env.do_exec(RUNTIME_ACCOUNT_ID, &invoke_request, funds)?
        .get()
}
