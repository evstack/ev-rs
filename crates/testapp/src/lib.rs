mod test_block_exec;

use evolve_block_info::account::BlockInfo;
use evolve_core::{
    AccountId, Environment, FungibleAsset, InvokeRequest, InvokeResponse, SdkResult,
};
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_ns::account::{NameService, NameServiceRef};
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server_core::mocks::MockedAccountsCodeStorage;
use evolve_server_core::{
    AccountsCodeStorage, Block as BlockTrait, PostTxExecution, Transaction, TxValidator, WritableKV,
};
use evolve_stf::Stf;
use evolve_token::account::{Token, TokenRef};

pub struct Tx {
    pub sender: AccountId,
    pub recipient: AccountId,
    pub request: InvokeRequest,
    pub gas_limit: u64,
    pub funds: Vec<FungibleAsset>,
}

impl Transaction for Tx {
    fn sender(&self) -> AccountId {
        self.sender
    }

    fn recipient(&self) -> AccountId {
        self.recipient
    }

    fn request(&self) -> &InvokeRequest {
        &self.request
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn funds(&self) -> &[FungibleAsset] {
        self.funds.as_slice()
    }
}

pub struct Block {
    pub height: u64,
    pub txs: Vec<Tx>,
}

impl BlockTrait<Tx> for Block {
    fn height(&self) -> u64 {
        self.height
    }

    fn txs(&self) -> &[Tx] {
        self.txs.as_slice()
    }
}

pub struct TxValidatorHandler;

impl TxValidator<Tx> for TxValidatorHandler {
    fn validate_tx(_tx: &Tx, _env: &mut dyn Environment) -> SdkResult<()> {
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

pub type TestAppStf =
Stf<Tx, Block, SchedulerBeginBlocker, TxValidatorHandler, SchedulerEndBlocker, NoOpPostTx>;

/// List of accounts installed.
pub fn account_codes() -> impl AccountsCodeStorage {
    let mut codes = MockedAccountsCodeStorage::new();

    codes.add_code(Token::new()).unwrap();
    codes.add_code(NameService::new()).unwrap();
    codes.add_code(Scheduler::new()).unwrap();
    codes.add_code(BlockInfo::new()).unwrap();

    codes
}

pub fn do_genesis<S: WritableKV, A: AccountsCodeStorage>(
    storage: &mut S,
    codes: &mut A,
) -> SdkResult<()> {
    let (_, changes) = TestAppStf::sudo(storage, codes, |env| {
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
            vec![],
            env,
        )?
            .0;
        // Create scheduler
        let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
        // Update well known names in the name service.
        ns_acc.updates_names(
            vec![
                ("scheduler".to_string(), scheduler_acc.0),
                ("atom".to_string(), atom.0),
            ],
            env,
        )?;
        Ok(())
    })?;

    storage.apply_changes(changes)?;

    Ok(())
}
