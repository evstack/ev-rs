//! Evolve Dev Node entrypoint.

use clap::{Args, Parser, Subcommand};
use evolve_core::ReadonlyKV;
use evolve_node::{
    init_dev_node, init_tracing as init_node_tracing, resolve_node_config,
    resolve_node_config_init, run_dev_node_with_rpc_and_mempool_eth,
    run_dev_node_with_rpc_and_mempool_mock_storage, GenesisOutput, InitArgs, RunArgs,
};
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use evolve_testapp::genesis_config::{load_genesis_config, EvdGenesisConfig};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis_inner,
    initialize_custom_genesis_resources, install_account_codes, GenesisAccounts, MempoolStf,
    PLACEHOLDER_ACCOUNT,
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

            let genesis_config = match load_genesis_config(args.custom.genesis_file.as_deref()) {
                Ok(genesis_config) => genesis_config,
                Err(err) => {
                    tracing::error!("{err}");
                    std::process::exit(2);
                }
            };

            let rpc_config = config.to_rpc_config();

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

            let genesis_config = match load_genesis_config(args.custom.genesis_file.as_deref()) {
                Ok(genesis_config) => genesis_config,
                Err(err) => {
                    tracing::error!("{err}");
                    std::process::exit(2);
                }
            };

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

    let funded_accounts = config.funded_accounts()?;

    let minter = AccountId::new(config.minter_id);
    let metadata = config.token.to_metadata();

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            let resources = initialize_custom_genesis_resources(
                &funded_accounts,
                metadata.clone(),
                minter,
                env,
            )?;
            let alice = resources.alice.unwrap_or(resources.token);
            let bob = resources.bob.unwrap_or(alice);

            Ok(GenesisAccounts {
                alice,
                bob,
                atom: resources.token,
                scheduler: resources.scheduler,
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
