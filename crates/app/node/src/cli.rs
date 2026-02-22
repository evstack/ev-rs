use std::path::PathBuf;

use clap::Args;
use tracing_subscriber::{fmt, EnvFilter};

use crate::{build_figment, ensure_config_exists, NodeConfig, DEFAULT_CONFIG_PATH};

#[derive(Debug, Clone, Args)]
pub struct CommonNodeArgs {
    /// Config YAML path (auto-generated if missing)
    #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
    pub config: PathBuf,

    /// Data directory override
    #[arg(long)]
    pub data_dir: Option<String>,

    /// Log level override
    #[arg(long)]
    pub log_level: Option<String>,
}

#[derive(Debug, Clone, Default, Args)]
pub struct NoArgs {}

#[derive(Debug, Clone, Default, Args)]
pub struct NativeRunConfigArgs {
    /// gRPC server bind address override
    #[arg(long)]
    pub grpc_addr: Option<String>,

    /// JSON-RPC HTTP address override
    #[arg(long)]
    pub rpc_addr: Option<String>,

    /// Chain ID override
    #[arg(long)]
    pub chain_id: Option<u64>,

    /// Disable gzip compression for gRPC
    #[arg(long)]
    pub disable_gzip: bool,

    /// Disable JSON-RPC server
    #[arg(long)]
    pub disable_rpc: bool,

    /// Disable chain indexing during block production
    #[arg(long)]
    pub disable_chain_index: bool,
}

#[derive(Debug, Clone, Args)]
pub struct RunArgs<T: Args> {
    #[command(flatten)]
    pub common: CommonNodeArgs,
    #[command(flatten)]
    pub native: NativeRunConfigArgs,
    #[command(flatten)]
    pub custom: T,
}

#[derive(Debug, Clone, Args)]
pub struct InitArgs<T: Args> {
    #[command(flatten)]
    pub common: CommonNodeArgs,
    #[command(flatten)]
    pub custom: T,
}

/// Resolve a NodeConfig from: defaults < YAML < env vars < CLI flags.
pub fn resolve_node_config(common: &CommonNodeArgs, native: &NativeRunConfigArgs) -> NodeConfig {
    ensure_config_exists(&common.config).expect("failed to ensure config file exists");

    let mut figment = build_figment(&common.config);

    // Apply CLI overrides (highest priority)
    if let Some(ref v) = common.data_dir {
        figment = figment.merge(("storage.path", v.as_str()));
    }
    if let Some(ref v) = common.log_level {
        figment = figment.merge(("observability.log_level", v.as_str()));
    }
    if let Some(ref v) = native.grpc_addr {
        figment = figment.merge(("grpc.addr", v.as_str()));
    }
    if let Some(ref v) = native.rpc_addr {
        figment = figment.merge(("rpc.http_addr", v.as_str()));
    }
    if let Some(v) = native.chain_id {
        figment = figment.merge(("chain.chain_id", v));
    }
    if native.disable_gzip {
        figment = figment.merge(("grpc.enable_gzip", false));
    }
    if native.disable_rpc {
        figment = figment.merge(("rpc.enabled", false));
    }
    if native.disable_chain_index {
        figment = figment.merge(("rpc.enable_block_indexing", false));
    }

    let config: NodeConfig = figment.extract().expect("failed to extract config");
    config.validate().expect("invalid config");
    config
}

/// Resolve a NodeConfig for the init subcommand (no NativeRunConfigArgs).
pub fn resolve_node_config_init(common: &CommonNodeArgs) -> NodeConfig {
    resolve_node_config(common, &NativeRunConfigArgs::default())
}

pub fn init_tracing(log_level: &str) {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_default_config;
    use tempfile::tempdir;

    #[test]
    fn resolve_node_config_applies_common_and_native_overrides() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.yaml");
        write_default_config(&config_path, false).expect("write config");

        let common = CommonNodeArgs {
            config: config_path,
            data_dir: Some("./from-common".to_string()),
            log_level: Some("warn".to_string()),
        };
        let native = NativeRunConfigArgs {
            rpc_addr: Some("127.0.0.1:1999".to_string()),
            ..Default::default()
        };

        let resolved = resolve_node_config(&common, &native);
        assert_eq!(resolved.storage.path, "./from-common");
        assert_eq!(resolved.observability.log_level, "warn");
        assert_eq!(resolved.rpc.http_addr, "127.0.0.1:1999");
    }

    #[test]
    fn resolve_node_config_disable_flags() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("config.yaml");
        write_default_config(&config_path, false).expect("write config");

        let common = CommonNodeArgs {
            config: config_path,
            data_dir: None,
            log_level: None,
        };
        let native = NativeRunConfigArgs {
            disable_gzip: true,
            disable_rpc: true,
            disable_chain_index: true,
            ..Default::default()
        };

        let resolved = resolve_node_config(&common, &native);
        assert!(!resolved.grpc.enable_gzip);
        assert!(!resolved.rpc.enabled);
        assert!(!resolved.rpc.enable_block_indexing);
    }
}
