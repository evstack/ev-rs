pub mod storage;
#[cfg(test)]
mod test_block_exec;
pub mod types;

use crate::types::Tx;
use borsh::{BorshDeserialize};
use evolve_block_info::account::BlockInfo;
use evolve_cometbft::types::TendermintBlock;
use evolve_core::runtime_api::RUNTIME_ACCOUNT_ID;
use evolve_core::{AccountId, Environment, InvokeResponse, ReadonlyKV, SdkResult, ERR_ENCODING};
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_gas::account::{GasService, GasServiceRef, StorageGasConfig};
use evolve_ns::account::{NameService, NameServiceRef};
use evolve_poa::account::{Poa, PoaRef};
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server_core::{
    AccountsCodeStorage, PostTxExecution, TxDecoder, TxValidator, WritableAccountsCodeStorage,
};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::Stf;
use evolve_token::account::{Token, TokenRef};

pub const ALICE: AccountId = AccountId::new(100_000);
pub const BOB: AccountId = AccountId::new(100_001);

pub struct TxValidatorHandler;

impl TxValidator<Tx> for TxValidatorHandler {
    fn validate_tx(&self, _tx: &Tx, _env: &mut dyn Environment) -> SdkResult<()> {
        Ok(())
    }
}

pub struct NoOpPostTx;

impl PostTxExecution<Tx> for NoOpPostTx {
    fn after_tx_executed(
        _tx: &Tx,
        _gas_consumed: u64,
        _tx_result: SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        Ok(())
    }
}

pub type CustomStf = Stf<
    Tx,
    TendermintBlock<Tx>,
    SchedulerBeginBlocker,
    TxValidatorHandler,
    SchedulerEndBlocker,
    NoOpPostTx,
>;

pub const STF: CustomStf = CustomStf::new(
    SchedulerBeginBlocker,
    SchedulerEndBlocker,
    TxValidatorHandler,
    NoOpPostTx,
);

#[derive(Clone)]
pub struct TxDecoderImpl;

impl TxDecoder<Tx> for TxDecoderImpl {
    fn decode(&self, bytes: &mut &[u8]) -> SdkResult<Tx> {
        Tx::deserialize(bytes).map_err(|_| ERR_ENCODING)
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
}

pub fn do_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
    stf: &CustomStf,
    storage: &'a S,
    codes: &'a A,
) -> SdkResult<ExecutionState<'a, S>> {
    let (_, state) = stf.sudo(storage, codes, |env| {
        // Create name service account: this must be done first
        let ns_acc = NameServiceRef::initialize(vec![], env)?.0;
        // Create atom token
        let atom = TokenRef::initialize(
            FungibleAssetMetadata {
                name: "uatom".to_string(),
                symbol: "ATOM".to_string(),
                decimals: 6,
                icon_url: "https://lol.wtf".to_string(),
                description: "The atom coin".to_string(),
            },
            vec![(ALICE, 1000), (BOB, 2000)],
            None,
            env,
        )?
        .0;
        // Create scheduler
        let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
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
                ("scheduler".to_string(), scheduler_acc.0),
                ("atom".to_string(), atom.0),
                ("gas".to_string(), gas_service_acc.0),
                ("poa".to_string(), poa.0),
            ],
            env,
        )?;

        // Update scheduler's account's list.
        scheduler_acc.update_begin_blockers(vec![poa.0], env)?;
        Ok(())
    })?;

    Ok(state)
}
