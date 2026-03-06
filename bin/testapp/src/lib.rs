pub mod eth_eoa;
pub mod genesis_config;
pub mod sim_testing;

use crate::eth_eoa::eth_eoa_account::{EthEoaAccount, EthEoaAccountRef};
use crate::genesis_config::{EvdGenesisConfig, EvdGenesisResult};
use evolve_authentication::AuthenticationTxValidator;
use evolve_core::{AccountId, BlockContext, Environment, InvokeResponse, ReadonlyKV, SdkResult};
use evolve_fungible_asset::FungibleAssetMetadata;
use evolve_node::{GenesisOutput, HasTokenAccountId};
use evolve_scheduler::scheduler_account::{Scheduler, SchedulerRef};
use evolve_scheduler::server::{SchedulerBeginBlocker, SchedulerEndBlocker};
use evolve_server::Block;
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::{Stf, StorageGasConfig};
use evolve_stf_traits::{AccountsCodeStorage, PostTxExecution, WritableAccountsCodeStorage};
use evolve_storage::Storage;
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::{Token, TokenRef};
use evolve_tx_eth::TxContext;
use evolve_tx_eth::{register_runtime_contract_account, resolve_or_create_eoa_account};

pub const MINTER: AccountId = AccountId::from_u64(100002);

pub struct MempoolNoOpPostTx;

impl PostTxExecution<TxContext> for MempoolNoOpPostTx {
    fn after_tx_executed(
        _tx: &TxContext,
        _gas_consumed: u64,
        _tx_result: &SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        Ok(())
    }
}

/// STF type for TxContext (Ethereum transactions via mempool).
pub type MempoolStf = Stf<
    TxContext,
    Block<TxContext>,
    SchedulerBeginBlocker,
    AuthenticationTxValidator<TxContext>,
    SchedulerEndBlocker,
    MempoolNoOpPostTx,
>;

pub const PLACEHOLDER_ACCOUNT: AccountId = AccountId::from_u64(u64::MAX);

/// Default gas configuration for the test application.
pub fn default_gas_config() -> StorageGasConfig {
    StorageGasConfig {
        storage_get_charge: 10,
        storage_set_charge: 10,
        storage_remove_charge: 10,
    }
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

/// List of accounts installed.
pub fn install_account_codes(codes: &mut impl WritableAccountsCodeStorage) {
    codes.add_code(Token::new()).unwrap();
    codes.add_code(Scheduler::new()).unwrap();
    codes.add_code(EthEoaAccount::new()).unwrap();
}

/// Build the standard account-code storage used by the test app binaries.
pub fn build_testapp_codes() -> AccountStorageMock {
    let mut codes = AccountStorageMock::default();
    install_account_codes(&mut codes);
    codes
}

/// Build the bootstrap STF used before the scheduler account exists.
pub fn build_genesis_mempool_stf() -> MempoolStf {
    build_mempool_stf(default_gas_config(), PLACEHOLDER_ACCOUNT)
}

/// Build the steady-state STF once the scheduler account is known.
pub fn build_mempool_stf_from_scheduler(scheduler: AccountId) -> MempoolStf {
    build_mempool_stf(default_gas_config(), scheduler)
}

#[derive(Clone, Copy, Debug, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct GenesisAccounts {
    pub alice: AccountId,
    pub bob: AccountId,
    pub atom: AccountId,
    pub scheduler: AccountId,
}

impl HasTokenAccountId for GenesisAccounts {
    fn token_account_id(&self) -> AccountId {
        self.atom
    }
}
fn parse_genesis_address_env(var: &str) -> Option<[u8; 20]> {
    use alloy_primitives::Address;
    use std::str::FromStr;

    let raw = std::env::var(var).ok()?;
    let addr = Address::from_str(raw.trim()).ok()?;
    Some(addr.into())
}

/// Genesis initialization logic - can be called from system_exec.
pub fn do_genesis_inner(env: &mut dyn Environment) -> SdkResult<GenesisAccounts> {
    let alice_eth = parse_genesis_address_env("GENESIS_ALICE_ETH_ADDRESS").unwrap_or([0xAA; 20]);
    let bob_eth = parse_genesis_address_env("GENESIS_BOB_ETH_ADDRESS").unwrap_or([0xBB; 20]);
    do_genesis_with_addresses(alice_eth, bob_eth, env)
}

/// Genesis with custom Ethereum addresses for Alice and Bob.
pub fn do_genesis_with_addresses(
    alice_eth_addr: [u8; 20],
    bob_eth_addr: [u8; 20],
    env: &mut dyn Environment,
) -> SdkResult<GenesisAccounts> {
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
    let _atom_eth_addr = register_runtime_contract_account(atom.0, env)?;

    // Create scheduler (no begin blockers needed for block info anymore)
    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
    let _scheduler_eth_addr = register_runtime_contract_account(scheduler_acc.0, env)?;

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
    stf: &MempoolStf,
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
    use std::str::FromStr;

    // Resolve/create canonical EOA accounts from full 20-byte ETH addresses.
    let alice_id = resolve_or_create_eoa_account(Address::from(alice_eth_address), env)?;
    let bob_id = resolve_or_create_eoa_account(Address::from(bob_eth_address), env)?;
    let alice_balance = std::env::var("GENESIS_ALICE_TOKEN_BALANCE")
        .ok()
        .and_then(|v| u128::from_str(v.trim()).ok())
        .unwrap_or(1000);
    let bob_balance = std::env::var("GENESIS_BOB_TOKEN_BALANCE")
        .ok()
        .and_then(|v| u128::from_str(v.trim()).ok())
        .unwrap_or(2000);

    // Create evolve token
    let evolve = TokenRef::initialize(
        FungibleAssetMetadata {
            name: "evolve".to_string(),
            symbol: "ev".to_string(),
            decimals: 6,
            icon_url: "https://lol.wtf".to_string(),
            description: "The evolve coin".to_string(),
        },
        vec![(alice_id, alice_balance), (bob_id, bob_balance)],
        Some(MINTER),
        env,
    )?
    .0;
    let _evolve_eth_addr = register_runtime_contract_account(evolve.0, env)?;

    // Create scheduler
    let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
    let _scheduler_eth_addr = register_runtime_contract_account(scheduler_acc.0, env)?;
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

/// Run the evd genesis flow, producing the persisted evd genesis result.
pub fn run_evd_genesis_output<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
    genesis_config: Option<&EvdGenesisConfig>,
) -> Result<GenesisOutput<EvdGenesisResult>, Box<dyn std::error::Error + Send + Sync>> {
    match genesis_config {
        Some(config) => run_custom_evd_genesis(stf, codes, storage, config),
        None => run_default_evd_genesis(stf, codes, storage),
    }
}

fn run_default_evd_genesis<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
) -> Result<GenesisOutput<EvdGenesisResult>, Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Running default ETH-mapped genesis...");

    let alice_eth_address =
        parse_genesis_address_env("GENESIS_ALICE_ETH_ADDRESS").unwrap_or([0xAA; 20]);
    let bob_eth_address =
        parse_genesis_address_env("GENESIS_BOB_ETH_ADDRESS").unwrap_or([0xBB; 20]);
    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            do_eth_genesis_inner(alice_eth_address, bob_eth_address, env)
        })
        .map_err(|e| format!("genesis failed: {e:?}"))?;
    let changes = state.into_changes().map_err(|e| format!("{e:?}"))?;

    Ok(GenesisOutput {
        genesis_result: EvdGenesisResult {
            token: accounts.evolve,
            scheduler: accounts.scheduler,
        },
        changes,
    })
}

fn run_custom_evd_genesis<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
    genesis_config: &EvdGenesisConfig,
) -> Result<GenesisOutput<EvdGenesisResult>, Box<dyn std::error::Error + Send + Sync>> {
    let funded_accounts = genesis_config.funded_accounts()?;
    let minter = AccountId::from_u64(genesis_config.minter_id);
    let metadata = genesis_config.token.to_metadata();
    let genesis_block = BlockContext::new(0, 0);

    let (genesis_result, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            let balances: Vec<(AccountId, u128)> = funded_accounts
                .iter()
                .map(
                    |(eth_addr, balance)| -> evolve_core::SdkResult<(AccountId, u128)> {
                        let addr = alloy_primitives::Address::from(*eth_addr);
                        Ok((resolve_or_create_eoa_account(addr, env)?, *balance))
                    },
                )
                .collect::<evolve_core::SdkResult<Vec<_>>>()?;

            let token = TokenRef::initialize(metadata.clone(), balances, Some(minter), env)?.0;
            let _token_eth_addr = register_runtime_contract_account(token.0, env)?;

            let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
            let _scheduler_eth_addr = register_runtime_contract_account(scheduler_acc.0, env)?;
            scheduler_acc.update_begin_blockers(vec![], env)?;

            Ok(EvdGenesisResult {
                token: token.0,
                scheduler: scheduler_acc.0,
            })
        })
        .map_err(|e| format!("genesis failed: {e:?}"))?;
    let changes = state.into_changes().map_err(|e| format!("{e:?}"))?;

    Ok(GenesisOutput {
        genesis_result,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::encoding::Encodable;
    use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
    use evolve_core::Message;
    use evolve_storage::MockStorage;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard};

    static ENV_VAR_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        entries: Vec<(&'static str, Option<String>)>,
        _guard: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn acquire() -> Self {
            let guard = ENV_VAR_LOCK.lock().expect("env var lock poisoned");
            Self {
                entries: Vec::new(),
                _guard: guard,
            }
        }

        fn set(&mut self, key: &'static str, value: &str) {
            let old = std::env::var(key).ok();
            std::env::set_var(key, value);
            self.entries.push((key, old));
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (key, old) in self.entries.iter().rev() {
                if let Some(value) = old {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    fn apply_changes_to_map(
        changes: Vec<evolve_stf_traits::StateChange>,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let mut out = BTreeMap::new();
        for change in changes {
            match change {
                evolve_stf_traits::StateChange::Set { key, value } => {
                    out.insert(key, value);
                }
                evolve_stf_traits::StateChange::Remove { key } => {
                    out.remove(&key);
                }
            }
        }
        out
    }

    fn read_token_balance(
        state: &BTreeMap<Vec<u8>, Vec<u8>>,
        token_account_id: AccountId,
        account_id: AccountId,
    ) -> u128 {
        let mut key = token_account_id.as_bytes().to_vec();
        key.push(1u8);
        key.extend(account_id.encode().expect("encode account id"));

        match state.get(&key) {
            Some(value) => Message::from_bytes(value.clone())
                .get::<u128>()
                .expect("decode balance"),
            None => 0,
        }
    }

    fn eoa_account_ids(state: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<AccountId> {
        state
            .iter()
            .filter_map(|(key, value)| {
                if key.len() != 33 || key[0] != ACCOUNT_IDENTIFIER_PREFIX {
                    return None;
                }
                let code_id = Message::from_bytes(value.clone()).get::<String>().ok()?;
                if code_id != "EthEoaAccount" {
                    return None;
                }
                let account_bytes: [u8; 32] = key[1..33].try_into().ok()?;
                Some(AccountId::from_bytes(account_bytes))
            })
            .collect()
    }

    #[test]
    fn default_evd_genesis_funds_eth_mapped_sender_account() {
        let mut env = EnvVarGuard::acquire();
        env.set(
            "GENESIS_ALICE_ETH_ADDRESS",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        );
        env.set(
            "GENESIS_BOB_ETH_ADDRESS",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        );
        env.set("GENESIS_ALICE_TOKEN_BALANCE", "1234");
        env.set("GENESIS_BOB_TOKEN_BALANCE", "5678");

        let storage = MockStorage::new();
        let codes = build_testapp_codes();
        let stf = build_genesis_mempool_stf();
        let output =
            run_evd_genesis_output(&stf, &codes, &storage, None).expect("genesis should succeed");
        let state = apply_changes_to_map(output.changes);

        let eoa_ids = eoa_account_ids(&state);
        assert_eq!(eoa_ids.len(), 2);
        assert_eq!(
            read_token_balance(&state, output.genesis_result.token, eoa_ids[0])
                + read_token_balance(&state, output.genesis_result.token, eoa_ids[1]),
            1234 + 5678,
        );
    }
}
