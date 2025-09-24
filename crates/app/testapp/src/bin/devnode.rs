//! Dev node binary - produces blocks at a fixed interval using DevConsensus.
//!
//! Usage: cargo run -p evolve_testapp --bin devnode

use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::Runner as RunnerTrait;
use evolve_server::{DevConfig, DevConsensus};
use evolve_core::ReadonlyKV;
use evolve_storage::{Operation, QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_stf, default_gas_config, install_account_codes, CustomStf, GenesisAccounts, TestTx,
    PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Run genesis on the storage and return the accounts.
async fn run_genesis<S: ReadonlyKV + Storage>(
    stf: &CustomStf,
    codes: &AccountStorageMock,
    storage: &S,
) -> Result<GenesisAccounts, Box<dyn std::error::Error + Send + Sync>> {
    use evolve_core::BlockContext;

    let genesis_block = BlockContext::new(0, 0);

    let (accounts, state) = stf
        .system_exec(storage, codes, genesis_block, |env| {
            evolve_testapp::do_genesis_inner(env)
        })
        .map_err(|e| format!("{:?}", e))?;

    let changes = state.into_changes().map_err(|e| format!("{:?}", e))?;

    // Convert StateChange to Operation and apply via async storage
    let operations: Vec<Operation> = changes.into_iter().map(Into::into).collect();
    log::info!("Genesis produced {} state changes", operations.len());

    storage
        .batch(operations)
        .await
        .map_err(|e| format!("batch failed: {:?}", e))?;

    storage
        .commit()
        .await
        .map_err(|e| format!("commit failed: {:?}", e))?;
    log::info!("Genesis committed successfully");

    Ok(accounts)
}

fn main() {
    simple_logger::init_with_level(log::Level::Info).expect("failed to init logger");

    log::info!("=== Evolve Dev Node (DevConsensus + QmdbStorage) ===");
    log::info!("Block interval: 100ms");

    // Create temp directory for storage
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let storage_config = StorageConfig {
        path: temp_dir.path().to_path_buf(),
        ..Default::default()
    };

    // Setup commonware runtime
    let runtime_config = TokioConfig::default()
        .with_storage_directory(temp_dir.path())
        .with_worker_threads(2);

    let runner = Runner::new(runtime_config);

    // Shared stop flag for Ctrl+C handler
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = Arc::clone(&stop_flag);

    ctrlc::set_handler(move || {
        log::info!("Received Ctrl+C, shutting down...");
        stop_flag_clone.store(true, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    runner.start(|context| async move {
        // Create QmdbStorage
        let storage = QmdbStorage::new(context, storage_config)
            .await
            .expect("failed to create storage");

        // Create account codes
        let mut codes = AccountStorageMock::default();
        install_account_codes(&mut codes);

        // Bootstrap STF and run genesis
        let gas_config = default_gas_config();
        let bootstrap_stf = build_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
        let accounts = run_genesis(&bootstrap_stf, &codes, &storage)
            .await
            .expect("genesis failed");

        log::info!("Genesis complete. Accounts:");
        log::info!("  Alice: {:?}", accounts.alice);
        log::info!("  Bob: {:?}", accounts.bob);
        log::info!("  Scheduler: {:?}", accounts.scheduler);

        // Build production STF
        let stf = build_stf(gas_config, accounts.scheduler);

        // Create DevConsensus
        let dev_config = DevConfig::with_interval(Duration::from_millis(100));
        let dev: Arc<DevConsensus<CustomStf, _, AccountStorageMock, TestTx>> =
            Arc::new(DevConsensus::new(stf, storage, codes, dev_config));

        // Link stop flag to DevConsensus
        let dev_for_stop = Arc::clone(&dev);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if stop_flag.load(Ordering::SeqCst) {
                    dev_for_stop.stop();
                    break;
                }
            }
        });

        log::info!("Starting block production... (Ctrl+C to stop)");

        // Run block production (this is a non-Send future, runs on current thread)
        dev.run_block_production().await;

        log::info!("Stopped at height: {}", dev.height());
    });

    // Keep temp_dir alive until runner completes
    drop(temp_dir);
}
