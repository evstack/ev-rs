//! Evolve external-consensus node daemon entrypoint.

use clap::{Args, Parser, Subcommand};
use commonware_runtime::tokio::Context as TokioContext;
use evolve_evnode::run_external_consensus_node_eth;
use evolve_node::{
    init_dev_node, init_tracing as init_node_tracing, resolve_node_config,
    resolve_node_config_init, InitArgs, NodeConfig, RunArgs,
};
use evolve_storage::{QmdbStorage, StorageConfig};
use evolve_testapp::genesis_config::{load_genesis_config, EvdGenesisConfig, EvdGenesisResult};
use evolve_testapp::{
    build_genesis_mempool_stf, build_mempool_stf_from_scheduler, build_testapp_codes,
    run_evd_genesis_output,
};

#[derive(Parser)]
#[command(name = "evd")]
#[command(about = "Evolve node daemon with gRPC execution layer")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the node with gRPC and JSON-RPC servers
    Run(EvdRunArgs),
    /// Initialize genesis state without running
    Init(EvdInitArgs),
}

type EvdRunArgs = RunArgs<EvdRunCustom>;
type EvdInitArgs = InitArgs<EvdInitCustom>;

#[derive(Args)]
struct EvdRunCustom {
    /// Path to a genesis JSON file with ETH accounts (uses default testapp genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

#[derive(Args)]
struct EvdInitCustom {
    /// Path to a genesis JSON file with ETH accounts (uses default testapp genesis if omitted)
    #[arg(long)]
    genesis_file: Option<String>,
}

type NodeStorage = QmdbStorage<TokioContext>;
type StorageError = Box<dyn std::error::Error + Send + Sync>;

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            let config = resolve_node_config(&args.common, &args.native);
            init_node_tracing(&config.observability.log_level);
            let genesis_config = parse_genesis_config(args.custom.genesis_file.as_deref());
            run_node(config, genesis_config);
        }
        Commands::Init(args) => {
            let config = resolve_node_config_init(&args.common);
            init_node_tracing(&config.observability.log_level);
            let genesis_config = parse_genesis_config(args.custom.genesis_file.as_deref());
            init_genesis(&config.storage.path, genesis_config);
        }
    }
}

fn parse_genesis_config(path: Option<&str>) -> Option<EvdGenesisConfig> {
    match load_genesis_config(path) {
        Ok(genesis_config) => genesis_config,
        Err(err) => {
            tracing::error!("{err}");
            std::process::exit(2);
        }
    }
}

fn run_node(config: NodeConfig, genesis_config: Option<EvdGenesisConfig>) {
    run_external_consensus_node_eth(
        config,
        build_genesis_mempool_stf,
        |genesis: &EvdGenesisResult| build_mempool_stf_from_scheduler(genesis.scheduler),
        build_testapp_codes,
        move |stf, codes, storage| {
            run_evd_genesis_output(stf, codes, storage, genesis_config.as_ref())
        },
        build_storage,
    );
}

fn init_genesis(data_dir: &str, genesis_config: Option<EvdGenesisConfig>) {
    init_dev_node(
        data_dir,
        build_genesis_mempool_stf,
        build_testapp_codes,
        move |stf, codes, storage| {
            run_evd_genesis_output(stf, codes, storage, genesis_config.as_ref())
        },
        build_storage,
    );
}

async fn build_storage(
    context: TokioContext,
    config: StorageConfig,
) -> Result<NodeStorage, StorageError> {
    Ok(QmdbStorage::new(context, config).await?)
}
