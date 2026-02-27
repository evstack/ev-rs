//! Example: Running an EVNode gRPC server with the testapp STF.
//!
//! This example demonstrates how to:
//! 1. Set up the account code storage
//! 2. Create a mock storage
//! 3. Build the STF
//! 4. Configure and start the EVNode gRPC server
//! 5. Wire up state change callbacks to persist genesis and block execution state
//!
//! Run with:
//! ```bash
//! cargo run --example run_server --features testapp -p evolve_evnode
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use evolve_core::{AccountId, ErrorCode, ReadonlyKV};
use evolve_evnode::{EvnodeServer, EvnodeServerConfig, ExecutorServiceConfig, StateChangeCallback};
use evolve_stf_traits::{AccountsCodeStorage, StateChange, WritableAccountsCodeStorage};
use evolve_testapp::{build_mempool_stf, default_gas_config, install_account_codes};

/// Scheduler account ID as allocated by do_genesis_inner in testapp.
/// This is the 4th account created: alice(65535), bob(65536), atom(65537), scheduler(65538)
const SCHEDULER_ACCOUNT_ID: u128 = 65538;

/// Simple in-memory storage for the example.
struct ExampleStorageInner {
    data: RwLock<BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl ExampleStorageInner {
    fn new() -> Self {
        Self {
            data: RwLock::new(BTreeMap::new()),
        }
    }

    /// Apply state changes to the storage.
    fn apply_changes(&self, changes: &[StateChange]) {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|e| panic!("example storage write lock poisoned: {e}"));
        for change in changes {
            match change {
                StateChange::Set { key, value } => {
                    data.insert(key.clone(), value.clone());
                }
                StateChange::Remove { key } => {
                    data.remove(key);
                }
            }
        }
    }
}

impl ReadonlyKV for ExampleStorageInner {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        Ok(self
            .data
            .read()
            .unwrap_or_else(|e| panic!("example storage read lock poisoned: {e}"))
            .get(key)
            .cloned())
    }
}

/// Shared storage wrapper that can be cloned and shared between threads.
#[derive(Clone)]
struct ExampleStorage(Arc<ExampleStorageInner>);

impl ExampleStorage {
    fn new() -> Self {
        Self(Arc::new(ExampleStorageInner::new()))
    }

    fn apply_changes(&self, changes: &[StateChange]) {
        self.0.apply_changes(changes);
    }
}

impl ReadonlyKV for ExampleStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        self.0.get(key)
    }
}

/// Simple account code storage for the example.
struct ExampleCodes {
    codes: BTreeMap<String, Box<dyn evolve_core::AccountCode>>,
}

impl ExampleCodes {
    fn new() -> Self {
        Self {
            codes: BTreeMap::new(),
        }
    }
}

impl AccountsCodeStorage for ExampleCodes {
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn evolve_core::AccountCode>) -> R,
    {
        Ok(f(self.codes.get(identifier).map(|c| c.as_ref())))
    }

    fn list_identifiers(&self) -> Vec<String> {
        self.codes.keys().cloned().collect()
    }
}

impl WritableAccountsCodeStorage for ExampleCodes {
    fn add_code(&mut self, code: impl evolve_core::AccountCode + 'static) -> Result<(), ErrorCode> {
        self.codes.insert(code.identifier(), Box::new(code));
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // 1. Set up account codes
    let mut codes = ExampleCodes::new();
    install_account_codes(&mut codes);
    tracing::info!("Installed account codes: {:?}", codes.list_identifiers());

    // 2. Create storage (using wrapper that's internally Arc-based for sharing)
    let storage = ExampleStorage::new();
    let storage_for_callback = storage.clone();

    // 3. Create state change callback to persist changes to storage
    let state_change_callback: StateChangeCallback = Arc::new(move |changes| {
        storage_for_callback.apply_changes(changes);
        tracing::debug!("Applied {} state changes to storage", changes.len());
    });

    // 4. Build the STF
    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config, AccountId::new(SCHEDULER_ACCOUNT_ID));

    // 5. Configure the server
    let addr = "127.0.0.1:50051".parse()?;
    let config = EvnodeServerConfig {
        addr,
        enable_gzip: true,
        max_message_size: 4 * 1024 * 1024,
        executor_config: ExecutorServiceConfig {
            max_gas: 30_000_000,
            max_bytes: 128 * 1024,
        },
    };

    tracing::info!("Starting EVNode gRPC server on {}", addr);
    tracing::info!("Configuration:");
    tracing::info!("  - Max gas per block: {}", config.executor_config.max_gas);
    tracing::info!("  - Max bytes per tx: {}", config.executor_config.max_bytes);
    tracing::info!("  - Gzip compression: {}", config.enable_gzip);

    // 6. Create and run the server with state change callback
    let server = EvnodeServer::new(config, stf, storage, codes)
        .with_state_change_callback(state_change_callback);

    // Run the server (this blocks until shutdown)
    server.serve().await
}
