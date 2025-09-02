pub mod block;
pub mod eoa;
pub mod storage;
pub mod testing;
pub mod types;

use crate::block::TestBlock;
use crate::eoa::eoa_account::{EoaAccount, EoaAccountRef};
pub use crate::types::TestTx;
use borsh::BorshDeserialize;
use evolve_authentication::AuthenticationTxValidator;
use evolve_block_info::account::{BlockInfo, BlockInfoRef};
use evolve_core::events_api::EVENT_HANDLER_ACCOUNT_ID;
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::storage_api::STORAGE_ACCOUNT_ID;
use evolve_core::unique_api::UNIQUE_HANDLER_ACCOUNT_ID;
use evolve_core::{AccountId, Environment, InvokeResponse, ReadonlyKV, SdkResult, ERR_ENCODING};
use evolve_escrow::escrow::Escrow;
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_gas::account::{GasService, GasServiceRef, StorageGasConfig};
use evolve_ns::account::{NameService, NameServiceRef};
use evolve_poa::account::{Poa, PoaRef};
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server_core::{
    AccountsCodeStorage, PostTxExecution, TxDecoder, WritableAccountsCodeStorage,
};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::Stf;
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
    TestBlock<TestTx>,
    SchedulerBeginBlocker,
    AuthenticationTxValidator<TestTx>,
    SchedulerEndBlocker,
    NoOpPostTx,
>;

pub const STF: CustomStf = CustomStf::new(
    SchedulerBeginBlocker,
    SchedulerEndBlocker,
    AuthenticationTxValidator::new(),
    NoOpPostTx,
);

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
    codes.add_code(NameService::new()).unwrap();
    codes.add_code(Scheduler::new()).unwrap();
    codes.add_code(GasService::new()).unwrap();
    codes.add_code(BlockInfo::new()).unwrap();
    codes.add_code(Poa::new()).unwrap();
    codes.add_code(Escrow::new()).unwrap();
    codes.add_code(EoaAccount::new()).unwrap();
}

pub fn do_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
    stf: &CustomStf,
    codes: &'a A,
    storage: &'a S,
) -> SdkResult<ExecutionState<'a, S>> {
    let genesis_height = 0; // TODO
    let genesis_time_unix_ms = 0; // TODO

    let (_, state) = stf.system_exec(storage, codes, genesis_height, |env| {
        // Create name service account: this must be done first
        let ns_acc = NameServiceRef::initialize(vec![], env)?.0;

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

        // Create block info service
        let block_info = BlockInfoRef::initialize(genesis_height, genesis_time_unix_ms, env)?.0;

        // Create scheduler
        let scheduler_acc = SchedulerRef::initialize(vec![block_info.0], vec![], env)?.0;
        // Create gas config service.
        let gas_service_acc = GasServiceRef::initialize(
            StorageGasConfig {
                storage_get_charge: 10,
                storage_set_charge: 10,
                storage_remove_charge: 10,
            },
            env,
        )?
        .0;
        // Create poa
        let poa = PoaRef::initialize(RUNTIME_ACCOUNT_ID, scheduler_acc.0, vec![], env)?.0;

        // Update well known names in the name service.
        ns_acc.updates_names(
            vec![
                ("runtime".to_string(), RUNTIME_ACCOUNT_ID),
                ("storage".to_string(), STORAGE_ACCOUNT_ID),
                ("events".to_string(), EVENT_HANDLER_ACCOUNT_ID),
                ("unique".to_string(), UNIQUE_HANDLER_ACCOUNT_ID),
                ("scheduler".to_string(), scheduler_acc.0),
                ("atom".to_string(), atom.0),
                ("gas".to_string(), gas_service_acc.0),
                ("poa".to_string(), poa.0),
                ("block_info".to_string(), block_info.0),
            ],
            env,
        )?;

        // Update scheduler's account's list.
        scheduler_acc.update_begin_blockers(vec![poa.0], env)?;
        Ok(())
    })?;

    Ok(state)
}
