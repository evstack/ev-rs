use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{keccak256, Address, Bytes, TxKind, U256};
use commonware_runtime::tokio::{Config as TokioConfig, Runner};
use commonware_runtime::Runner as RunnerTrait;
use evolve_server::load_chain_state;
use evolve_storage::{QmdbStorage, StorageConfig};
use evolve_testapp::genesis_config::EvdGenesisResult;
use evolve_tx_eth::{derive_eth_eoa_account_id, derive_runtime_contract_address, sign_hash};
use k256::ecdsa::{SigningKey, VerifyingKey};
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

const SENDER_KEY_HEX: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const RECIPIENT_KEY_HEX: &str = "59c6995e998f97a5a0044966f0945382d7f0f8cb9b8b9c5d91b3ef1c0d8f4a7c";
const INITIAL_SENDER_BALANCE: u128 = 1_000;
const INITIAL_RECIPIENT_BALANCE: u128 = 1;
const TRANSFER_AMOUNT: u128 = 100;
const EVNODE_IMAGE: &str = "ghcr.io/evstack/ev-node-grpc:main";
const EVNODE_HOME_IN_CONTAINER: &str = "/evnode-home";
const EVNODE_PASSPHRASE_IN_CONTAINER: &str = "/evnode-passphrase.txt";
const EVNODE_SIGNER_PASSPHRASE: &str = "evnode-test-passphrase";
const POLL_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
static UNIQUE_PROJECT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Poll `$probe` until it returns `Some(value)`, panicking with compose logs on timeout.
macro_rules! poll_until {
    ($compose:expr, $msg:expr, $probe:expr) => {{
        let deadline = tokio::time::Instant::now() + POLL_TIMEOUT;
        loop {
            if let Some(val) = $probe {
                break val;
            }
            if tokio::time::Instant::now() >= deadline {
                panic_with_logs($compose, $msg);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }};
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires Docker Compose, a running Docker daemon, and access to ghcr.io/evstack/ev-node-grpc"]
async fn stack_starts_and_produces_blocks_via_evnode_grpc_container() {
    let harness = start_stack().await;
    assert!(
        harness.first_block_number >= 1,
        "expected at least one externally-produced block"
    );
    harness.unwrap(harness.compose.assert_service_running("evd"));
    harness.unwrap(harness.compose.assert_service_running("ev-node"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires Docker Compose, a running Docker daemon, and access to ghcr.io/evstack/ev-node-grpc"]
async fn transaction_submission_succeeds_via_evnode_grpc_container() {
    let harness = start_stack().await;

    let sender_balance_before =
        harness.unwrap(harness.rpc.get_balance(harness.sender_address).await);
    let recipient_balance_before =
        harness.unwrap(harness.rpc.get_balance(harness.recipient_address).await);
    assert_eq!(sender_balance_before, INITIAL_SENDER_BALANCE);
    assert_eq!(recipient_balance_before, INITIAL_RECIPIENT_BALANCE);

    let nonce = harness.unwrap(
        harness
            .rpc
            .get_transaction_count(harness.sender_address)
            .await,
    );
    let raw_tx = build_transfer_tx(
        &harness.sender_key,
        nonce,
        harness.token_address,
        derive_eth_eoa_account_id(harness.recipient_address),
        TRANSFER_AMOUNT,
    );
    let tx_hash = harness.unwrap(harness.rpc.send_raw_transaction(&raw_tx).await);

    let receipt = poll_until!(
        &harness.compose,
        &format!("timed out waiting for receipt for tx {tx_hash}"),
        match harness.rpc.get_transaction_receipt(&tx_hash).await {
            Ok(receipt) => receipt,
            Err(err) => panic_with_logs(&harness.compose, &err),
        }
    );
    assert_eq!(receipt.status.as_deref(), Some("0x1"));
    let receipt_block = harness
        .unwrap(
            receipt
                .block_number
                .as_deref()
                .map(parse_hex_u64)
                .transpose(),
        )
        .expect("receipt must include block number");
    assert!(
        receipt_block >= harness.first_block_number,
        "receipt block must be on or after the first externally-produced block"
    );

    let sender_balance_after =
        harness.unwrap(harness.rpc.get_balance(harness.sender_address).await);
    let recipient_balance_after =
        harness.unwrap(harness.rpc.get_balance(harness.recipient_address).await);
    assert_eq!(
        sender_balance_after,
        INITIAL_SENDER_BALANCE - TRANSFER_AMOUNT
    );
    assert_eq!(
        recipient_balance_after,
        INITIAL_RECIPIENT_BALANCE + TRANSFER_AMOUNT
    );

    let sender_nonce_after = harness.unwrap(
        harness
            .rpc
            .get_transaction_count(harness.sender_address)
            .await,
    );
    assert_eq!(sender_nonce_after, nonce + 1);

    harness.unwrap(harness.compose.assert_service_running("ev-node"));
}

// ============================================================================
// Test Harness
// ============================================================================

struct StackHarness {
    compose: ComposeStack,
    rpc: JsonRpcClient,
    sender_key: SigningKey,
    sender_address: Address,
    recipient_address: Address,
    token_address: Address,
    first_block_number: u64,
    _temp_dir: TempDir,
}

impl StackHarness {
    fn unwrap<T>(&self, result: Result<T, String>) -> T {
        result.unwrap_or_else(|err| panic_with_logs(&self.compose, &err))
    }
}

async fn start_stack() -> StackHarness {
    ensure_docker_available();

    let repo_root = workspace_root();
    let evd_binary = PathBuf::from(env!("CARGO_BIN_EXE_evd"));
    let sender_key = parse_signing_key(SENDER_KEY_HEX);
    let sender_address = wallet_address(&sender_key);
    let recipient_address = wallet_address(&parse_signing_key(RECIPIENT_KEY_HEX));

    let temp_dir = TempDir::new().expect("create temp dir");
    let data_dir = temp_dir.path().join("data");
    let genesis_file = temp_dir.path().join("evd-genesis.json");
    let evnode_home = temp_dir.path().join("evnode-home");
    let evnode_passphrase_file = temp_dir.path().join("evnode-passphrase.txt");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    write_genesis_file(
        &genesis_file,
        sender_address,
        recipient_address,
        INITIAL_SENDER_BALANCE,
        INITIAL_RECIPIENT_BALANCE,
    );

    init_data_dir(&evd_binary, &genesis_file, &data_dir);
    init_evnode_home(&evnode_home, &evnode_passphrase_file);
    let token_address = load_token_address(data_dir.clone());
    let rpc_port = find_unused_port();

    let compose_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/docker-compose.e2e.yml");
    let env = vec![
        ("REPO_ROOT".into(), path_str(&repo_root)),
        ("RPC_PORT".into(), rpc_port.to_string()),
        ("DATA_DIR".into(), path_str(&data_dir)),
        ("GENESIS_FILE".into(), path_str(&genesis_file)),
        ("EVNODE_HOME".into(), path_str(&evnode_home)),
        (
            "EVNODE_PASSPHRASE_FILE".into(),
            path_str(&evnode_passphrase_file),
        ),
    ];
    let compose = ComposeStack::new(unique_project_name(), compose_file, env);
    compose.unwrap(compose.up());
    compose.unwrap(compose.wait_for_service_running("evd", POLL_TIMEOUT));
    let rpc = JsonRpcClient::new(format!("http://127.0.0.1:{rpc_port}"));
    poll_until!(
        &compose,
        "timed out waiting for evd JSON-RPC to become ready",
        rpc.chain_id().await.ok().map(|_| ())
    );
    compose.unwrap(compose.wait_for_service_running("ev-node", POLL_TIMEOUT));
    let first_block_number = poll_until!(
        &compose,
        "timed out waiting for ev-node to drive external block production",
        rpc.block_number().await.ok().filter(|&n| n >= 1)
    );

    StackHarness {
        compose,
        rpc,
        sender_key,
        sender_address,
        recipient_address,
        token_address,
        first_block_number,
        _temp_dir: temp_dir,
    }
}

// ============================================================================
// Docker Compose
// ============================================================================

struct ComposeStack {
    project_name: String,
    compose_file: PathBuf,
    env: Vec<(String, String)>,
}

impl ComposeStack {
    fn new(
        project_name: String,
        compose_file: PathBuf,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            project_name,
            compose_file,
            env,
        }
    }

    fn unwrap<T>(&self, result: Result<T, String>) -> T {
        result.unwrap_or_else(|err| panic_with_logs(self, &err))
    }

    fn up(&self) -> Result<(), String> {
        self.run(["up", "--build", "--detach"])
    }

    fn logs(&self) -> String {
        match self.run_output(["logs", "--no-color"]) {
            Ok(output) => format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(err) => format!("failed to read docker compose logs: {err}"),
        }
    }

    fn assert_service_running(&self, service: &str) -> Result<(), String> {
        if self.running_services()?.iter().any(|name| name == service) {
            return Ok(());
        }
        Err(format!("service '{service}' is not running"))
    }

    fn wait_for_service_running(&self, service: &str, timeout: Duration) -> Result<(), String> {
        let max_attempts = poll_attempts(timeout);
        for _ in 0..max_attempts {
            if self.running_services()?.iter().any(|name| name == service) {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        Err(format!(
            "timed out waiting for service '{service}' to reach running state"
        ))
    }

    fn running_services(&self) -> Result<Vec<String>, String> {
        let output = self.run_output(["ps", "--services", "--status", "running"])?;
        if !output.status.success() {
            return Err(format!(
                "docker compose ps failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    fn run<const N: usize>(&self, args: [&str; N]) -> Result<(), String> {
        let output = self.run_output(args)?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "docker compose {:?} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }

    fn run_output<const N: usize>(&self, args: [&str; N]) -> Result<Output, String> {
        let mut command = Command::new("docker");
        command.arg("compose");
        command.arg("-p").arg(&self.project_name);
        command.arg("-f").arg(&self.compose_file);
        command.args(args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
            .output()
            .map_err(|err| format!("spawn docker compose {:?}: {err}", args))
    }
}

impl Drop for ComposeStack {
    fn drop(&mut self) {
        let _ = self.run(["down", "--volumes", "--remove-orphans"]);
    }
}

// ============================================================================
// JSON-RPC Client
// ============================================================================

struct JsonRpcClient {
    client: reqwest::Client,
    url: String,
    next_id: AtomicU64,
}

impl JsonRpcClient {
    fn new(url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        Self {
            client,
            url,
            next_id: AtomicU64::new(1),
        }
    }

    async fn chain_id(&self) -> Result<u64, String> {
        let result = self.call("eth_chainId", json!([])).await?;
        parse_hex_u64_value(&result)
    }

    async fn block_number(&self) -> Result<u64, String> {
        let result = self.call("eth_blockNumber", json!([])).await?;
        parse_hex_u64_value(&result)
    }

    async fn get_balance(&self, address: Address) -> Result<u128, String> {
        let result = self
            .call("eth_getBalance", json!([format!("{address:#x}"), "latest"]))
            .await?;
        parse_hex_u128(&result)
    }

    async fn get_transaction_count(&self, address: Address) -> Result<u64, String> {
        let result = self
            .call(
                "eth_getTransactionCount",
                json!([format!("{address:#x}"), "latest"]),
            )
            .await?;
        parse_hex_u64_value(&result)
    }

    async fn send_raw_transaction(&self, raw_tx: &[u8]) -> Result<String, String> {
        let result = self
            .call(
                "eth_sendRawTransaction",
                json!([format!("0x{}", hex::encode(raw_tx))]),
            )
            .await?;
        result
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("eth_sendRawTransaction returned non-string result: {result}"))
    }

    async fn get_transaction_receipt(&self, tx_hash: &str) -> Result<Option<TxReceipt>, String> {
        let result = self
            .call("eth_getTransactionReceipt", json!([tx_hash]))
            .await?;
        if result.is_null() {
            return Ok(None);
        }
        serde_json::from_value(result)
            .map(Some)
            .map_err(|err| format!("decode eth_getTransactionReceipt result: {err}"))
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });
        let response = self
            .client
            .post(&self.url)
            .json(&payload)
            .send()
            .await
            .map_err(|err| format!("RPC {method} request failed: {err}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|err| format!("RPC {method} returned invalid JSON: {err}"))?;
        if !status.is_success() {
            return Err(format!("RPC {method} returned HTTP {status}: {body}"));
        }
        if let Some(error) = body.get("error") {
            return Err(format!("RPC {method} returned error: {error}"));
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| format!("RPC {method} response missing result: {body}"))
    }
}

#[derive(Deserialize)]
struct TxReceipt {
    status: Option<String>,
    #[serde(rename = "blockNumber")]
    block_number: Option<String>,
}

// ============================================================================
// Setup Helpers
// ============================================================================

fn ensure_docker_available() {
    let status = Command::new("docker")
        .args(["compose", "version"])
        .status()
        .expect("spawn docker compose version");
    assert!(status.success(), "docker compose is not available");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("resolve workspace root")
        .to_path_buf()
}

fn write_genesis_file(
    path: &Path,
    sender: Address,
    recipient: Address,
    sender_balance: u128,
    recipient_balance: u128,
) {
    let contents = json!({
        "token": {
            "name": "Evolve",
            "symbol": "EV",
            "decimals": 6,
            "icon_url": "https://example.com/token.png",
            "description": "Evolve test token"
        },
        "minter_id": 100002,
        "accounts": [
            {
                "eth_address": format!("{sender:#x}"),
                "balance": sender_balance
            },
            {
                "eth_address": format!("{recipient:#x}"),
                "balance": recipient_balance
            }
        ]
    });
    let bytes = serde_json::to_vec_pretty(&contents).expect("serialize genesis json");
    std::fs::write(path, bytes).expect("write genesis json");
}

fn init_data_dir(evd_binary: &Path, genesis_file: &Path, data_dir: &Path) {
    let status = Command::new(evd_binary)
        .args([
            "init",
            "--genesis-file",
            genesis_file.to_str().expect("genesis path utf8"),
            "--data-dir",
            data_dir.to_str().expect("data dir utf8"),
        ])
        .status()
        .expect("spawn evd init");
    assert!(status.success(), "evd init failed");
}

fn init_evnode_home(home_dir: &Path, passphrase_file: &Path) {
    std::fs::create_dir_all(home_dir).expect("create ev-node home dir");
    std::fs::write(passphrase_file, EVNODE_SIGNER_PASSPHRASE).expect("write ev-node passphrase");

    let home_mount = format!("{}:{EVNODE_HOME_IN_CONTAINER}", home_dir.display());
    let passphrase_mount = format!(
        "{}:{EVNODE_PASSPHRASE_IN_CONTAINER}:ro",
        passphrase_file.display()
    );
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &home_mount,
            "-v",
            &passphrase_mount,
            EVNODE_IMAGE,
            "init",
            "--home",
            EVNODE_HOME_IN_CONTAINER,
            "--evnode.node.aggregator",
            "--evnode.signer.passphrase_file",
            EVNODE_PASSPHRASE_IN_CONTAINER,
        ])
        .output()
        .expect("spawn ev-node init container");

    assert!(
        output.status.success(),
        "ev-node init failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Load the token address from persisted chain state.
///
/// Runs on a separate thread because `QmdbStorage` requires its own async runtime,
/// which cannot be nested within the test's tokio context.
fn load_token_address(data_dir: PathBuf) -> Address {
    std::thread::spawn(move || {
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
            let state = load_chain_state::<EvdGenesisResult, _>(&storage)
                .expect("load persisted chain state");
            tx.send(state.genesis_result.token)
                .expect("send token account id");
        });
        derive_runtime_contract_address(rx.recv().expect("receive token account id"))
    })
    .join()
    .expect("load token address thread panicked")
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("path utf8").to_owned()
}

fn unique_project_name() -> String {
    let counter = UNIQUE_PROJECT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("evd-e2e-{}-{counter}", std::process::id())
}

fn find_unused_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

// ============================================================================
// Transaction Helpers
// ============================================================================

fn build_transfer_tx(
    signing_key: &SigningKey,
    nonce: u64,
    token_address: Address,
    recipient: evolve_core::AccountId,
    amount: u128,
) -> Vec<u8> {
    let mut input = Vec::new();
    input.extend_from_slice(&keccak256(b"transfer")[..4]);
    input.extend_from_slice(&borsh::to_vec(&(recipient, amount)).expect("encode transfer args"));

    let tx = TxEip1559 {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 20_000_000_000,
        gas_limit: 150_000,
        to: TxKind::Call(token_address),
        value: U256::ZERO,
        input: Bytes::from(input),
        access_list: Default::default(),
    };
    let signature = sign_hash(signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);
    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);
    encoded
}

fn parse_signing_key(hex_key: &str) -> SigningKey {
    let bytes: [u8; 32] = hex::decode(hex_key)
        .expect("decode private key")
        .try_into()
        .expect("private key length");
    SigningKey::from_bytes((&bytes).into()).expect("construct signing key")
}

fn wallet_address(signing_key: &SigningKey) -> Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let hash = keccak256(&public_key.as_bytes()[1..]);
    Address::from_slice(&hash.as_slice()[12..])
}

// ============================================================================
// Hex Parsing
// ============================================================================

fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let raw = strip_hex_prefix(s);
    if raw.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(raw, 16).map_err(|err| format!("invalid hex u64 '{s}': {err}"))
}

fn parse_hex_u64_value(value: &Value) -> Result<u64, String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("expected hex string, got {value}"))?;
    parse_hex_u64(text)
}

fn parse_hex_u128(value: &Value) -> Result<u128, String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("expected hex string, got {value}"))?;
    let raw = strip_hex_prefix(text);
    if raw.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(raw, 16).map_err(|err| format!("invalid hex u128 '{text}': {err}"))
}

fn poll_attempts(timeout: Duration) -> u32 {
    let timeout_ms = timeout.as_millis();
    let poll_ms = POLL_INTERVAL.as_millis();
    let attempts = timeout_ms.div_ceil(poll_ms).max(1);
    u32::try_from(attempts).unwrap_or(u32::MAX)
}

fn panic_with_logs(compose: &ComposeStack, message: &str) -> ! {
    panic!("{message}\n\nDocker Compose logs:\n{}", compose.logs());
}
