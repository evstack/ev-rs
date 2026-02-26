//! Evolve Dev Node entrypoint.

use clap::{Args, Parser, Subcommand};
use evolve_core::ReadonlyKV;
use evolve_node::{
    init_dev_node, init_tracing as init_node_tracing, resolve_node_config,
    resolve_node_config_init, run_dev_node_with_rpc_and_mempool_eth,
    run_dev_node_with_rpc_and_mempool_mock_storage, GenesisOutput, InitArgs, NoArgs, RunArgs,
};
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
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
type TestappInitArgs = InitArgs<NoArgs>;

#[derive(Args)]
struct TestappRunCustom {
    /// Use in-memory mock storage instead of persistent storage
    #[arg(long)]
    mock_storage: bool,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            let config = resolve_node_config(&args.common, &args.native);
            init_node_tracing(&config.observability.log_level);

            let rpc_config = config.to_rpc_config();
            if args.custom.mock_storage {
                run_dev_node_with_rpc_and_mempool_mock_storage(
                    &config.storage.path,
                    build_genesis_stf,
                    build_stf_from_genesis,
                    build_codes,
                    run_genesis_output,
                    rpc_config,
                );
            } else {
                run_dev_node_with_rpc_and_mempool_eth(
                    &config.storage.path,
                    build_genesis_stf,
                    build_stf_from_genesis,
                    build_codes,
                    run_genesis_output,
                    build_storage,
                    rpc_config,
                );
            }
        }
        Commands::Init(args) => {
            let config = resolve_node_config_init(&args.common);
            init_node_tracing(&config.observability.log_level);

            init_dev_node(
                &config.storage.path,
                build_genesis_stf,
                build_codes,
                run_genesis_output,
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

async fn build_storage(
    context: commonware_runtime::tokio::Context,
    config: StorageConfig,
) -> Result<QmdbStorage<commonware_runtime::tokio::Context>, Box<dyn std::error::Error + Send + Sync>>
{
    QmdbStorage::new(context, config)
        .await
        .map_err(|e| Box::new(e) as _)
}
