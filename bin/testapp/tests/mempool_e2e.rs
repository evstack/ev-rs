//! E2E test for mempool and dev consensus integration.
//!
//! This test proves the complete token transfer flow:
//! 1. Sign an Ethereum transaction targeting the token account
//! 2. Submit to mempool
//! 3. DevConsensus produces a block
//! 4. Transaction is authenticated and executed
//! 5. Token balances are updated correctly

#![allow(unexpected_cfgs)]

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{keccak256 as keccak256_b256, Address, Bytes, TxKind, B256, U256};
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_consensus::{SigningKey as Ed25519SigningKey, VerificationKey};
use evolve_core::encoding::Encodable;
use evolve_core::{define_error, AccountId, ErrorCode, Message, ReadonlyKV, SdkResult};
use evolve_mempool::MempoolTx;
use evolve_node::{build_dev_node_with_mempool, DevNodeMempoolHandles};
use evolve_server::DevConfig;
use evolve_simulator::{generate_signing_key, SimConfig, Simulator};
use evolve_stf_traits::WritableAccountsCodeStorage;
use evolve_storage::{CommitHash, Operation};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis, install_account_codes,
    EthGenesisAccounts, MempoolStf,
};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_tx_eth::{
    derive_runtime_contract_address, sender_types, EthGateway, EthIntentPayload,
    SignatureVerifierDyn, TxContext, TxEnvelope, TxPayload, TypedTransaction,
};
use k256::ecdsa::{SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use std::sync::RwLock;

type TestNodeHandles =
    DevNodeMempoolHandles<MempoolStf, AsyncMockStorage, AccountStorageMock, TxContext>;

#[derive(Clone, BorshSerialize, BorshDeserialize)]
struct Ed25519AuthPayload {
    nonce: u64,
    signature: [u8; 64],
}

#[derive(Clone, BorshSerialize, BorshDeserialize)]
struct Ed25519EthIntentProof {
    public_key: [u8; 32],
    signature: [u8; 64],
}

struct Ed25519PayloadVerifier;

define_error!(
    ERR_ED25519_PAYLOAD_NOT_CUSTOM,
    0x75,
    "ed25519 verifier expects custom payload"
);
define_error!(
    ERR_ED25519_INTENT_DECODE,
    0x76,
    "failed to decode ed25519 intent payload"
);
define_error!(
    ERR_ED25519_PROOF_DECODE,
    0x77,
    "failed to decode ed25519 auth proof"
);
define_error!(
    ERR_ED25519_ENVELOPE_DECODE,
    0x78,
    "failed to decode ed25519 envelope"
);
define_error!(
    ERR_ED25519_REQUEST_MISSING,
    0x79,
    "missing invoke request in ed25519 envelope"
);
define_error!(
    ERR_ED25519_REQUEST_ENCODE,
    0x7A,
    "failed to encode invoke request"
);
define_error!(ERR_ED25519_PUBLIC_KEY, 0x7B, "invalid ed25519 public key");
define_error!(ERR_ED25519_SIGNATURE, 0x7C, "invalid ed25519 signature");

impl SignatureVerifierDyn for Ed25519PayloadVerifier {
    fn verify(&self, payload: &TxPayload) -> SdkResult<()> {
        let TxPayload::Custom(bytes) = payload else {
            return Err(ERR_ED25519_PAYLOAD_NOT_CUSTOM);
        };
        let intent: EthIntentPayload =
            borsh::from_slice(bytes).map_err(|_| ERR_ED25519_INTENT_DECODE)?;
        let decoded: Ed25519EthIntentProof =
            borsh::from_slice(&intent.auth_proof).map_err(|_| ERR_ED25519_PROOF_DECODE)?;
        let envelope = intent
            .decode_envelope()
            .map_err(|_| ERR_ED25519_ENVELOPE_DECODE)?;
        let invoke_request = envelope
            .to_invoke_requests()
            .into_iter()
            .next()
            .ok_or(ERR_ED25519_REQUEST_MISSING)?;
        let request_digest = keccak256(
            &invoke_request
                .encode()
                .map_err(|_| ERR_ED25519_REQUEST_ENCODE)?,
        );
        let public_key =
            VerificationKey::try_from(decoded.public_key).map_err(|_| ERR_ED25519_PUBLIC_KEY)?;
        let signature = ed25519_consensus::Signature::from(decoded.signature);
        public_key
            .verify(&signature, &request_digest)
            .map_err(|_| ERR_ED25519_SIGNATURE)
    }
}

#[evolve_core::account_impl(Ed25519AuthAccount)]
mod ed25519_auth_account {
    use super::{keccak256, Ed25519AuthPayload};
    use core::convert::TryFrom;
    use ed25519_consensus::{Signature, VerificationKey};
    use evolve_authentication::auth_interface::AuthenticationInterface;
    use evolve_collections::item::Item;
    use evolve_core::encoding::Encodable;
    use evolve_core::{define_error, Environment, Message, SdkResult};
    use evolve_macros::{exec, init, query};
    use evolve_stf_traits::Transaction as _;
    use evolve_tx_eth::TxContext;

    define_error!(
        ERR_INVALID_AUTH_PAYLOAD,
        0x80,
        "invalid ed25519 auth payload"
    );
    define_error!(ERR_NONCE_MISMATCH, 0x81, "ed25519 nonce mismatch");
    define_error!(ERR_INVALID_PUBLIC_KEY, 0x82, "invalid ed25519 public key");
    define_error!(
        ERR_INVALID_SIGNATURE,
        0x83,
        "invalid ed25519 account signature"
    );

    pub struct Ed25519AuthAccount {
        pub nonce: Item<u64>,
        pub public_key: Item<[u8; 32]>,
    }

    impl Default for Ed25519AuthAccount {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Ed25519AuthAccount {
        pub const fn new() -> Self {
            Self {
                nonce: Item::new(0),
                public_key: Item::new(1),
            }
        }

        #[init]
        pub fn initialize(&self, public_key: [u8; 32], env: &mut dyn Environment) -> SdkResult<()> {
            self.nonce.set(&0, env)?;
            self.public_key.set(&public_key, env)?;
            Ok(())
        }

        #[query]
        pub fn get_nonce(&self, env: &mut dyn evolve_core::EnvironmentQuery) -> SdkResult<u64> {
            Ok(self.nonce.may_get(env)?.unwrap_or(0))
        }
    }

    impl AuthenticationInterface for Ed25519AuthAccount {
        #[exec]
        fn authenticate(&self, tx: Message, env: &mut dyn Environment) -> SdkResult<()> {
            let tx_context: TxContext = tx.get().map_err(|_| ERR_INVALID_AUTH_PAYLOAD)?;
            let payload: Ed25519AuthPayload = tx_context
                .account_authentication_payload()
                .get()
                .map_err(|_| ERR_INVALID_AUTH_PAYLOAD)?;
            let current_nonce = self.nonce.may_get(env)?.unwrap_or(0);
            if payload.nonce != current_nonce {
                return Err(ERR_NONCE_MISMATCH);
            }
            let request_digest = keccak256(
                &tx_context
                    .request()
                    .encode()
                    .map_err(|_| ERR_INVALID_AUTH_PAYLOAD)?,
            );

            let pubkey_bytes = self
                .public_key
                .may_get(env)?
                .ok_or(ERR_INVALID_PUBLIC_KEY)?;
            let verify_key =
                VerificationKey::try_from(pubkey_bytes).map_err(|_| ERR_INVALID_PUBLIC_KEY)?;
            let signature = Signature::from(payload.signature);
            verify_key
                .verify(&signature, &request_digest)
                .map_err(|_| ERR_INVALID_SIGNATURE)?;

            self.nonce.set(&(current_nonce + 1), env)?;
            Ok(())
        }
    }
}

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Mock storage with async Storage trait implementation.
struct AsyncMockStorage {
    data: RwLock<BTreeMap<Vec<u8>, Vec<u8>>>,
}

#[allow(dead_code)]
impl AsyncMockStorage {
    fn new() -> Self {
        Self {
            data: RwLock::new(BTreeMap::new()),
        }
    }

    fn from_changes(changes: impl IntoIterator<Item = evolve_stf_traits::StateChange>) -> Self {
        let mut data = BTreeMap::new();
        for change in changes {
            match change {
                evolve_stf_traits::StateChange::Set { key, value } => {
                    data.insert(key, value);
                }
                evolve_stf_traits::StateChange::Remove { key } => {
                    data.remove(&key);
                }
            }
        }
        Self {
            data: RwLock::new(data),
        }
    }

    /// Apply state changes to existing storage (merging with existing data).
    fn apply_changes(&self, changes: impl IntoIterator<Item = evolve_stf_traits::StateChange>) {
        let mut data = self.data.write().unwrap();
        for change in changes {
            match change {
                evolve_stf_traits::StateChange::Set { key, value } => {
                    data.insert(key, value);
                }
                evolve_stf_traits::StateChange::Remove { key } => {
                    data.remove(&key);
                }
            }
        }
    }

    /// Pre-register an account's code identifier in storage.
    fn register_account_code(&self, account_id: AccountId, code_id: &str) {
        use evolve_core::runtime_api::ACCOUNT_IDENTIFIER_PREFIX;
        use evolve_core::Message;

        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account_id.as_bytes());
        let value = Message::new(&code_id.to_string())
            .unwrap()
            .into_bytes()
            .unwrap();
        self.data.write().unwrap().insert(key, value);
    }

    /// Initialize an EthEoaAccount's storage (nonce and eth_address).
    fn init_eth_eoa_storage(&self, account_id: AccountId, eth_address: [u8; 20]) {
        // Storage keys are: account_id + prefix (u8)
        // Item::new(0) = nonce, Item::new(1) = eth_address
        let mut data = self.data.write().unwrap();

        let mut nonce_key = account_id.as_bytes().to_vec();
        nonce_key.push(0u8);
        data.insert(
            nonce_key,
            Message::new(&0u64).unwrap().into_bytes().unwrap(),
        );

        let mut addr_key = account_id.as_bytes().to_vec();
        addr_key.push(1u8);
        data.insert(
            addr_key,
            Message::new(&eth_address).unwrap().into_bytes().unwrap(),
        );
    }

    /// Initialize an Ed25519AuthAccount's storage (nonce and public key).
    fn init_ed25519_auth_storage(&self, account_id: AccountId, public_key: [u8; 32]) {
        // Storage keys are: account_id + prefix (u8)
        // Item::new(0) = nonce, Item::new(1) = public key
        let mut data = self.data.write().unwrap();

        let mut nonce_key = account_id.as_bytes().to_vec();
        nonce_key.push(0u8);
        data.insert(
            nonce_key,
            Message::new(&0u64).unwrap().into_bytes().unwrap(),
        );

        let mut pubkey_key = account_id.as_bytes().to_vec();
        pubkey_key.push(1u8);
        data.insert(
            pubkey_key,
            Message::new(&public_key).unwrap().into_bytes().unwrap(),
        );
    }

    /// Set token balance directly in storage for a specific account.
    fn set_token_balance(&self, token_account_id: AccountId, account_id: AccountId, balance: u128) {
        let mut key = token_account_id.as_bytes().to_vec();
        key.push(1u8); // Token::balances storage prefix
        key.extend(account_id.encode().expect("encode account id"));
        let value = Message::new(&balance).unwrap().into_bytes().unwrap();
        self.data.write().unwrap().insert(key, value);
    }
}

impl Clone for AsyncMockStorage {
    fn clone(&self) -> Self {
        Self {
            data: RwLock::new(self.data.read().unwrap().clone()),
        }
    }
}

impl ReadonlyKV for AsyncMockStorage {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode> {
        Ok(self.data.read().unwrap().get(key).cloned())
    }
}

#[async_trait(?Send)]
impl evolve_storage::Storage for AsyncMockStorage {
    async fn commit(&self) -> Result<CommitHash, ErrorCode> {
        Ok(CommitHash::new([0u8; 32]))
    }

    async fn batch(&self, operations: Vec<Operation>) -> Result<(), ErrorCode> {
        let mut data = self.data.write().unwrap();
        for op in operations {
            match op {
                Operation::Set { key, value } => {
                    data.insert(key, value);
                }
                Operation::Remove { key } => {
                    data.remove(&key);
                }
            }
        }
        Ok(())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn keccak256(data: &[u8]) -> [u8; 32] {
    keccak256_b256(data).0
}

/// Compute function selector from function name (first 4 bytes of keccak256).
fn compute_selector(fn_name: &str) -> [u8; 4] {
    let hash = keccak256(fn_name.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

use evolve_tx_eth::sign_hash;

/// Get Ethereum address from signing key.
fn get_address(signing_key: &SigningKey) -> alloy_primitives::Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let public_key_bytes = &public_key.as_bytes()[1..]; // Skip the 0x04 prefix
    let hash = keccak256(public_key_bytes);
    alloy_primitives::Address::from_slice(&hash[12..])
}

/// Create a signed EIP-1559 transaction.
fn create_signed_tx(
    signing_key: &SigningKey,
    chain_id: u64,
    nonce: u64,
    to: alloy_primitives::Address,
    value: U256,
    input: Bytes,
) -> Vec<u8> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1_000_000_000, // 1 gwei
        max_fee_per_gas: 20_000_000_000,         // 20 gwei
        gas_limit: 100_000,
        to: TxKind::Call(to),
        value,
        input,
        access_list: Default::default(),
    };

    let signature = sign_hash(signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    // Encode with EIP-2718 type prefix
    let mut encoded = vec![0x02]; // EIP-1559 type prefix
    signed.rlp_encode(&mut encoded);
    encoded
}

fn read_nonce<S: ReadonlyKV>(storage: &S, account_id: AccountId) -> u64 {
    use evolve_core::Message;

    let mut nonce_key = account_id.as_bytes().to_vec();
    nonce_key.push(0u8);
    match storage.get(&nonce_key).expect("read nonce") {
        Some(value) => Message::from_bytes(value)
            .get::<u64>()
            .expect("decode nonce"),
        None => 0,
    }
}

fn read_token_balance<S: ReadonlyKV>(
    storage: &S,
    token_account_id: AccountId,
    account_id: AccountId,
) -> u128 {
    use evolve_core::encoding::Encodable;
    use evolve_core::Message;

    let mut key = token_account_id.as_bytes().to_vec();
    key.push(1u8); // Token::balances storage prefix
    key.extend(account_id.encode().expect("encode account id"));

    match storage.get(&key).expect("read balance") {
        Some(value) => Message::from_bytes(value)
            .get::<u128>()
            .expect("decode balance"),
        None => 0,
    }
}

// ============================================================================
// E2E Test
// ============================================================================

fn deterministic_signing_keys() -> (SigningKey, SigningKey) {
    let mut simulator = Simulator::new(0xD15E_A5E5, SimConfig::default());
    let alice_key = generate_signing_key(&mut simulator, 64).expect("alice signing key");
    let bob_key = generate_signing_key(&mut simulator, 64).expect("bob signing key");
    (alice_key, bob_key)
}

fn setup_genesis(
    chain_id: u64,
    alice_address: Address,
    bob_address: Address,
) -> (TestNodeHandles, EthGenesisAccounts, AccountId, AccountId) {
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    let init_storage = AsyncMockStorage::new();
    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config.clone(), AccountId::from_u64(0));

    let (genesis_state, genesis_accounts) = do_eth_genesis(
        &stf,
        &codes,
        &init_storage,
        alice_address.into(),
        bob_address.into(),
    )
    .expect("genesis should succeed");

    let genesis_changes = genesis_state.into_changes().expect("get changes");
    init_storage.apply_changes(genesis_changes);

    let stf = build_mempool_stf(gas_config, genesis_accounts.scheduler);
    let config = DevConfig {
        block_interval: None,
        gas_limit: 30_000_000,
        initial_height: 1,
        chain_id,
    };
    let handles = build_dev_node_with_mempool(stf, init_storage, codes, config);

    let alice_account_id =
        evolve_tx_eth::lookup_account_id_in_storage(handles.dev.storage(), alice_address)
            .expect("lookup alice id")
            .expect("alice id exists");
    let bob_account_id =
        evolve_tx_eth::lookup_account_id_in_storage(handles.dev.storage(), bob_address)
            .expect("lookup bob id")
            .expect("bob id exists");

    (handles, genesis_accounts, alice_account_id, bob_account_id)
}

fn setup_genesis_with_ed25519_sender(
    chain_id: u64,
    alice_address: Address,
    bob_address: Address,
    sender_account_id: AccountId,
    sender_public_key: [u8; 32],
    sender_initial_balance: u128,
) -> (TestNodeHandles, EthGenesisAccounts, AccountId, AccountId) {
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);
    codes
        .add_code(ed25519_auth_account::Ed25519AuthAccount::new())
        .expect("install ed25519 auth account code");

    let init_storage = AsyncMockStorage::new();
    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config.clone(), AccountId::from_u64(0));

    let (genesis_state, genesis_accounts) = do_eth_genesis(
        &stf,
        &codes,
        &init_storage,
        alice_address.into(),
        bob_address.into(),
    )
    .expect("genesis should succeed");

    let genesis_changes = genesis_state.into_changes().expect("get changes");
    init_storage.apply_changes(genesis_changes);
    init_storage.register_account_code(sender_account_id, "Ed25519AuthAccount");
    init_storage.init_ed25519_auth_storage(sender_account_id, sender_public_key);
    init_storage.set_token_balance(
        genesis_accounts.evolve,
        sender_account_id,
        sender_initial_balance,
    );

    let stf = build_mempool_stf(gas_config, genesis_accounts.scheduler);
    let config = DevConfig {
        block_interval: None,
        gas_limit: 30_000_000,
        initial_height: 1,
        chain_id,
    };
    let handles = build_dev_node_with_mempool(stf, init_storage, codes, config);

    let alice_account_id =
        evolve_tx_eth::lookup_account_id_in_storage(handles.dev.storage(), alice_address)
            .expect("lookup alice id")
            .expect("alice id exists");
    let bob_account_id =
        evolve_tx_eth::lookup_account_id_in_storage(handles.dev.storage(), bob_address)
            .expect("lookup bob id")
            .expect("bob id exists");

    (handles, genesis_accounts, alice_account_id, bob_account_id)
}

fn build_transfer_tx(
    alice_key: &SigningKey,
    chain_id: u64,
    nonce: u64,
    token_address: Address,
    bob_account_id: AccountId,
    transfer_amount: u128,
) -> Vec<u8> {
    let selector = compute_selector("transfer");
    let args = borsh::to_vec(&(bob_account_id, transfer_amount)).expect("encode args");
    let mut calldata = Vec::with_capacity(4 + args.len());
    calldata.extend_from_slice(&selector);
    calldata.extend_from_slice(&args);

    create_signed_tx(
        alice_key,
        chain_id,
        nonce,
        token_address,
        U256::ZERO,
        Bytes::from(calldata),
    )
}

struct Ed25519CustomTxBuildInput<'a> {
    tx_template_signer: &'a SigningKey,
    auth_signer: &'a Ed25519SigningKey,
    chain_id: u64,
    nonce: u64,
    token_address: Address,
    recipient_account_id: AccountId,
    transfer_amount: u128,
    sender_account_id: AccountId,
}

fn build_ed25519_custom_tx_context(input: Ed25519CustomTxBuildInput<'_>) -> TxContext {
    let template_raw_tx = build_transfer_tx(
        input.tx_template_signer,
        input.chain_id,
        input.nonce,
        input.token_address,
        input.recipient_account_id,
        input.transfer_amount,
    );
    let envelope = TxEnvelope::decode(&template_raw_tx).expect("decode transfer tx envelope");
    let invoke_request = envelope
        .to_invoke_requests()
        .into_iter()
        .next()
        .expect("expected transfer invoke request");
    let request_digest = keccak256(
        &invoke_request
            .encode()
            .expect("encode invoke request for digest"),
    );
    let signature = input.auth_signer.sign(&request_digest).to_bytes();
    let auth_payload = Ed25519AuthPayload {
        nonce: input.nonce,
        signature,
    };
    let proof = Ed25519EthIntentProof {
        public_key: input.auth_signer.verification_key().to_bytes(),
        signature,
    };
    let intent = EthIntentPayload {
        envelope: template_raw_tx,
        auth_proof: borsh::to_vec(&proof).expect("encode intent proof"),
    };

    TxContext::from_eth_intent(
        sender_types::CUSTOM,
        intent,
        input.sender_account_id,
        input.sender_account_id.as_bytes().to_vec(),
        Message::new(&auth_payload).expect("encode auth payload"),
        0,
    )
    .expect("construct custom tx context from eth intent")
}

fn build_ed25519_custom_wire_tx(input: Ed25519CustomTxBuildInput<'_>) -> Vec<u8> {
    build_ed25519_custom_tx_context(input)
        .encode()
        .expect("encode custom tx context")
}

async fn submit_verified_context_and_produce_block(
    handles: &TestNodeHandles,
    tx_context: TxContext,
) -> B256 {
    let tx_hash = {
        let tx_id = tx_context.tx_id();
        let mut pool = handles.mempool.write().await;
        pool.add(tx_context).expect("add tx to mempool");
        B256::from(tx_id)
    };

    assert_eq!(
        handles.mempool.read().await.len(),
        1,
        "mempool should have 1 tx"
    );

    let block_result = handles
        .dev
        .produce_block_from_mempool(10)
        .await
        .expect("produce block");

    assert_eq!(block_result.height, 1, "should be block 1");
    assert_eq!(block_result.tx_count, 1, "should have 1 tx");
    assert_eq!(
        block_result.successful_txs, 1,
        "transaction should have succeeded"
    );
    assert_eq!(
        block_result.failed_txs, 0,
        "no transactions should have failed"
    );
    assert!(
        handles.mempool.read().await.is_empty(),
        "mempool should be empty after block"
    );

    tx_hash
}

async fn submit_and_produce_block(handles: &TestNodeHandles, chain_id: u64, raw_tx: &[u8]) -> B256 {
    let tx_context = {
        let gateway = EthGateway::new(chain_id);
        gateway.decode_and_verify(raw_tx).expect("decode tx")
    };
    submit_verified_context_and_produce_block(handles, tx_context).await
}

async fn submit_with_gateway_and_produce_block(
    handles: &TestNodeHandles,
    gateway: &EthGateway,
    raw_tx: &[u8],
) -> B256 {
    let tx_context = gateway
        .decode_and_verify(raw_tx)
        .expect("decode and verify custom tx");
    submit_verified_context_and_produce_block(handles, tx_context).await
}

fn assert_post_block_state(
    handles: &TestNodeHandles,
    genesis_accounts: &EthGenesisAccounts,
    alice_account_id: AccountId,
    bob_account_id: AccountId,
    transfer_amount: u128,
    alice_balance_before: u128,
    bob_balance_before: u128,
) {
    let alice_nonce_after = read_nonce(handles.dev.storage(), alice_account_id);
    let alice_balance_after = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        alice_account_id,
    );
    let bob_balance_after = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        bob_account_id,
    );

    assert_eq!(
        alice_nonce_after, 1,
        "alice nonce should increment after tx"
    );
    assert_eq!(
        alice_balance_after,
        alice_balance_before - transfer_amount,
        "alice balance should decrease by transfer amount"
    );
    assert_eq!(
        bob_balance_after,
        bob_balance_before + transfer_amount,
        "bob balance should increase by transfer amount"
    );
}

struct Ed25519SenderTestSetup {
    handles: TestNodeHandles,
    genesis_accounts: EthGenesisAccounts,
    bob_account_id: AccountId,
    auth_signer: Ed25519SigningKey,
    alice_key: SigningKey,
    token_address: Address,
    sender_account_id: AccountId,
    sender_nonce_before: u64,
    sender_balance_before: u128,
    bob_balance_before: u128,
    transfer_amount: u128,
}

fn setup_ed25519_sender_test() -> Ed25519SenderTestSetup {
    let chain_id = 1338u64;
    let transfer_amount = 75u128;
    let sender_account_id = AccountId::from_u64(900_001);
    let sender_initial_balance = 500u128;

    let (alice_key, bob_key) = deterministic_signing_keys();
    let alice_address = get_address(&alice_key);
    let bob_address = get_address(&bob_key);

    let auth_signer = Ed25519SigningKey::from([0x42; 32]);
    let sender_public_key = auth_signer.verification_key().to_bytes();

    let (handles, genesis_accounts, _alice_account_id, bob_account_id) =
        setup_genesis_with_ed25519_sender(
            chain_id,
            alice_address,
            bob_address,
            sender_account_id,
            sender_public_key,
            sender_initial_balance,
        );

    let sender_nonce_before = read_nonce(handles.dev.storage(), sender_account_id);
    let sender_balance_before = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        sender_account_id,
    );
    let bob_balance_before = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        bob_account_id,
    );

    assert_eq!(
        sender_nonce_before, 0,
        "custom sender nonce should start at 0"
    );
    assert_eq!(
        sender_balance_before, sender_initial_balance,
        "custom sender should start funded"
    );

    let token_address = derive_runtime_contract_address(genesis_accounts.evolve);

    Ed25519SenderTestSetup {
        handles,
        genesis_accounts,
        bob_account_id,
        auth_signer,
        alice_key,
        token_address,
        sender_account_id,
        sender_nonce_before,
        sender_balance_before,
        bob_balance_before,
        transfer_amount,
    }
}

fn assert_custom_sender_post_state(setup: &Ed25519SenderTestSetup) {
    let sender_nonce_after = read_nonce(setup.handles.dev.storage(), setup.sender_account_id);
    let sender_balance_after = read_token_balance(
        setup.handles.dev.storage(),
        setup.genesis_accounts.evolve,
        setup.sender_account_id,
    );
    let bob_balance_after = read_token_balance(
        setup.handles.dev.storage(),
        setup.genesis_accounts.evolve,
        setup.bob_account_id,
    );

    assert_eq!(
        sender_nonce_after,
        setup.sender_nonce_before + 1,
        "ed25519 sender nonce should increment after auth"
    );
    assert_eq!(
        sender_balance_after,
        setup.sender_balance_before - setup.transfer_amount,
        "custom sender balance should decrease by transfer amount"
    );
    assert_eq!(
        bob_balance_after,
        setup.bob_balance_before + setup.transfer_amount,
        "recipient balance should increase by transfer amount"
    );
}

/// End-to-end test of token transfer via Ethereum transaction.
///
/// This test verifies:
/// 1. Alice signs a tx calling token.transfer(bob, 100)
/// 2. Transaction is submitted to mempool
/// 3. DevConsensus produces a block
/// 4. Transaction is authenticated (signature verified, nonce incremented)
/// 5. Token transfer executes (Alice balance decreases, Bob balance increases)
#[tokio::test]
async fn test_token_transfer_e2e() {
    let chain_id = 1337u64;
    let transfer_amount = 100u128;

    let (alice_key, bob_key) = deterministic_signing_keys();
    let alice_address = get_address(&alice_key);
    let bob_address = get_address(&bob_key);

    let (handles, genesis_accounts, alice_account_id, bob_account_id) =
        setup_genesis(chain_id, alice_address, bob_address);

    // Read initial state
    let alice_nonce_before = read_nonce(handles.dev.storage(), alice_account_id);
    let alice_balance_before = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        alice_account_id,
    );
    let bob_balance_before = read_token_balance(
        handles.dev.storage(),
        genesis_accounts.evolve,
        bob_account_id,
    );

    assert_eq!(alice_nonce_before, 0);
    assert_eq!(alice_balance_before, 1000);
    assert_eq!(bob_balance_before, 2000);

    let token_address = derive_runtime_contract_address(genesis_accounts.evolve);
    let raw_tx = build_transfer_tx(
        &alice_key,
        chain_id,
        0,
        token_address,
        bob_account_id,
        transfer_amount,
    );

    let _tx_hash = submit_and_produce_block(&handles, chain_id, &raw_tx).await;
    assert_post_block_state(
        &handles,
        &genesis_accounts,
        alice_account_id,
        bob_account_id,
        transfer_amount,
        alice_balance_before,
        bob_balance_before,
    );
}

#[tokio::test]
async fn test_custom_sender_ed25519_transfer_e2e() {
    let setup = setup_ed25519_sender_test();
    let mut gateway = EthGateway::new(1338);
    gateway.register_payload_verifier(sender_types::CUSTOM, Ed25519PayloadVerifier);

    let raw_tx = build_ed25519_custom_wire_tx(Ed25519CustomTxBuildInput {
        tx_template_signer: &setup.alice_key,
        auth_signer: &setup.auth_signer,
        chain_id: 1338,
        nonce: setup.sender_nonce_before,
        token_address: setup.token_address,
        recipient_account_id: setup.bob_account_id,
        transfer_amount: setup.transfer_amount,
        sender_account_id: setup.sender_account_id,
    });

    let _tx_hash = submit_with_gateway_and_produce_block(&setup.handles, &gateway, &raw_tx).await;
    assert_custom_sender_post_state(&setup);
}
