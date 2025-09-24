pub mod eoa;
pub mod sim_testing;
pub mod testing;
pub mod types;

use crate::eoa::eoa_account::{EoaAccount, EoaAccountRef};
pub use crate::types::TestTx;
use borsh::BorshDeserialize;
use evolve_authentication::AuthenticationTxValidator;
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::{
    AccountId, BlockContext, Environment, InvokeResponse, ReadonlyKV, SdkResult, ERR_ENCODING,
};
use evolve_escrow::escrow::Escrow;
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_poa::account::{Poa, PoaRef};
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server::Block;
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::{Stf, StorageGasConfig};
use evolve_stf_traits::{
    AccountsCodeStorage, PostTxExecution, TxDecoder, WritableAccountsCodeStorage,
};
use evolve_token::account::{Token, TokenRef};

pub const MINTER: AccountId = AccountId::new(100_002);

pub struct NoOpPostTx;

impl PostTxExecution<TestTx> for NoOpPostTx {
    fn after_tx_executed(
        _tx: &TestTx,
        _gas_consumed: u64,
        _tx_result: SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        Ok(())
    }
}

pub type CustomStf = Stf<
    TestTx,
    Block<TestTx>,
    SchedulerBeginBlocker,
    AuthenticationTxValidator<TestTx>,
    SchedulerEndBlocker,
    NoOpPostTx,
>;

pub const PLACEHOLDER_ACCOUNT: AccountId = AccountId::new(u128::MAX);

/// Default gas configuration for the test application.
pub fn default_gas_config() -> StorageGasConfig {
    StorageGasConfig {
        storage_get_charge: 10,
        storage_set_charge: 10,
        storage_remove_charge: 10,
    }
}

pub fn build_stf(gas_config: StorageGasConfig, scheduler_id: AccountId) -> CustomStf {
    CustomStf::new(
        SchedulerBeginBlocker::new(scheduler_id),
        SchedulerEndBlocker::new(scheduler_id),
        AuthenticationTxValidator::new(),
        NoOpPostTx,
        gas_config,
    )
}

#[derive(Clone)]
pub struct TxDecoderImpl;

impl TxDecoder<TestTx> for TxDecoderImpl {
    fn decode(&self, bytes: &mut &[u8]) -> SdkResult<TestTx> {
        TestTx::deserialize(bytes).map_err(|_| ERR_ENCODING)
    }
}

/// List of accounts installed.
pub fn install_account_codes(codes: &mut impl WritableAccountsCodeStorage) {
    codes.add_code(Token::new()).unwrap();
    codes.add_code(Scheduler::new()).unwrap();
    codes.add_code(Poa::new()).unwrap();
    codes.add_code(Escrow::new()).unwrap();
    codes.add_code(EoaAccount::new()).unwrap();
}

#[derive(Clone, Copy, Debug)]
pub struct GenesisAccounts {
    pub alice: AccountId,
    pub bob: AccountId,
    pub atom: AccountId,
    pub scheduler: AccountId,
    pub poa: AccountId,
}

/// Genesis initialization logic - can be called from system_exec.
pub fn do_genesis_inner(env: &mut dyn Environment) -> SdkResult<GenesisAccounts> {
    // create EOAs
    let alice_account = EoaAccountRef::initialize(env)?.0;
    let bob_account = EoaAccountRef::initialize(env)?.0;

    // Create atom token
    let atom = TokenRef::initialize(
        FungibleAssetMetadata {
            name: "uatom".to_string(),
            symbol: "ATOM".to_string(),
            decimals: 6,
            icon_url: "https://lol.wtf".to_string(),
            description: "The atom coin".to_string(),
        },
        vec![(alice_account.0, 1000), (bob_account.0, 2000)],
        Some(MINTER),
        env,
    )?
    .0;

    // Create scheduler (no begin blockers needed for block info anymore)
    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;

    // Create poa
    let poa = PoaRef::initialize(RUNTIME_ACCOUNT_ID, scheduler_acc.0, vec![], env)?.0;

    // Update scheduler's account's list.
    scheduler_acc.update_begin_blockers(vec![poa.0], env)?;

    Ok(GenesisAccounts {
        alice: alice_account.0,
        bob: bob_account.0,
        atom: atom.0,
        scheduler: scheduler_acc.0,
        poa: poa.0,
    })
}

pub fn do_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
    stf: &CustomStf,
    codes: &'a A,
    storage: &'a S,
) -> SdkResult<(ExecutionState<'a, S>, GenesisAccounts)> {
    let genesis_height = 0;
    let genesis_time_unix_ms = 0;
    let genesis_block = BlockContext::new(genesis_height, genesis_time_unix_ms);

    let (accounts, state) = stf.system_exec(storage, codes, genesis_block, do_genesis_inner)?;

    Ok((state, accounts))
}
