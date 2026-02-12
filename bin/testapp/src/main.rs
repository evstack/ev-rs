//! Evolve Dev Node entrypoint.

use evolve_core::ReadonlyKV;
use evolve_node::{
    init_dev_node, run_dev_node_with_rpc_and_mempool_eth,
    run_dev_node_with_rpc_and_mempool_mock_storage, GenesisOutput, DEFAULT_DATA_DIR,
};
use evolve_storage::{QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis_inner, install_account_codes,
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

    let disable_chain_index = std::env::var("EVOLVE_DISABLE_CHAIN_INDEX")
        .ok()
        .map(|v| {
            let s = v.trim().to_ascii_lowercase();
            s == "1" || s == "true" || s == "yes" || s == "on"
        })
        .unwrap_or(false);

    let rpc_config = evolve_node::RpcConfig::default().with_block_indexing(!disable_chain_index);

    match command.as_str() {
        "run" => run_dev_node_with_rpc_and_mempool_eth(
            DEFAULT_DATA_DIR,
            build_genesis_stf,
            build_stf_from_genesis,
            build_codes,
            run_genesis_output,
            build_storage,
            rpc_config.clone(),
        ),
        "run-mock" => run_dev_node_with_rpc_and_mempool_mock_storage(
            DEFAULT_DATA_DIR,
            build_genesis_stf,
            build_stf_from_genesis,
            build_codes,
            run_genesis_output,
            rpc_config,
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
