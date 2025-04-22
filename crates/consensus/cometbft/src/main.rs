use crate::consensus::Consensus;
use crate::tower::start_server;
use evolve_testapp::{do_genesis, install_account_codes, new_stf_with_custom_block, TxDecoderImpl};
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};
use log::LevelFilter;
use simple_logger::SimpleLogger;

mod consensus;
mod tower;
mod types;

#[tokio::main]
async fn main() {
    SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap();

    log::debug!("logging initialized");

    let mut account_codes = AccountStorageMock::new();
    install_account_codes(&mut account_codes);

    let consensus = Consensus::new(
        TxDecoderImpl,
        StorageMock::new(),
        account_codes,
        new_stf_with_custom_block(),
        do_genesis,
    );
    start_server("127.0.0.1:26658".to_string(), consensus).await;
}
