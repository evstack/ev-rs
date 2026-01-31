//! Evolve Dev Node entrypoint.

use evolve_core::ReadonlyKV;
use evolve_node::{
    init_dev_node, run_dev_node_with_rpc_and_mempool,
    run_dev_node_with_rpc_and_mempool_mock_storage, GenesisOutput, DEFAULT_DATA_DIR,
};
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_genesis_inner, install_account_codes,
    GenesisAccounts, MempoolStf, PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;
use tracing_subscriber::{fmt, EnvFilter};

fn main() {
    // Initialize tracing with env filter (RUST_LOG=info by default)
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let command = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());

    match command.as_str() {
        "run" => run_dev_node_with_rpc_and_mempool(
            DEFAULT_DATA_DIR,
            build_genesis_stf,
            build_stf_from_genesis,
            build_codes,
            run_genesis_output,
            build_storage,
            evolve_node::RpcConfig::default(),
        ),
        "run-mock" => run_dev_node_with_rpc_and_mempool_mock_storage(
            DEFAULT_DATA_DIR,
            build_genesis_stf,
            build_stf_from_genesis,
            build_codes,
            run_genesis_output,
            evolve_node::RpcConfig::default(),
        ),
        "init" => init_dev_node(
            DEFAULT_DATA_DIR,
            build_genesis_stf,
            build_codes,
            run_genesis_output,
            build_storage,
        ),
        "help" | "--help" | "-h" => {
            println!("Usage: evolve_testapp [run|run-mock|init]");
        }
        other => {
            eprintln!("Unknown command: {other}");
            eprintln!("Usage: evolve_testapp [run|run-mock|init]");
            std::process::exit(2);
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
    use evolve_core::BlockContext;

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| do_genesis_inner(env))
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
