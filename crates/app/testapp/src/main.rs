use evolve_operations::{
    config::load_config,
    init_logging,
    observability::logging::{parse_level, LogFormat},
    run_startup_with_mkdir, MetricsRegistry, ShutdownCoordinator, SignalHandler,
};
use evolve_stf::SystemAccounts;
use evolve_testapp::{build_stf, do_genesis, install_account_codes, PLACEHOLDER_ACCOUNT};
use evolve_testing::server_mocks::AccountStorageMock;
use slog::info;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration from file or use defaults
    let config = match load_config("config.yaml") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            eprintln!("Using default configuration for demo");
            // Create a default config for demo purposes
            evolve_operations::config::NodeConfig {
                chain: evolve_operations::config::ChainConfig {
                    chain_id: 1,
                    gas_service_account: 0,
                },
                storage: evolve_operations::config::StorageConfig {
                    path: "./data".to_string(),
                    cache_size: 1024 * 1024 * 1024,
                    write_buffer_size: 64 * 1024 * 1024,
                    partition_prefix: "evolve-state".to_string(),
                },
                rpc: evolve_operations::config::RpcConfig::default(),
                operations: evolve_operations::config::OperationsConfig::default(),
                observability: evolve_operations::config::ObservabilityConfig::default(),
            }
        }
    };

    // Initialize structured logging
    let log_level = parse_level(&config.observability.log_level);
    let log_format: LogFormat = config.observability.log_format.parse().unwrap();
    let (logger, _level_switch) = init_logging(log_level, log_format);

    info!(logger, "Evolve testapp starting";
        "chain_id" => config.chain.chain_id,
        "storage_path" => &config.storage.path
    );

    // Run startup checks
    match run_startup_with_mkdir(&config) {
        Ok(result) => {
            for warning in &result.warnings {
                slog::warn!(logger, "{}", warning);
            }
            info!(logger, "Startup checks passed");
        }
        Err(e) => {
            slog::error!(logger, "Startup checks failed: {}", e);
            return Err(e.into());
        }
    }

    // Initialize metrics
    let metrics = MetricsRegistry::new();
    info!(logger, "Metrics registry initialized");

    // Set up shutdown handling
    let signal_handler = SignalHandler::new();
    let coordinator =
        ShutdownCoordinator::new(Duration::from_secs(config.operations.shutdown_timeout_secs));
    signal_handler.start();

    // Initialize storage and STF
    let storage = evolve_testapp::storage::Storage::default();
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    let bootstrap_stf = build_stf(SystemAccounts::placeholder(), PLACEHOLDER_ACCOUNT);
    let (state, _accounts) = do_genesis(&bootstrap_stf, &codes, &storage).expect("genesis failed");
    let _changes = state.into_changes().expect("failed to get state changes");

    info!(logger, "Genesis complete");

    // Record initial metrics
    metrics.node.record_block(0, 0, 0, 0.0, 0, 0);

    info!(logger, "Evolve testapp initialized successfully";
        "rpc_enabled" => config.rpc.enabled,
        "rpc_addr" => &config.rpc.http_addr
    );

    println!("Node initialized. Waiting for shutdown signal (Ctrl+C)...");
    println!("Metrics available via: MetricsRegistry::encode_prometheus()");

    // Wait for shutdown signal
    let mut rx = signal_handler.subscribe();
    rx.changed().await?;

    info!(
        logger,
        "Shutdown signal received, initiating graceful shutdown"
    );

    // Perform graceful shutdown
    coordinator.shutdown().await;

    info!(logger, "Shutdown complete");
    Ok(())
}
