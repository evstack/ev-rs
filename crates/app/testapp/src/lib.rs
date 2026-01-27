pub mod eth_eoa;
pub mod sim_testing;
pub mod testing;
pub mod types;

use crate::eth_eoa::eth_eoa_account::{EthEoaAccount, EthEoaAccountRef};
pub use crate::types::TestTx;
use borsh::BorshDeserialize;
use evolve_authentication::AuthenticationTxValidator;
use evolve_core::{
    AccountId, BlockContext, Environment, InvokeResponse, ReadonlyKV, SdkResult, ERR_ENCODING,
};
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_mempool::TxContext;
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server::Block;
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::{Stf, StorageGasConfig};
use evolve_stf_traits::{
    AccountsCodeStorage, PostTxExecution, TxDecoder, WritableAccountsCodeStorage,
};
use evolve_token::account::{Token, TokenRef};
use evolve_tx::address_to_account_id;

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

pub struct MempoolNoOpPostTx;

impl PostTxExecution<TxContext> for MempoolNoOpPostTx {
    fn after_tx_executed(
        _tx: &TxContext,
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

/// STF type for TxContext (Ethereum transactions via mempool).
pub type MempoolStf = Stf<
    TxContext,
    Block<TxContext>,
    SchedulerBeginBlocker,
    AuthenticationTxValidator<TxContext>,
    SchedulerEndBlocker,
    MempoolNoOpPostTx,
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

/// Build an STF for TxContext (Ethereum transactions).
pub fn build_mempool_stf(gas_config: StorageGasConfig, scheduler_id: AccountId) -> MempoolStf {
    MempoolStf::new(
        SchedulerBeginBlocker::new(scheduler_id),
        SchedulerEndBlocker::new(scheduler_id),
        AuthenticationTxValidator::new(),
        MempoolNoOpPostTx,
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
    codes.add_code(EthEoaAccount::new()).unwrap();
}

#[derive(Clone, Copy, Debug, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct GenesisAccounts {
    pub alice: AccountId,
    pub bob: AccountId,
    pub atom: AccountId,
    pub scheduler: AccountId,
}

/// Genesis initialization logic - can be called from system_exec.
pub fn do_genesis_inner(env: &mut dyn Environment) -> SdkResult<GenesisAccounts> {
    // create EOAs with dummy eth addresses for testing
    let alice_eth_addr: [u8; 20] = [0xAA; 20];
    let bob_eth_addr: [u8; 20] = [0xBB; 20];
    let alice_account = EthEoaAccountRef::initialize(alice_eth_addr, env)?.0;
    let bob_account = EthEoaAccountRef::initialize(bob_eth_addr, env)?.0;

    // Create atom token
    let atom = TokenRef::initialize(
        FungibleAssetMetadata {
            name: "evolve".to_string(),
            symbol: "ev".to_string(),
            decimals: 6,
            icon_url: "https://lol.wtf".to_string(),
            description: "The evolve coin".to_string(),
        },
        vec![(alice_account.0, 1000), (bob_account.0, 2000)],
        Some(MINTER),
        env,
    )?
    .0;

    // Create scheduler (no begin blockers needed for block info anymore)
    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;

    // Update scheduler's account's list.
    scheduler_acc.update_begin_blockers(vec![], env)?;

    Ok(GenesisAccounts {
        alice: alice_account.0,
        bob: bob_account.0,
        atom: atom.0,
        scheduler: scheduler_acc.0,
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

/// Genesis accounts for Ethereum-compatible testing.
#[derive(Clone, Copy, Debug)]
pub struct EthGenesisAccounts {
    /// Alice's account ID (derived from Ethereum address).
    pub alice: AccountId,
    /// Alice's Ethereum address.
    pub alice_eth_address: [u8; 20],
    /// Bob's account ID (derived from Ethereum address).
    pub bob: AccountId,
    /// Bob's Ethereum address.
    pub bob_eth_address: [u8; 20],
    /// Evolve token account.
    pub evolve: AccountId,
    /// Scheduler account.
    pub scheduler: AccountId,
}

/// Create Ethereum EOA accounts at specific addresses.
///
/// NOTE: This function expects the ETH EOA accounts to already be pre-registered
/// in storage (code identifier, nonce, eth_address). It only creates the scheduler.
/// Use the test helper `AsyncMockStorage::register_account_code` and
/// `AsyncMockStorage::init_eth_eoa_storage` to pre-populate account data.
pub fn do_eth_genesis_inner(
    alice_eth_address: [u8; 20],
    bob_eth_address: [u8; 20],
    env: &mut dyn Environment,
) -> SdkResult<EthGenesisAccounts> {
    use alloy_primitives::Address;

    // Convert Ethereum addresses to AccountIds
    // (accounts should already be registered in storage)
    let alice_id = address_to_account_id(Address::from(alice_eth_address));
    let bob_id = address_to_account_id(Address::from(bob_eth_address));

    // Create evolve token
    let evolve = TokenRef::initialize(
        FungibleAssetMetadata {
            name: "evolve".to_string(),
            symbol: "ev".to_string(),
            decimals: 6,
            icon_url: "https://lol.wtf".to_string(),
            description: "The evolve coin".to_string(),
        },
        vec![(alice_id, 1000), (bob_id, 2000)],
        Some(MINTER),
        env,
    )?
    .0;

    // Create scheduler
    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
    scheduler_acc.update_begin_blockers(vec![], env)?;

    Ok(EthGenesisAccounts {
        alice: alice_id,
        alice_eth_address,
        bob: bob_id,
        bob_eth_address,
        evolve: evolve.0,
        scheduler: scheduler_acc.0,
    })
}

/// Execute genesis for Ethereum accounts.
pub fn do_eth_genesis<'a, S: ReadonlyKV, A: AccountsCodeStorage>(
    stf: &MempoolStf,
    codes: &'a A,
    storage: &'a S,
    alice_eth_address: [u8; 20],
    bob_eth_address: [u8; 20],
) -> SdkResult<(ExecutionState<'a, S>, EthGenesisAccounts)> {
    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf.system_exec(storage, codes, genesis_block, |env| {
        do_eth_genesis_inner(alice_eth_address, bob_eth_address, env)
    })?;

    Ok((state, accounts))
}
