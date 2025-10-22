//! Evolve Dev Node - produces blocks at a fixed interval using DevConsensus.
//!
//! Uses QmdbStorage for persistent state across restarts.
//! Data is stored in `./data` by default.

use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::Runner as RunnerTrait;
use evolve_core::ReadonlyKV;
use evolve_server::{
    load_chain_state, save_chain_state, ChainState, DevConfig, DevConsensus, CHAIN_STATE_KEY,
};
use evolve_stf_traits::StateChange;
use evolve_storage::{Operation, QmdbStorage, Storage, StorageConfig};
use evolve_testapp::{
    build_stf, default_gas_config, install_account_codes, CustomStf, GenesisAccounts, TestTx,
    PLACEHOLDER_ACCOUNT,
};
use evolve_testing::server_mocks::AccountStorageMock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Default data directory for persistent storage.
const DEFAULT_DATA_DIR: &str = "./data";

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

    let mut changes = state.into_changes().map_err(|e| format!("{:?}", e))?;

    // Store initial chain state (height 1, since genesis is block 0)
    let chain_state: ChainState<GenesisAccounts> = ChainState {
        height: 1,
        genesis_result: accounts,
    };
    changes.push(StateChange::Set {
        key: CHAIN_STATE_KEY.to_vec(),
        value: borsh::to_vec(&chain_state).map_err(|e| format!("serialize chain state: {e}"))?,
    });

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

    log::info!("=== Evolve Dev Node ===");

    // Use stable data directory for persistence
    let data_dir = PathBuf::from(DEFAULT_DATA_DIR);
    std::fs::create_dir_all(&data_dir).expect("failed to create data directory");

    let storage_config = StorageConfig {
        path: data_dir.clone(),
        ..Default::default()
    };

    // Setup commonware runtime
    let runtime_config = TokioConfig::default()
        .with_storage_directory(&data_dir)
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
        // Create QmdbStorage with persistent path
        let storage = QmdbStorage::new(context, storage_config)
            .await
            .expect("failed to create storage");

        // Create account codes
        let mut codes = AccountStorageMock::default();
        install_account_codes(&mut codes);

        let gas_config = default_gas_config();

        // Check if we have existing chain state
        let (accounts, initial_height) = match load_chain_state::<GenesisAccounts, _>(&storage) {
            Some(state) => {
                log::info!("Resuming from existing state at height {}", state.height);
                log::info!("  Alice: {:?}", state.genesis_result.alice);
                log::info!("  Bob: {:?}", state.genesis_result.bob);
                log::info!("  Scheduler: {:?}", state.genesis_result.scheduler);
                (state.genesis_result, state.height)
            }
            None => {
                log::info!("No existing state found, running genesis...");
                let bootstrap_stf = build_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
                let accounts = run_genesis(&bootstrap_stf, &codes, &storage)
                    .await
                    .expect("genesis failed");

                log::info!("Genesis complete. Accounts:");
                log::info!("  Alice: {:?}", accounts.alice);
                log::info!("  Bob: {:?}", accounts.bob);
                log::info!("  Scheduler: {:?}", accounts.scheduler);
                (accounts, 1)
            }
        };

        // Build production STF
        let stf = build_stf(gas_config, accounts.scheduler);

        // Create DevConsensus with 1 second block interval, starting from persisted height
        let block_interval = Duration::from_secs(1);
        let dev_config = DevConfig {
            block_interval: Some(block_interval),
            initial_height,
            ..Default::default()
        };
        let dev: Arc<DevConsensus<CustomStf, _, AccountStorageMock, TestTx>> =
            Arc::new(DevConsensus::new(stf, storage, codes, dev_config));

        log::info!(
            "Block interval: {:?}, starting at height {}",
            block_interval,
            initial_height
        );

        // Link stop flag to DevConsensus (just signals stop, no storage ops)
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

        // Run block production (blocks until stopped)
        dev.run_block_production().await;

        // Save chain state after block production stops
        let final_height = dev.height();
        log::info!("Stopped at height: {}", final_height);

        let chain_state = ChainState {
            height: final_height,
            genesis_result: accounts,
        };
        if let Err(e) = save_chain_state(dev.storage(), &chain_state).await {
            log::error!("Failed to save chain state: {}", e);
        } else {
            log::info!("Saved chain state at height {}", final_height);
        }
    });
}
