//! Evolve Dev Node entrypoint.

use clap::{Args, Parser, Subcommand};
use evolve_core::ReadonlyKV;
use evolve_node::{
    init_dev_node, init_tracing as init_node_tracing, resolve_node_config,
    resolve_node_config_init, run_dev_node_with_rpc_and_mempool_eth,
    run_dev_node_with_rpc_and_mempool_mock_storage, GenesisOutput, InitArgs, RunArgs,
};
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use evolve_testapp::genesis_config::EvdGenesisConfig;
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis_inner, install_account_codes,
    GenesisAccounts, MempoolStf, PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;

#[derive(Parser)]
#[command(name = "testapp")]
#[command(about = "Evolve testapp node")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the dev node with persistent storage
    Run(TestappRunArgs),
    /// Initialize genesis state without running
    Init(TestappInitArgs),
}

type TestappRunArgs = RunArgs<TestappRunCustom>;
type TestappInitArgs = InitArgs<TestappInitCustom>;

#[derive(Args)]
struct TestappRunCustom {
    /// Use in-memory mock storage instead of persistent storage
    #[arg(long)]
    mock_storage: bool,

    /// Path to a genesis JSON file with ETH accounts (uses default Alice/Bob genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

#[derive(Args)]
struct TestappInitCustom {
    /// Path to a genesis JSON file with ETH accounts (uses default Alice/Bob genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            let config = resolve_node_config(&args.common, &args.native);
            init_node_tracing(&config.observability.log_level);

            let genesis_config = load_genesis_config(args.custom.genesis_file.as_deref());

            let mut rpc_config = config.to_rpc_config();
            rpc_config.grpc_addr = Some(config.parsed_grpc_addr());

            if args.custom.mock_storage {
                run_dev_node_with_rpc_and_mempool_mock_storage(
                    &config.storage.path,
                    build_genesis_stf,
                    build_stf_from_genesis,
                    build_codes,
                    move |stf, codes, storage| {
                        run_genesis_output(stf, codes, storage, genesis_config.as_ref())
                    },
                    rpc_config,
                );
            } else {
                run_dev_node_with_rpc_and_mempool_eth(
                    &config.storage.path,
                    build_genesis_stf,
                    build_stf_from_genesis,
                    build_codes,
                    move |stf, codes, storage| {
                        run_genesis_output(stf, codes, storage, genesis_config.as_ref())
                    },
                    build_storage,
                    rpc_config,
                );
            }
        }
        Commands::Init(args) => {
            let config = resolve_node_config_init(&args.common);
            init_node_tracing(&config.observability.log_level);

            let genesis_config = load_genesis_config(args.custom.genesis_file.as_deref());

            init_dev_node(
                &config.storage.path,
                build_genesis_stf,
                build_codes,
                move |stf, codes, storage| {
                    run_genesis_output(stf, codes, storage, genesis_config.as_ref())
                },
                build_storage,
            );
        }
    }
}

fn load_genesis_config(path: Option<&str>) -> Option<EvdGenesisConfig> {
    path.map(|p| {
        tracing::info!("Loading genesis config from: {}", p);
        EvdGenesisConfig::load(p)
            .unwrap_or_else(|e| panic!("failed to load genesis config '{p}': {e}"))
    })
}

fn build_codes() -> AccountStorageMock {
    let mut codes = AccountStorageMock::default();
    install_account_codes(&mut codes);
    codes
}

fn build_genesis_stf() -> MempoolStf {
    build_mempool_stf(default_gas_config(), PLACEHOLDER_ACCOUNT)
}

fn build_stf_from_genesis(genesis: &GenesisAccounts) -> MempoolStf {
    build_mempool_stf(default_gas_config(), genesis.scheduler)
}

fn run_genesis_output<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
    genesis_config: Option<&EvdGenesisConfig>,
) -> Result<GenesisOutput<GenesisAccounts>, Box<dyn std::error::Error + Send + Sync>> {
    match genesis_config {
        Some(config) => run_custom_genesis(stf, codes, storage, config),
        None => run_default_genesis(stf, codes, storage),
    }
}

fn run_default_genesis<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
) -> Result<GenesisOutput<GenesisAccounts>, Box<dyn std::error::Error + Send + Sync>> {
    use alloy_primitives::Address;
    use evolve_core::BlockContext;
    use std::str::FromStr;

    let genesis_block = BlockContext::new(0, 0);
    let alice_eth_address = std::env::var("GENESIS_ALICE_ETH_ADDRESS")
        .ok()
        .and_then(|s| Address::from_str(s.trim()).ok())
        .map_or([0xAA; 20], Into::into);
    let bob_eth_address = std::env::var("GENESIS_BOB_ETH_ADDRESS")
        .ok()
        .and_then(|s| Address::from_str(s.trim()).ok())
        .map_or([0xBB; 20], Into::into);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            let eth_accounts = do_eth_genesis_inner(alice_eth_address, bob_eth_address, env)?;
            Ok(GenesisAccounts {
                alice: eth_accounts.alice,
                bob: eth_accounts.bob,
                atom: eth_accounts.evolve,
                scheduler: eth_accounts.scheduler,
            })
        })
        .map_err(|e| format!("{:?}", e))?;

    let changes = state.into_changes().map_err(|e| format!("{:?}", e))?;

    Ok(GenesisOutput {
        genesis_result: accounts,
        changes,
    })
}

fn run_custom_genesis<S: ReadonlyKV + Storage>(
    stf: &MempoolStf,
    codes: &AccountStorageMock,
    storage: &S,
    config: &EvdGenesisConfig,
) -> Result<GenesisOutput<GenesisAccounts>, Box<dyn std::error::Error + Send + Sync>> {
    use evolve_core::{AccountId, BlockContext};
    use evolve_scheduler::scheduler_account::SchedulerRef;
    use evolve_token::account::TokenRef;
    use evolve_tx_eth::{register_runtime_contract_account, resolve_or_create_eoa_account};

    let funded_accounts: Vec<([u8; 20], u128)> = config
        .accounts
        .iter()
        .filter(|acc| acc.balance > 0)
        .map(|acc| {
            let addr = acc
                .parse_address()
                .unwrap_or_else(|e| panic!("invalid address in genesis config: {e}"));
            (addr.into_array(), acc.balance)
        })
        .collect();

    let minter = AccountId::new(config.minter_id);
    let metadata = config.token.to_metadata();

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
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

            Ok(GenesisAccounts {
                alice: token.0,
                bob: token.0,
                atom: token.0,
                scheduler: scheduler_acc.0,
            })
        })
        .map_err(|e| format!("{:?}", e))?;

    let changes = state.into_changes().map_err(|e| format!("{:?}", e))?;

    Ok(GenesisOutput {
        genesis_result: accounts,
        changes,
    })
}

async fn build_storage(
    context: commonware_runtime::tokio::Context,
    config: StorageConfig,
) -> Result<QmdbStorage<commonware_runtime::tokio::Context>, Box<dyn std::error::Error + Send + Sync>>
{
    QmdbStorage::new(context, config)
        .await
        .map_err(|e| Box::new(e) as _)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::map_unwrap_or)]
mod tests {
    use super::*;
    use evolve_core::encoding::Encodable;
    use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
    use evolve_core::AccountId;
    use evolve_core::Message;
    use evolve_storage::MockStorage;
    use evolve_testapp::genesis_config::{AccountConfig, TokenConfig};
    use std::collections::BTreeMap;

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
        key.push(1u8); // Token::balances storage prefix
        key.extend(account_id.encode().expect("encode account id"));

        match state.get(&key) {
            Some(value) => Message::from_bytes(value.clone())
                .get::<u128>()
                .expect("decode balance"),
            None => 0,
        }
    }

    fn count_registered_code_id(
        state: &BTreeMap<Vec<u8>, Vec<u8>>,
        expected_code_id: &str,
    ) -> usize {
        state
            .iter()
            .filter(|(key, value)| {
                if key.len() != 33 || key[0] != ACCOUNT_IDENTIFIER_PREFIX {
                    return false;
                }
                Message::from_bytes((*value).clone())
                    .get::<String>()
                    .map(|code_id| code_id == expected_code_id)
                    .unwrap_or(false)
            })
            .count()
    }

    #[test]
    fn custom_genesis_funds_registry_resolved_eoa_account() {
        let stf = build_genesis_stf();
        let codes = build_codes();
        let storage = MockStorage::new();

        let config = EvdGenesisConfig {
            token: TokenConfig {
                name: "evolve".to_string(),
                symbol: "ev".to_string(),
                decimals: 6,
                icon_url: "https://example.com/icon.png".to_string(),
                description: "token".to_string(),
            },
            minter_id: 100_002,
            accounts: vec![AccountConfig {
                eth_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
                balance: 777,
            }],
        };

        let output = run_custom_genesis(&stf, &codes, &storage, &config).expect("custom genesis");
        let state = apply_changes_to_map(output.changes);

        let mapped_id = state
            .iter()
            .find_map(|(key, value)| {
                if key.len() != 33 || key[0] != ACCOUNT_IDENTIFIER_PREFIX {
                    return None;
                }
                let code_id = Message::from_bytes((*value).clone()).get::<String>().ok()?;
                if code_id != "EthEoaAccount" {
                    return None;
                }
                let id_bytes: [u8; 32] = key[1..33].try_into().ok()?;
                Some(AccountId::from_bytes(id_bytes))
            })
            .expect("eoa account id");
        assert_eq!(
            read_token_balance(&state, output.genesis_result.atom, mapped_id),
            777
        );

        assert_eq!(count_registered_code_id(&state, "EthEoaAccount"), 1);
    }
}
