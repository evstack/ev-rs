use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{keccak256, Address, Bytes, PrimitiveSignature, TxKind, B256, U256};
use clap::Parser;
use evolve_core::AccountId;
use evolve_tx_eth::derive_eth_eoa_account_id;
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use rand::RngCore;
use serde_json::{json, Value};
use tokio::time::Instant;
use tracing_subscriber::{fmt, EnvFilter};

const MAX_SIGNING_KEY_ATTEMPTS: usize = 16;

#[derive(Parser)]
#[command(name = "txload")]
#[command(about = "Tx-only RPC load tester for Evolve")]
#[command(version)]
struct Args {
    /// JSON-RPC URL for eth_sendRawTransaction
    #[arg(long, default_value = "http://127.0.0.1:8545")]
    rpc_url: String,

    /// Chain ID used for signing EIP-1559 transactions
    #[arg(long, default_value_t = 1)]
    chain_id: u64,

    /// Token account Ethereum address (0x...) that exposes `transfer`
    #[arg(long)]
    token_account: String,

    /// Funder private key (0x... or hex) that holds initial token balance
    #[arg(long)]
    funder_private_key: String,

    /// Number of worker wallets to create
    #[arg(long, default_value_t = 64)]
    workers: usize,

    /// Target aggregate TPS across all workers
    #[arg(long, default_value_t = 500)]
    target_tps: u64,

    /// Test duration in seconds
    #[arg(long, default_value_t = 30)]
    duration_secs: u64,

    /// Token amount each worker receives from the funder before the run
    #[arg(long, default_value_t = 100_000)]
    fanout_amount: u128,

    /// Token transfer amount per worker tx during the run
    #[arg(long, default_value_t = 1)]
    tx_amount: u128,

    /// Gas limit for generated EIP-1559 txs
    #[arg(long, default_value_t = 150_000)]
    gas_limit: u64,

    /// Max priority fee per gas (wei)
    #[arg(long, default_value_t = 1_000_000_000)]
    max_priority_fee_per_gas: u128,

    /// Max fee per gas (wei)
    #[arg(long, default_value_t = 20_000_000_000)]
    max_fee_per_gas: u128,

    /// Wait time after fanout before worker phase starts
    #[arg(long, default_value_t = 3)]
    funding_settle_secs: u64,

    /// HTTP timeout for individual RPC calls
    #[arg(long, default_value_t = 3000)]
    request_timeout_ms: u64,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
}

struct LoadtestConfig {
    rpc_url: String,
    chain_id: u64,
    token_account: Address,
    funder_key: SigningKey,
    workers: usize,
    target_tps: u64,
    duration_secs: u64,
    fanout_amount: u128,
    tx_amount: u128,
    gas_limit: u64,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    funding_settle_secs: u64,
    request_timeout_ms: u64,
}

struct WorkerWallet {
    signing_key: SigningKey,
    address: Address,
    account_id: AccountId,
    next_nonce: u64,
}

#[derive(Clone, Copy)]
struct TxBuildParams {
    chain_id: u64,
    to: Address,
    gas_limit: u64,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
}

struct JsonRpcClient {
    client: reqwest::Client,
    url: String,
    next_id: AtomicU64,
}

impl JsonRpcClient {
    fn new(url: String, timeout: Duration) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Self {
            client,
            url,
            next_id: AtomicU64::new(1),
        })
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
            .map_err(|e| format!("RPC {method} request failed: {e}"))?;

        let status = response.status();
        let value: Value = response
            .json()
            .await
            .map_err(|e| format!("RPC {method} invalid JSON response: {e}"))?;

        if !status.is_success() {
            return Err(format!("RPC {method} HTTP {status}: {value}"));
        }
        if let Some(err) = value.get("error") {
            return Err(format!("RPC {method} error: {err}"));
        }

        value
            .get("result")
            .cloned()
            .ok_or_else(|| format!("RPC {method} missing result field"))
    }

    async fn get_transaction_count(&self, address: Address) -> Result<u64, String> {
        let result = self
            .call(
                "eth_getTransactionCount",
                json!([format!("{address:#x}"), "latest"]),
            )
            .await?;
        let value = result
            .as_str()
            .ok_or_else(|| format!("eth_getTransactionCount returned non-string: {result}"))?;
        parse_hex_u64(value)
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
            .ok_or_else(|| format!("eth_sendRawTransaction returned non-string: {result}"))
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    init_tracing(&args.log_level);

    let token_account = Address::from_str(&args.token_account).expect("invalid token account");
    let funder_key = parse_signing_key(&args.funder_private_key).expect("invalid funder key");
    let config = LoadtestConfig {
        rpc_url: args.rpc_url,
        chain_id: args.chain_id,
        token_account,
        funder_key,
        workers: args.workers,
        target_tps: args.target_tps,
        duration_secs: args.duration_secs,
        fanout_amount: args.fanout_amount,
        tx_amount: args.tx_amount,
        gas_limit: args.gas_limit,
        max_priority_fee_per_gas: args.max_priority_fee_per_gas,
        max_fee_per_gas: args.max_fee_per_gas,
        funding_settle_secs: args.funding_settle_secs,
        request_timeout_ms: args.request_timeout_ms,
    };

    if let Err(err) = run_loadtest(config).await {
        eprintln!("loadtest failed: {err}");
        std::process::exit(1);
    }
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    fmt().with_env_filter(filter).init();
}

fn parse_signing_key(input: &str) -> Result<SigningKey, String> {
    let trimmed = input.trim();
    let hex_str = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let decoded = hex::decode(hex_str).map_err(|e| format!("invalid private key hex: {e}"))?;
    let bytes: [u8; 32] = decoded
        .try_into()
        .map_err(|_| "private key must be 32 bytes".to_string())?;
    SigningKey::from_bytes((&bytes).into()).map_err(|e| format!("invalid private key: {e}"))
}

fn random_signing_key() -> SigningKey {
    let mut rng = rand::thread_rng();
    for _ in 0..MAX_SIGNING_KEY_ATTEMPTS {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        if let Ok(key) = SigningKey::from_bytes((&bytes).into()) {
            return key;
        }
    }
    panic!("failed to generate signing key");
}

fn wallet_address(signing_key: &SigningKey) -> Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let hash = keccak256(&public_key.as_bytes()[1..]);
    Address::from_slice(&hash.as_slice()[12..])
}

fn sign_hash(signing_key: &SigningKey, hash: B256) -> PrimitiveSignature {
    let (sig, recovery_id): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
        .sign_prehash(hash.as_ref())
        .expect("signing failed");
    let r = U256::from_be_slice(sig.r().to_bytes().as_slice());
    let s = U256::from_be_slice(sig.s().to_bytes().as_slice());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

fn create_signed_tx(
    signing_key: &SigningKey,
    nonce: u64,
    input: Vec<u8>,
    tx_params: TxBuildParams,
) -> Vec<u8> {
    let tx = TxEip1559 {
        chain_id: tx_params.chain_id,
        nonce,
        max_priority_fee_per_gas: tx_params.max_priority_fee_per_gas,
        max_fee_per_gas: tx_params.max_fee_per_gas,
        gas_limit: tx_params.gas_limit,
        to: TxKind::Call(tx_params.to),
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

fn transfer_selector() -> [u8; 4] {
    let hash = keccak256("transfer".as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn build_transfer_calldata(recipient: AccountId, amount: u128) -> Result<Vec<u8>, String> {
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&transfer_selector());
    let args = borsh::to_vec(&(recipient, amount))
        .map_err(|e| format!("failed to encode transfer args: {e}"))?;
    calldata.extend_from_slice(&args);
    Ok(calldata)
}

fn parse_hex_u64(input: &str) -> Result<u64, String> {
    let value = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .unwrap_or(input);
    if value.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(value, 16).map_err(|e| format!("invalid hex u64 '{input}': {e}"))
}

async fn run_loadtest(config: LoadtestConfig) -> Result<(), String> {
    if config.workers < 2 {
        return Err("workers must be >= 2".to_string());
    }
    if config.target_tps == 0 {
        return Err("target_tps must be > 0".to_string());
    }
    if config.duration_secs == 0 {
        return Err("duration_secs must be > 0".to_string());
    }

    let total_target_txs = config
        .target_tps
        .checked_mul(config.duration_secs)
        .ok_or_else(|| "target_tps * duration_secs overflowed".to_string())?;
    let rpc = Arc::new(JsonRpcClient::new(
        config.rpc_url.clone(),
        Duration::from_millis(config.request_timeout_ms),
    )?);
    let funder_address = wallet_address(&config.funder_key);

    tracing::info!("=== tx-only load test ===");
    tracing::info!("RPC URL: {}", config.rpc_url);
    tracing::info!("Chain ID: {}", config.chain_id);
    tracing::info!("Token account: {:#x}", config.token_account);
    tracing::info!("Funder address: {funder_address:#x}");
    tracing::info!(
        "Workers: {}, Target TPS: {}, Duration: {}s, Target txs: {}",
        config.workers,
        config.target_tps,
        config.duration_secs,
        total_target_txs
    );

    let tx_params = TxBuildParams {
        chain_id: config.chain_id,
        to: config.token_account,
        gas_limit: config.gas_limit,
        max_priority_fee_per_gas: config.max_priority_fee_per_gas,
        max_fee_per_gas: config.max_fee_per_gas,
    };

    let mut wallets = Vec::with_capacity(config.workers);
    let mut seen = BTreeMap::new();
    for _ in 0..config.workers.saturating_mul(32) {
        if wallets.len() >= config.workers {
            break;
        }
        let key = random_signing_key();
        let address = wallet_address(&key);
        if seen.insert(address, ()).is_none() {
            wallets.push(WorkerWallet {
                signing_key: key,
                address,
                account_id: derive_eth_eoa_account_id(address),
                next_nonce: 0,
            });
        }
    }
    if wallets.len() != config.workers {
        return Err(format!(
            "failed to create {} unique worker wallets",
            config.workers
        ));
    }

    tracing::info!("Funding {} workers from funder wallet...", config.workers);
    let mut funder_nonce = rpc.get_transaction_count(funder_address).await?;
    for (idx, wallet) in wallets.iter().enumerate() {
        let calldata = build_transfer_calldata(wallet.account_id, config.fanout_amount)?;
        let raw_tx = create_signed_tx(&config.funder_key, funder_nonce, calldata, tx_params);
        rpc.send_raw_transaction(&raw_tx)
            .await
            .map_err(|e| format!("funding tx {} failed: {e}", idx + 1))?;
        funder_nonce = funder_nonce.saturating_add(1);

        if (idx + 1) % 100 == 0 || idx + 1 == config.workers {
            tracing::info!("Funding progress: {}/{}", idx + 1, config.workers);
        }
    }

    if config.funding_settle_secs > 0 {
        tokio::time::sleep(Duration::from_secs(config.funding_settle_secs)).await;
    }

    for wallet in &mut wallets {
        wallet.next_nonce = rpc.get_transaction_count(wallet.address).await?;
    }

    let recipients: Arc<Vec<AccountId>> = Arc::new(wallets.iter().map(|w| w.account_id).collect());
    let interval_nanos = (1_000_000_000u64 / config.target_tps).max(1);
    let counter = Arc::new(AtomicU64::new(0));
    let success = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(tokio::sync::Mutex::new(BTreeMap::<String, u64>::new()));
    let phase_start = Instant::now();
    let first_slot = phase_start + Duration::from_millis(250);

    let mut handles = Vec::with_capacity(wallets.len());
    for (worker_idx, mut wallet) in wallets.into_iter().enumerate() {
        let rpc = Arc::clone(&rpc);
        let recipients = Arc::clone(&recipients);
        let counter = Arc::clone(&counter);
        let success = Arc::clone(&success);
        let failed = Arc::clone(&failed);
        let errors = Arc::clone(&errors);
        let tx_amount = config.tx_amount;

        handles.push(tokio::spawn(async move {
            loop {
                let slot = counter.fetch_add(1, Ordering::Relaxed);
                if slot >= total_target_txs {
                    break;
                }

                let scheduled_at = first_slot + Duration::from_nanos(interval_nanos * slot);
                let now = Instant::now();
                if scheduled_at > now {
                    tokio::time::sleep_until(scheduled_at).await;
                }

                let mut recipient_idx =
                    (slot as usize).wrapping_add(worker_idx).wrapping_add(1) % recipients.len();
                if recipients.len() > 1 && recipient_idx == worker_idx {
                    recipient_idx = (recipient_idx + 1) % recipients.len();
                }
                let recipient = recipients[recipient_idx];

                let calldata = match build_transfer_calldata(recipient, tx_amount) {
                    Ok(data) => data,
                    Err(err) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let mut map = errors.lock().await;
                        *map.entry(err).or_insert(0) += 1;
                        continue;
                    }
                };

                let raw_tx =
                    create_signed_tx(&wallet.signing_key, wallet.next_nonce, calldata, tx_params);
                wallet.next_nonce = wallet.next_nonce.saturating_add(1);

                match rpc.send_raw_transaction(&raw_tx).await {
                    Ok(_) => {
                        success.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(err) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let mut map = errors.lock().await;
                        *map.entry(err).or_insert(0) += 1;
                    }
                }
            }
        }));
    }
    for handle in handles {
        handle
            .await
            .map_err(|e| format!("loadtest worker task failed to join: {e}"))?;
    }

    let elapsed = phase_start.elapsed().as_secs_f64();
    let success_count = success.load(Ordering::Relaxed);
    let failed_count = failed.load(Ordering::Relaxed);
    let attempted = success_count + failed_count;
    let attempted_tps = if elapsed > 0.0 {
        attempted as f64 / elapsed
    } else {
        0.0
    };
    let accepted_tps = if elapsed > 0.0 {
        success_count as f64 / elapsed
    } else {
        0.0
    };

    println!();
    println!("=== Load test results ===");
    println!("attempted:       {attempted}");
    println!("accepted:        {success_count}");
    println!("errors:          {failed_count}");
    println!("elapsed_secs:    {:.3}", elapsed);
    println!("attempted_tps:   {:.2}", attempted_tps);
    println!("accepted_tps:    {:.2}", accepted_tps);

    let mut error_entries: Vec<(String, u64)> = errors
        .lock()
        .await
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    error_entries.sort_by(|a, b| b.1.cmp(&a.1));
    if !error_entries.is_empty() {
        println!("top_errors:");
        for (idx, (msg, count)) in error_entries.into_iter().take(10).enumerate() {
            println!("  {}. {} x {}", idx + 1, count, msg);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_key() -> SigningKey {
        parse_signing_key("0x1111111111111111111111111111111111111111111111111111111111111111")
            .expect("fixed key should parse")
    }

    #[test]
    fn parse_signing_key_accepts_prefixed_and_unprefixed_hex() {
        let with_prefix =
            parse_signing_key("0x1111111111111111111111111111111111111111111111111111111111111111")
                .expect("with-prefix key should parse");
        let without_prefix =
            parse_signing_key("1111111111111111111111111111111111111111111111111111111111111111")
                .expect("without-prefix key should parse");

        assert_eq!(
            wallet_address(&with_prefix),
            wallet_address(&without_prefix)
        );
    }

    #[test]
    fn parse_signing_key_rejects_wrong_length() {
        let err = parse_signing_key("0x1234").expect_err("short key must fail");
        assert!(err.contains("private key must be 32 bytes"));
    }

    #[test]
    fn parse_hex_u64_handles_common_cases() {
        assert_eq!(parse_hex_u64("0x0").unwrap(), 0);
        assert_eq!(parse_hex_u64("0Xff").unwrap(), 255);
        assert_eq!(parse_hex_u64("a").unwrap(), 10);
        assert_eq!(parse_hex_u64("").unwrap(), 0);
        assert!(parse_hex_u64("0xzz").is_err());
    }

    #[test]
    fn build_transfer_calldata_has_selector_and_borsh_args() {
        let recipient = AccountId::from_bytes([0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,42]);
        let amount = 999_u128;

        let calldata = build_transfer_calldata(recipient, amount).unwrap();
        assert!(calldata.len() > 4);
        assert_eq!(&calldata[0..4], &transfer_selector());

        let decoded: (AccountId, u128) = borsh::from_slice(&calldata[4..]).unwrap();
        assert_eq!(decoded.0, recipient);
        assert_eq!(decoded.1, amount);
    }

    #[test]
    fn create_signed_tx_is_type_2_and_deterministic() {
        let key = fixed_key();
        let params = TxBuildParams {
            chain_id: 1,
            to: wallet_address(&key),
            gas_limit: 21_000,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 2_000_000_000,
        };
        let input = vec![1, 2, 3, 4];

        let tx1 = create_signed_tx(&key, 7, input.clone(), params);
        let tx2 = create_signed_tx(&key, 7, input, params);

        assert!(!tx1.is_empty());
        assert_eq!(tx1[0], 0x02);
        assert_eq!(tx1, tx2);
    }

    #[tokio::test]
    async fn run_loadtest_validates_config_before_network() {
        let key = fixed_key();
        let token_account = wallet_address(&key);

        let base = LoadtestConfig {
            rpc_url: "http://127.0.0.1:8545".to_string(),
            chain_id: 1,
            token_account,
            funder_key: key,
            workers: 2,
            target_tps: 1,
            duration_secs: 1,
            fanout_amount: 1,
            tx_amount: 1,
            gas_limit: 21_000,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            funding_settle_secs: 0,
            request_timeout_ms: 100,
        };

        let err_workers = run_loadtest(LoadtestConfig { workers: 1, ..base })
            .await
            .expect_err("workers < 2 should fail");
        assert!(err_workers.contains("workers must be >= 2"));

        let key2 = fixed_key();
        let token2 = wallet_address(&key2);
        let err_tps = run_loadtest(LoadtestConfig {
            rpc_url: "http://127.0.0.1:8545".to_string(),
            chain_id: 1,
            token_account: token2,
            funder_key: key2,
            workers: 2,
            target_tps: 0,
            duration_secs: 1,
            fanout_amount: 1,
            tx_amount: 1,
            gas_limit: 21_000,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            funding_settle_secs: 0,
            request_timeout_ms: 100,
        })
        .await
        .expect_err("target_tps == 0 should fail");
        assert!(err_tps.contains("target_tps must be > 0"));
    }
}
