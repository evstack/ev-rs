//! Evolve Dev Node entrypoint.

use std::net::SocketAddr;

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

    /// Enable gRPC server on this address (e.g. 127.0.0.1:9545)
    #[arg(long)]
    grpc_addr: Option<SocketAddr>,
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
            if let Some(grpc_addr) = args.custom.grpc_addr {
                rpc_config.grpc_addr = Some(grpc_addr);
            }

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
        EvdGenesisConfig::load(p).expect("failed to load genesis config")
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
        .map(Into::into)
        .unwrap_or([0xAA; 20]);
    let bob_eth_address = std::env::var("GENESIS_BOB_ETH_ADDRESS")
        .ok()
        .and_then(|s| Address::from_str(s.trim()).ok())
        .map(Into::into)
        .unwrap_or([0xBB; 20]);

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
    use evolve_testapp::eth_eoa::eth_eoa_account::EthEoaAccountRef;
    use evolve_token::account::TokenRef;
    use evolve_tx_eth::address_to_account_id;

    let funded_accounts: Vec<([u8; 20], u128)> = config
        .accounts
        .iter()
        .filter(|acc| acc.balance > 0)
        .map(|acc| {
            let addr = acc
                .parse_address()
                .expect("invalid address in genesis config");
            (addr.into_array(), acc.balance)
        })
        .collect();

    let minter = AccountId::new(config.minter_id);
    let metadata = config.token.to_metadata();

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            // Register funded EOA accounts through the STF environment
            for (eth_addr, _) in &funded_accounts {
                EthEoaAccountRef::initialize(*eth_addr, env)?;
            }

            let balances: Vec<(AccountId, u128)> = funded_accounts
                .iter()
                .map(|(eth_addr, balance)| {
                    let addr = alloy_primitives::Address::from(*eth_addr);
                    (address_to_account_id(addr), *balance)
                })
                .collect();

            let token = TokenRef::initialize(metadata.clone(), balances, Some(minter), env)?.0;

            let scheduler_acc = SchedulerRef::initialize(vec![], vec![], env)?.0;
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
