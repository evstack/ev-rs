use evolve_server_core::{AccountsCodeStorage, Transaction};
use std::marker::PhantomData;
use evolve_core::{AccountId, Context, InvokeRequest, InvokeResponse, Invoker, SdkResult};

pub struct Stf<Tx>(PhantomData<Tx>);

impl<T> Stf<T> {
    fn apply_tx<A: AccountsCodeStorage<ExecCtx>>(account_storage: A) {

    }
}

struct ExecCtx {

}

impl Invoker for ExecCtx {
    fn do_query(&self, ctx: &Context, to: AccountId, data: InvokeRequest) -> SdkResult<InvokeResponse> {
        todo!()
    }

    fn do_exec(&mut self, ctx: &mut Context, to: AccountId, data: InvokeRequest) -> SdkResult<InvokeResponse> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
    }
}
