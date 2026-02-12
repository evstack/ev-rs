use crate::Simulator;
use evolve_core::{AccountId, BlockContext, Environment, InvokeResponse, SdkResult};
use evolve_stf::results::BlockResult;
use evolve_stf::Stf;
use evolve_stf_traits::{
    AccountsCodeStorage, BeginBlocker, Block as BlockTrait, EndBlocker, PostTxExecution,
    Transaction, TxValidator,
};

pub fn system_exec_and_commit<Tx, Block, Begin, Validator, End, Post, Codes, R>(
    sim: &mut Simulator,
    stf: &Stf<Tx, Block, Begin, Validator, End, Post>,
    codes: &Codes,
    block: BlockContext,
    action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
) -> SdkResult<R>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    Begin: BeginBlocker<Block>,
    Validator: TxValidator<Tx>,
    End: EndBlocker,
    Post: PostTxExecution<Tx>,
    Codes: AccountsCodeStorage,
{
    let storage = sim.readonly_kv();
    let (resp, state) = stf.system_exec(&storage, codes, block, action)?;
    sim.apply_state_changes(state.into_changes()?)?;
    Ok(resp)
}

pub fn system_exec_genesis_and_commit<Tx, Block, Begin, Validator, End, Post, Codes, R>(
    sim: &mut Simulator,
    stf: &Stf<Tx, Block, Begin, Validator, End, Post>,
    codes: &Codes,
    action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
) -> SdkResult<R>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    Begin: BeginBlocker<Block>,
    Validator: TxValidator<Tx>,
    End: EndBlocker,
    Post: PostTxExecution<Tx>,
    Codes: AccountsCodeStorage,
{
    system_exec_and_commit(sim, stf, codes, BlockContext::new(0, 0), action)
}

pub fn system_exec_as_and_commit<Tx, Block, Begin, Validator, End, Post, Codes, R>(
    sim: &mut Simulator,
    stf: &Stf<Tx, Block, Begin, Validator, End, Post>,
    codes: &Codes,
    block: BlockContext,
    impersonate: AccountId,
    action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
) -> SdkResult<R>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    Begin: BeginBlocker<Block>,
    Validator: TxValidator<Tx>,
    End: EndBlocker,
    Post: PostTxExecution<Tx>,
    Codes: AccountsCodeStorage,
{
    let storage = sim.readonly_kv();
    let (resp, state) = stf.system_exec_as(&storage, codes, block, impersonate, action)?;
    sim.apply_state_changes(state.into_changes()?)?;
    Ok(resp)
}

pub fn apply_block_and_commit<Tx, Block, Begin, Validator, End, Post, Codes>(
    sim: &mut Simulator,
    stf: &Stf<Tx, Block, Begin, Validator, End, Post>,
    codes: &Codes,
    block: &Block,
) -> BlockResult
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    Begin: BeginBlocker<Block>,
    Validator: TxValidator<Tx>,
    End: EndBlocker,
    Post: PostTxExecution<Tx>,
    Codes: AccountsCodeStorage,
{
    let storage = sim.readonly_kv();
    let (result, state) = stf.apply_block(&storage, codes, block);
    sim.apply_state_changes(state.into_changes().expect("state changes"))
        .expect("apply state changes");
    result
}

pub fn query_stf<Tx, Block, Begin, Validator, End, Post, Codes, Req>(
    sim: &Simulator,
    stf: &Stf<Tx, Block, Begin, Validator, End, Post>,
    codes: &Codes,
    target: AccountId,
    req: &Req,
    gc: evolve_stf::gas::GasCounter,
) -> SdkResult<InvokeResponse>
where
    Tx: Transaction,
    Block: BlockTrait<Tx>,
    Begin: BeginBlocker<Block>,
    Validator: TxValidator<Tx>,
    End: EndBlocker,
    Post: PostTxExecution<Tx>,
    Codes: AccountsCodeStorage,
    Req: evolve_core::InvokableMessage,
{
    let storage = sim.readonly_kv();
    stf.query(&storage, codes, target, req, gc)
}

pub fn current_block_context(sim: &Simulator) -> BlockContext {
    BlockContext::new(sim.time().block_height(), 0)
}

pub fn run_block_iterations<T, R>(
    context: &mut T,
    num_blocks: u64,
    mut current_height: impl FnMut(&T) -> u64,
    mut run_block: impl FnMut(u64, &mut T) -> R,
    mut advance_block: impl FnMut(&mut T),
) -> Vec<R> {
    let mut results = Vec::with_capacity(num_blocks as usize);
    for _ in 0..num_blocks {
        let height = current_height(context);
        let result = run_block(height, context);
        results.push(result);
        advance_block(context);
    }
    results
}
