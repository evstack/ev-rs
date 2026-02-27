//! E2E test for mempool and dev consensus integration.
//!
//! This test proves the complete token transfer flow:
//! 1. Sign an Ethereum transaction targeting the token account
//! 2. Submit to mempool
//! 3. DevConsensus produces a block
//! 4. Transaction is authenticated and executed
//! 5. Token balances are updated correctly

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Bytes, PrimitiveSignature, TxKind, U256};
use async_trait::async_trait;
use evolve_core::{AccountId, ErrorCode, ReadonlyKV};
use evolve_node::build_dev_node_with_mempool;
use evolve_server::DevConfig;
use evolve_storage::{CommitHash, Operation};
use evolve_testapp::{
    build_mempool_stf, default_gas_config, do_eth_genesis, install_account_codes,
};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_tx_eth::{derive_runtime_contract_address, EthGateway};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use std::collections::BTreeMap;
use std::sync::RwLock;
use tiny_keccak::{Hasher, Keccak};

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
        use evolve_core::Message;

        // Storage keys are: account_id + prefix (u8)
        // Item::new(0) = nonce, Item::new(1) = eth_address

        // Set nonce = 0
        let mut nonce_key = account_id.as_bytes().to_vec();
        nonce_key.push(0u8); // Item prefix for nonce
        let nonce_value = Message::new(&0u64).unwrap().into_bytes().unwrap();
        self.data.write().unwrap().insert(nonce_key, nonce_value);

        // Set eth_address
        let mut addr_key = account_id.as_bytes().to_vec();
        addr_key.push(1u8); // Item prefix for eth_address
        let addr_value = Message::new(&eth_address).unwrap().into_bytes().unwrap();
        self.data.write().unwrap().insert(addr_key, addr_value);
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

/// Compute keccak256 hash.
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut keccak = Keccak::v256();
    let mut output = [0u8; 32];
    keccak.update(data);
    keccak.finalize(&mut output);
    output
}

/// Compute function selector from function name (first 4 bytes of keccak256).
fn compute_selector(fn_name: &str) -> [u8; 4] {
    let hash = keccak256(fn_name.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

/// Sign a transaction hash and create an alloy signature.
fn sign_hash(signing_key: &SigningKey, hash: alloy_primitives::B256) -> PrimitiveSignature {
    let (sig, recovery_id) = signing_key.sign_prehash(hash.as_ref()).unwrap();
    let r = U256::from_be_slice(&sig.r().to_bytes());
    let s = U256::from_be_slice(&sig.s().to_bytes());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

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

    // Create signing keys for Alice and Bob
    let alice_key = SigningKey::random(&mut OsRng);
    let bob_key = SigningKey::random(&mut OsRng);
    let alice_address = get_address(&alice_key);
    let bob_address = get_address(&bob_key);

    // Set up account codes
    let mut codes = AccountStorageMock::new();
    install_account_codes(&mut codes);

    // Create initial storage
    let init_storage = AsyncMockStorage::new();

    // Build STF for TxContext
    let gas_config = default_gas_config();
    let stf = build_mempool_stf(gas_config.clone(), AccountId::new(0));

    // Run genesis to create token and scheduler
    let (genesis_state, genesis_accounts) = do_eth_genesis(
        &stf,
        &codes,
        &init_storage,
        alice_address.into(),
        bob_address.into(),
    )
    .expect("genesis should succeed");

    // Apply genesis state changes to existing storage
    let genesis_changes = genesis_state.into_changes().expect("get changes");
    init_storage.apply_changes(genesis_changes);
    let storage = init_storage;

    // Rebuild STF with correct scheduler ID
    let stf = build_mempool_stf(gas_config, genesis_accounts.scheduler);

    let config = DevConfig {
        block_interval: None, // Manual block production
        gas_limit: 30_000_000,
        initial_height: 1,
        chain_id,
    };
    let handles = build_dev_node_with_mempool(stf, storage, codes, config);
    let dev = handles.dev;
    let mempool = handles.mempool;

    let alice_account_id =
        evolve_tx_eth::lookup_account_id_in_storage(dev.storage(), alice_address)
            .expect("lookup alice id")
            .expect("alice id exists");
    let bob_account_id = evolve_tx_eth::lookup_account_id_in_storage(dev.storage(), bob_address)
        .expect("lookup bob id")
        .expect("bob id exists");

    // Read initial state
    let alice_nonce_before = read_nonce(dev.storage(), alice_account_id);
    let alice_balance_before =
        read_token_balance(dev.storage(), genesis_accounts.evolve, alice_account_id);
    let bob_balance_before =
        read_token_balance(dev.storage(), genesis_accounts.evolve, bob_account_id);

    println!("Initial state:");
    println!("  Alice nonce: {}", alice_nonce_before);
    println!("  Alice token balance: {}", alice_balance_before);
    println!("  Bob token balance: {}", bob_balance_before);

    assert_eq!(alice_nonce_before, 0);
    assert_eq!(alice_balance_before, 1000); // From genesis
    assert_eq!(bob_balance_before, 2000); // From genesis

    // Build the transfer call
    // Function: transfer(to: AccountId, amount: u128)
    // Selector: keccak256("transfer")[0..4]
    let transfer_amount = 100u128;
    let selector = compute_selector("transfer");

    // Borsh-encode the arguments: (AccountId, u128)
    // AccountId is u128 (16 bytes), amount is u128 (16 bytes)
    let args = borsh::to_vec(&(bob_account_id, transfer_amount)).expect("encode args");

    // Calldata = selector + args
    let mut calldata = Vec::with_capacity(4 + args.len());
    calldata.extend_from_slice(&selector);
    calldata.extend_from_slice(&args);

    // Get token's Ethereum address
    let token_address = derive_runtime_contract_address(genesis_accounts.evolve);

    println!("\nTransaction details:");
    println!("  Token account: {:?}", genesis_accounts.evolve);
    println!("  Token address: {:?}", token_address);
    println!("  Selector: 0x{}", hex::encode(selector));
    println!("  Transfer amount: {}", transfer_amount);

    // Create and sign transaction from Alice to Token
    let raw_tx = create_signed_tx(
        &alice_key,
        chain_id,
        0, // nonce
        token_address,
        U256::ZERO,
        Bytes::from(calldata),
    );

    // Submit transaction to mempool
    let tx_hash = {
        let gateway = EthGateway::new(chain_id);
        let tx_context = gateway.decode_and_verify(&raw_tx).expect("decode tx");
        let mut pool = mempool.write().await;
        let tx_id = pool.add(tx_context).expect("add tx to mempool");
        alloy_primitives::B256::from(tx_id)
    };
    println!("\nSubmitted transaction: {:?}", tx_hash);

    // Verify transaction is in mempool
    assert_eq!(mempool.read().await.len(), 1, "mempool should have 1 tx");

    // Produce a block from mempool
    let block_result = dev
        .produce_block_from_mempool(10)
        .await
        .expect("produce block");

    println!("\nBlock produced at height {}", block_result.height);
    println!("  Transactions: {}", block_result.tx_count);
    println!("  Successful: {}", block_result.successful_txs);
    println!("  Failed: {}", block_result.failed_txs);

    // Verify block was produced
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

    // Mempool should be empty after block production
    assert!(
        mempool.read().await.is_empty(),
        "mempool should be empty after block"
    );

    // Verify state changes
    let alice_nonce_after = read_nonce(dev.storage(), alice_account_id);
    let alice_balance_after =
        read_token_balance(dev.storage(), genesis_accounts.evolve, alice_account_id);
    let bob_balance_after =
        read_token_balance(dev.storage(), genesis_accounts.evolve, bob_account_id);

    println!("\nFinal state:");
    println!("  Alice nonce: {}", alice_nonce_after);
    println!("  Alice token balance: {}", alice_balance_after);
    println!("  Bob token balance: {}", bob_balance_after);

    // Verify nonce was incremented
    assert_eq!(
        alice_nonce_after, 1,
        "alice nonce should increment after tx"
    );

    // Verify token balances changed correctly
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

    println!("\n✓ Token transfer e2e test passed!");
    println!("  - Transaction submitted to mempool");
    println!("  - DevConsensus produced block from mempool");
    println!("  - Authentication succeeded (nonce incremented)");
    println!(
        "  - Token transfer executed ({} tokens from Alice to Bob)",
        transfer_amount
    );
}
