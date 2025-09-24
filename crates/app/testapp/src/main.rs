use evolve_operations::SignalHandler;
use evolve_server::EvolveServer;
use evolve_testapp::{
    build_stf, default_gas_config, do_genesis, install_account_codes, CustomStf,
    PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize storage and codes
    let storage = StorageMock::default();
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    // Build STF and run genesis
    let stf = build_stf(default_gas_config(), PLACEHOLDER_ACCOUNT);
    let (state, _accounts) = do_genesis(&stf, &codes, &storage).expect("genesis failed");
    let _changes = state.into_changes().expect("failed to get state changes");

    // Build server (uses defaults if no config provided)
    let server = EvolveServer::<CustomStf, StorageMock, AccountStorageMock>::builder()
        .config_path("config.yaml") // optional - uses defaults if file not found
        .build(stf, storage, codes)
        .await?;

    log::info!(
        "Evolve testapp initialized - RPC: {}, chain_id: {}",
        server.config().rpc.http_addr,
        server.config().chain.chain_id
    );

    println!("Node initialized. Waiting for shutdown signal (Ctrl+C)...");

    // Wait for shutdown signal
    let signal_handler = SignalHandler::new();
    signal_handler.start();
    let mut rx = signal_handler.subscribe();
    rx.changed().await?;

    // Graceful shutdown
    server.shutdown().await?;

    Ok(())
}
