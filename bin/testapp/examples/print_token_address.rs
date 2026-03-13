use std::path::PathBuf;
use std::sync::mpsc;

use clap::Parser;
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::Runner as RunnerTrait;
use evolve_node::HasTokenAccountId;
use evolve_server::load_chain_state;
use evolve_storage::{QmdbStorage, StorageConfig};
use evolve_testapp::GenesisAccounts;
use evolve_tx_eth::derive_runtime_contract_address;

#[derive(Debug, Parser)]
#[command(name = "print_token_address")]
#[command(about = "Print the testapp token contract address for an initialized data dir")]
struct Cli {
    /// Path to the initialized node data directory
    #[arg(long)]
    data_dir: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    let data_dir = cli.data_dir;
    let (tx, rx) = mpsc::channel();

    let runtime_config = TokioConfig::default()
        .with_storage_directory(&data_dir)
        .with_worker_threads(2);

    Runner::new(runtime_config).start(move |context| async move {
        let storage = QmdbStorage::new(
            context,
            StorageConfig {
                path: data_dir,
                ..Default::default()
            },
        )
        .await
        .expect("open qmdb storage");

        let state =
            load_chain_state::<GenesisAccounts, _>(&storage).expect("load initialized chain state");
        let token_address =
            derive_runtime_contract_address(state.genesis_result.token_account_id());

        tx.send(token_address).expect("send token address");
    });

    let token_address = rx.recv().expect("receive token address");
    println!("{token_address:#x}");
}
