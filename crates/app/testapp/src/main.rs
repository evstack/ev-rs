use evolve_cometbft::consensus::Consensus;
use evolve_cometbft::tower::start_server;
use evolve_testapp::{do_genesis, install_account_codes, TxDecoderImpl, STF};
use evolve_testing::server_mocks::AccountStorageMock;
use log::LevelFilter;
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() {
    let storage = evolve_testapp::storage::Storage::default();

    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .init()
        .unwrap();

    log::debug!("logging initialized");

    let consensus = Consensus::new(TxDecoderImpl, storage, codes, STF, do_genesis, "poa");
    start_server("127.0.0.1:26658".to_string(), consensus).await;
}
