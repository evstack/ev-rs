use crate::{
    build_mempool_stf, default_gas_config, install_account_codes, GenesisAccounts, MINTER,
    PLACEHOLDER_ACCOUNT,
};
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Bytes, PrimitiveSignature, TxKind, U256};
use evolve_core::{
    encoding::Decodable, runtime_api::ACCOUNT_IDENTIFIER_PREFIX, AccountId, BlockContext,
    Environment, FungibleAsset, InvokableMessage, Message, ReadonlyKV, SdkResult,
};
use evolve_debugger::{ExecutionTrace, StateSnapshot, TraceBuilder};
use evolve_mempool::{new_shared_mempool, SharedMempool, TxContext};
use evolve_server::Block;
use evolve_simulator::{SimConfig, SimStorageAdapter, Simulator};
use evolve_stf::gas::GasCounter;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{Block as BlockTrait, StateChange, Transaction, WritableKV};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::TokenRef;
use evolve_tx::{account_id_to_address, address_to_account_id};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use tiny_keccak::{Hasher, Keccak};

pub struct SimTestApp {
    sim: Simulator,
    codes: AccountStorageMock,
    stf: crate::MempoolStf,
    accounts: GenesisAccounts,
    mempool: SharedMempool,
    chain_id: u64,
    signers: BTreeMap<AccountId, SigningKey>,
    nonces: BTreeMap<AccountId, u64>,
}

const SIM_CHAIN_ID: u64 = 1337;
const MAX_SIGNING_KEY_ATTEMPTS: usize = 16;

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut keccak = Keccak::v256();
    let mut output = [0u8; 32];
    keccak.update(data);
    keccak.finalize(&mut output);
    output
}

fn compute_selector(fn_name: &str) -> [u8; 4] {
    let hash = keccak256(fn_name.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn get_address(signing_key: &SigningKey) -> alloy_primitives::Address {
    let verifying_key = VerifyingKey::from(signing_key);
    let public_key = verifying_key.to_encoded_point(false);
    let public_key_bytes = &public_key.as_bytes()[1..];
    let hash = keccak256(public_key_bytes);
    alloy_primitives::Address::from_slice(&hash[12..])
}

fn sign_hash(signing_key: &SigningKey, hash: alloy_primitives::B256) -> PrimitiveSignature {
    let (sig, recovery_id): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) =
        signing_key.sign_prehash(hash.as_ref()).unwrap();
    let r = U256::from_be_slice(sig.r().to_bytes().as_slice());
    let s = U256::from_be_slice(sig.s().to_bytes().as_slice());
    let v = recovery_id.is_y_odd();
    PrimitiveSignature::new(r, s, v)
}

fn create_signed_tx(
    signing_key: &SigningKey,
    chain_id: u64,
    nonce: u64,
    to: alloy_primitives::Address,
    value: U256,
    input: Bytes,
    gas_limit: u64,
) -> Vec<u8> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 20_000_000_000,
        gas_limit,
        to: TxKind::Call(to),
        value,
        input,
        access_list: Default::default(),
    };

    let signature = sign_hash(signing_key, tx.signature_hash());
    let signed = tx.into_signed(signature);

    let mut encoded = vec![0x02];
    signed.rlp_encode(&mut encoded);
    encoded
}

pub trait TxGenerator {
    fn generate_tx(&mut self, height: u64, app: &mut SimTestApp) -> Option<Vec<u8>>;
}

pub struct TxGeneratorRegistry {
    generators: Vec<WeightedGenerator>,
}

struct WeightedGenerator {
    weight: u32,
    generator: Box<dyn TxGenerator>,
}

impl TxGeneratorRegistry {
    pub fn new() -> Self {
        Self {
            generators: Vec::new(),
        }
    }

    pub fn register<G: TxGenerator + 'static>(&mut self, weight: u32, generator: G) {
        if weight == 0 {
            return;
        }
        self.generators.push(WeightedGenerator {
            weight,
            generator: Box::new(generator),
        });
    }

    pub fn generate_block(
        &mut self,
        height: u64,
        app: &mut SimTestApp,
        max_txs: usize,
    ) -> Vec<Vec<u8>> {
        if self.generators.is_empty() || max_txs == 0 {
            return Vec::new();
        }

        let total_weight: u32 = self.generators.iter().map(|g| g.weight).sum();
        if total_weight == 0 {
            return Vec::new();
        }

        let mut txs = Vec::with_capacity(max_txs);
        for _ in 0..max_txs {
            let roll = app.sim.rng().gen_range(0..total_weight);
            let selected = {
                let mut acc = 0u32;
                let mut idx = None;
                for (i, entry) in self.generators.iter().enumerate() {
                    acc = acc.saturating_add(entry.weight);
                    if roll < acc {
                        idx = Some(i);
                        break;
                    }
                }
                idx
            };

            if let Some(idx) = selected {
                if let Some(tx) = self.generators[idx].generator.generate_tx(height, app) {
                    txs.push(tx);
                }
            }
        }

        txs
    }
}

impl Default for TxGeneratorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TokenTransferGenerator {
    token_account: AccountId,
    senders: Vec<AccountId>,
    recipients: Vec<AccountId>,
    min_amount: u128,
    max_amount: u128,
    gas_limit: u64,
}

impl TokenTransferGenerator {
    pub fn new(
        token_account: AccountId,
        senders: Vec<AccountId>,
        recipients: Vec<AccountId>,
        min_amount: u128,
        max_amount: u128,
        gas_limit: u64,
    ) -> Self {
        Self {
            token_account,
            senders,
            recipients,
            min_amount,
            max_amount,
            gas_limit,
        }
    }
}

impl TxGenerator for TokenTransferGenerator {
    fn generate_tx(&mut self, _height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if self.senders.is_empty() || self.recipients.is_empty() {
            return None;
        }
        if self.min_amount > self.max_amount {
            return None;
        }

        let sender_idx = app.sim.rng().gen_range(0..self.senders.len());
        let recipient_idx = app.sim.rng().gen_range(0..self.recipients.len());
        let amount = app.sim.rng().gen_range(self.min_amount..=self.max_amount);

        let sender = self.senders[sender_idx];
        let recipient = self.recipients[recipient_idx];

        app.build_token_transfer_tx(
            sender,
            self.token_account,
            recipient,
            amount,
            self.gas_limit,
        )
    }
}

/// Generates transfers with balance awareness to reduce failed txs.
/// Tracks expected balances and only generates transfers when funds are available.
pub struct BalanceAwareTransferGenerator {
    token_account: AccountId,
    accounts: Vec<AccountId>,
    balances: std::collections::BTreeMap<AccountId, u128>,
    min_amount: u128,
    max_amount: u128,
    gas_limit: u64,
}

impl BalanceAwareTransferGenerator {
    pub fn new(
        token_account: AccountId,
        initial_balances: Vec<(AccountId, u128)>,
        min_amount: u128,
        max_amount: u128,
        gas_limit: u64,
    ) -> Self {
        let accounts: Vec<AccountId> = initial_balances.iter().map(|(a, _)| *a).collect();
        let balances: std::collections::BTreeMap<_, _> = initial_balances.into_iter().collect();
        Self {
            token_account,
            accounts,
            balances,
            min_amount,
            max_amount,
            gas_limit,
        }
    }

    /// Updates internal balance tracking after a successful transfer.
    pub fn record_transfer(&mut self, from: AccountId, to: AccountId, amount: u128) {
        if let Some(bal) = self.balances.get_mut(&from) {
            *bal = bal.saturating_sub(amount);
        }
        *self.balances.entry(to).or_insert(0) += amount;
    }

    /// Adds a new account with initial balance (e.g., after minting).
    pub fn add_balance(&mut self, account: AccountId, amount: u128) {
        *self.balances.entry(account).or_insert(0) += amount;
        if !self.accounts.contains(&account) {
            self.accounts.push(account);
        }
    }
}

impl TxGenerator for BalanceAwareTransferGenerator {
    fn generate_tx(&mut self, _height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if self.accounts.len() < 2 {
            return None;
        }

        // Find accounts with sufficient balance
        let funded_accounts: Vec<_> = self
            .accounts
            .iter()
            .filter(|a| self.balances.get(a).copied().unwrap_or(0) >= self.min_amount)
            .copied()
            .collect();

        if funded_accounts.is_empty() {
            return None;
        }

        let sender_idx = app.sim.rng().gen_range(0..funded_accounts.len());
        let sender = funded_accounts[sender_idx];
        let sender_balance = self.balances.get(&sender).copied().unwrap_or(0);

        // Pick recipient different from sender
        let mut recipient = self.accounts[0];
        for _ in 0..self.accounts.len() {
            let idx = app.sim.rng().gen_range(0..self.accounts.len());
            if self.accounts[idx] != sender {
                recipient = self.accounts[idx];
                break;
            }
        }

        // Clamp amount to available balance
        let max_transfer = sender_balance.min(self.max_amount);
        if max_transfer < self.min_amount {
            return None;
        }
        let amount = app.sim.rng().gen_range(self.min_amount..=max_transfer);

        // Pre-update balances (optimistic)
        self.record_transfer(sender, recipient, amount);

        app.build_token_transfer_tx(
            sender,
            self.token_account,
            recipient,
            amount,
            self.gas_limit,
        )
    }
}

/// Generates random transfer amounts that may exceed balances (for failure testing).
pub struct FailingTransferGenerator {
    token_account: AccountId,
    senders: Vec<AccountId>,
    recipients: Vec<AccountId>,
    amount: u128,
    gas_limit: u64,
}

impl FailingTransferGenerator {
    pub fn new(
        token_account: AccountId,
        senders: Vec<AccountId>,
        recipients: Vec<AccountId>,
        amount: u128,
        gas_limit: u64,
    ) -> Self {
        Self {
            token_account,
            senders,
            recipients,
            amount,
            gas_limit,
        }
    }
}

impl TxGenerator for FailingTransferGenerator {
    fn generate_tx(&mut self, _height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if self.senders.is_empty() || self.recipients.is_empty() {
            return None;
        }

        let sender_idx = app.sim.rng().gen_range(0..self.senders.len());
        let recipient_idx = app.sim.rng().gen_range(0..self.recipients.len());
        let sender = self.senders[sender_idx];
        let recipient = self.recipients[recipient_idx];

        app.build_token_transfer_tx(
            sender,
            self.token_account,
            recipient,
            self.amount,
            self.gas_limit,
        )
    }
}

/// Generates a sequence of transfers in round-robin fashion for predictable testing.
pub struct RoundRobinTransferGenerator {
    token_account: AccountId,
    participants: Vec<AccountId>,
    current_sender_idx: usize,
    amount: u128,
    gas_limit: u64,
}

impl RoundRobinTransferGenerator {
    pub fn new(
        token_account: AccountId,
        participants: Vec<AccountId>,
        amount: u128,
        gas_limit: u64,
    ) -> Self {
        Self {
            token_account,
            participants,
            current_sender_idx: 0,
            amount,
            gas_limit,
        }
    }
}

impl TxGenerator for RoundRobinTransferGenerator {
    fn generate_tx(&mut self, _height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if self.participants.len() < 2 {
            return None;
        }

        let sender = self.participants[self.current_sender_idx];
        let recipient_idx = (self.current_sender_idx + 1) % self.participants.len();
        let recipient = self.participants[recipient_idx];

        self.current_sender_idx = (self.current_sender_idx + 1) % self.participants.len();

        app.build_token_transfer_tx(
            sender,
            self.token_account,
            recipient,
            self.amount,
            self.gas_limit,
        )
    }
}

impl SimTestApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: SimConfig, seed: u64) -> Self {
        let gas_config = default_gas_config();
        let bootstrap_stf = build_mempool_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
        let mut codes = AccountStorageMock::default();
        install_account_codes(&mut codes);

        let mut sim = Simulator::new(seed, config);

        let alice_key = Self::generate_signing_key(&mut sim);
        let bob_key = Self::generate_signing_key(&mut sim);
        let alice_address = get_address(&alice_key);
        let bob_address = get_address(&bob_key);

        let alice_id = address_to_account_id(alice_address);
        let bob_id = address_to_account_id(bob_address);

        Self::register_account_code_identifier(&mut sim, alice_id, "EthEoaAccount");
        Self::register_account_code_identifier(&mut sim, bob_id, "EthEoaAccount");
        Self::init_eth_eoa_storage(&mut sim, alice_id, alice_address.into());
        Self::init_eth_eoa_storage(&mut sim, bob_id, bob_address.into());

        let adapter = SimStorageAdapter::new(sim.storage());

        let genesis_block = BlockContext::new(0, 0);
        let (accounts, state) = bootstrap_stf
            .system_exec(&adapter, &codes, genesis_block, |env| {
                let atom = TokenRef::initialize(
                    evolve_fungible_asset::FungibleAssetMetadata {
                        name: "evolve".to_string(),
                        symbol: "ev".to_string(),
                        decimals: 6,
                        icon_url: "https://lol.wtf".to_string(),
                        description: "The evolve coin".to_string(),
                    },
                    vec![(alice_id, 1000), (bob_id, 2000)],
                    Some(MINTER),
                    env,
                )?
                .0;

                let scheduler = evolve_scheduler::scheduler_account::SchedulerRef::initialize(
                    vec![],
                    vec![],
                    env,
                )?
                .0;
                scheduler.update_begin_blockers(vec![], env)?;

                Ok(GenesisAccounts {
                    alice: alice_id,
                    bob: bob_id,
                    atom: atom.0,
                    scheduler: scheduler.0,
                })
            })
            .expect("genesis failed");

        sim.apply_state_changes(state.into_changes().expect("genesis changes"))
            .expect("apply genesis");

        let stf = build_mempool_stf(gas_config, accounts.scheduler);
        let mempool = new_shared_mempool(SIM_CHAIN_ID);

        let mut signers = BTreeMap::new();
        signers.insert(alice_id, alice_key);
        signers.insert(bob_id, bob_key);

        let mut nonces = BTreeMap::new();
        nonces.insert(alice_id, 0);
        nonces.insert(bob_id, 0);

        Self {
            sim,
            codes,
            stf,
            accounts,
            mempool,
            chain_id: SIM_CHAIN_ID,
            signers,
            nonces,
        }
    }

    fn generate_signing_key(sim: &mut Simulator) -> SigningKey {
        for _ in 0..MAX_SIGNING_KEY_ATTEMPTS {
            let bytes: [u8; 32] = sim.rng().gen();
            if let Ok(key) = SigningKey::from_bytes(&bytes.into()) {
                return key;
            }
        }
        panic!("failed to generate signing key");
    }

    fn register_account_code_identifier(sim: &mut Simulator, account_id: AccountId, code_id: &str) {
        let mut key = vec![ACCOUNT_IDENTIFIER_PREFIX];
        key.extend_from_slice(&account_id.as_bytes());
        let value = Message::new(&code_id.to_string())
            .expect("encode code id")
            .into_bytes()
            .expect("code id bytes");
        sim.storage_mut()
            .apply_changes(vec![StateChange::Set { key, value }])
            .expect("register account code");
    }

    fn init_eth_eoa_storage(sim: &mut Simulator, account_id: AccountId, eth_address: [u8; 20]) {
        let mut nonce_key = account_id.as_bytes();
        nonce_key.push(0u8);
        let nonce_value = Message::new(&0u64)
            .expect("encode nonce")
            .into_bytes()
            .expect("nonce bytes");

        let mut addr_key = account_id.as_bytes();
        addr_key.push(1u8);
        let addr_value = Message::new(&eth_address)
            .expect("encode eth address")
            .into_bytes()
            .expect("addr bytes");

        sim.storage_mut()
            .apply_changes(vec![
                StateChange::Set {
                    key: nonce_key,
                    value: nonce_value,
                },
                StateChange::Set {
                    key: addr_key,
                    value: addr_value,
                },
            ])
            .expect("init eoa storage");
    }

    fn next_nonce(&mut self, sender: AccountId) -> u64 {
        let entry = self.nonces.entry(sender).or_insert(0);
        let nonce = *entry;
        *entry = nonce.saturating_add(1);
        nonce
    }

    pub fn build_token_transfer_tx(
        &mut self,
        sender: AccountId,
        token_account: AccountId,
        recipient: AccountId,
        amount: u128,
        gas_limit: u64,
    ) -> Option<Vec<u8>> {
        let signing_key = self.signers.get(&sender)?.clone();
        let selector = compute_selector("transfer");
        let args = borsh::to_vec(&(recipient, amount)).ok()?;
        let mut calldata = Vec::with_capacity(4 + args.len());
        calldata.extend_from_slice(&selector);
        calldata.extend_from_slice(&args);

        let to = account_id_to_address(token_account);
        let nonce = self.next_nonce(sender);
        Some(create_signed_tx(
            &signing_key,
            self.chain_id,
            nonce,
            to,
            U256::ZERO,
            Bytes::from(calldata),
            gas_limit,
        ))
    }

    pub fn submit_raw_tx(
        &mut self,
        raw_tx: &[u8],
    ) -> Result<alloy_primitives::B256, evolve_mempool::MempoolError> {
        let mut pool = self.mempool.blocking_write();
        pool.add_raw(raw_tx)
    }

    pub fn produce_block_from_mempool(&mut self, max_txs: usize) -> BlockResult {
        let selected = {
            let mut pool = self.mempool.blocking_write();
            pool.select(max_txs)
        };

        let tx_hashes: Vec<_> = selected.iter().map(|tx| tx.hash()).collect();
        let transactions: Vec<TxContext> = selected.into_iter().map(|tx| (*tx).clone()).collect();

        let height = self.sim.time().block_height();
        let block = Block::for_testing(height, transactions);
        let result = self.apply_block(&block);

        if !tx_hashes.is_empty() {
            let mut pool = self.mempool.blocking_write();
            pool.remove_many(&tx_hashes);
        }

        self.sim.advance_block();
        result
    }

    pub fn submit_transfer_and_produce_block(
        &mut self,
        sender: AccountId,
        recipient: AccountId,
        amount: u128,
        gas_limit: u64,
    ) -> BlockResult {
        let raw_tx = self
            .build_token_transfer_tx(sender, self.accounts.atom, recipient, amount, gas_limit)
            .expect("build tx");
        self.submit_raw_tx(&raw_tx).expect("submit tx");
        self.produce_block_from_mempool(1)
    }

    pub fn system_exec_as<R>(
        &mut self,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let adapter = SimStorageAdapter::new(self.sim.storage());
        let block = BlockContext::new(self.sim.time().block_height(), 0);
        let (resp, state) =
            self.stf
                .system_exec_as(&adapter, &self.codes, block, impersonate, action)?;
        let changes = state.into_changes()?;
        self.sim.apply_state_changes(changes)?;
        Ok(resp)
    }

    /// Queries an account and decodes the response.
    pub fn query<Req: InvokableMessage, Resp: Decodable>(
        &self,
        target: AccountId,
        request: &Req,
    ) -> SdkResult<Resp> {
        let adapter = SimStorageAdapter::new(self.sim.storage());
        let gc = GasCounter::infinite();
        let response = self.stf.query(&adapter, &self.codes, target, request, gc)?;
        response.get::<Resp>()
    }

    pub fn apply_block(&mut self, block: &Block<TxContext>) -> BlockResult {
        let adapter = SimStorageAdapter::new(self.sim.storage());
        let (result, state) = self.stf.apply_block(&adapter, &self.codes, block);
        self.sim
            .apply_state_changes(state.into_changes().expect("state changes"))
            .expect("apply state changes");
        result
    }

    pub fn apply_block_with_trace(
        &mut self,
        block: &Block<TxContext>,
        builder: &mut TraceBuilder,
    ) -> BlockResult {
        let height = block.context().height;
        let timestamp_ms = self.sim.time().now_ms();
        builder.block_start(height, timestamp_ms);

        for tx in block.txs() {
            builder.tx_start(tx.compute_identifier(), tx.sender(), tx.recipient());
        }

        let adapter = SimStorageAdapter::new(self.sim.storage());
        let (result, state) = self.stf.apply_block(&adapter, &self.codes, block);
        let changes = state.into_changes().expect("state changes");

        for change in &changes {
            match change {
                evolve_stf_traits::StateChange::Set { key, value } => {
                    let old_value = self.sim.storage().get(key).expect("storage read");
                    builder.state_change(key.clone(), old_value, Some(value.clone()));
                }
                evolve_stf_traits::StateChange::Remove { key } => {
                    let old_value = self.sim.storage().get(key).expect("storage read");
                    builder.state_change(key.clone(), old_value, None);
                }
            }
        }

        self.sim
            .apply_state_changes(changes)
            .expect("apply state changes");

        for (tx, tx_result) in block.txs().iter().zip(result.tx_results.iter()) {
            builder.tx_end(
                tx.compute_identifier(),
                tx_result.response.is_ok(),
                tx_result.gas_used,
            );
        }

        builder.block_end(height, self.sim.storage().state_hash());
        result
    }

    pub fn next_block(&mut self) {
        self.sim.advance_block();
    }

    pub fn run_blocks_with<F>(&mut self, num_blocks: u64, mut make_txs: F) -> Vec<BlockResult>
    where
        F: FnMut(u64, &mut SimTestApp) -> Vec<Vec<u8>>,
    {
        let mut results = Vec::with_capacity(num_blocks as usize);
        let max_txs = self.sim.config().max_txs_per_block;

        for _ in 0..num_blocks {
            let height = self.sim.time().block_height();
            let mut txs = make_txs(height, self);
            if txs.len() > max_txs {
                txs.truncate(max_txs);
            }

            for raw_tx in &txs {
                self.submit_raw_tx(raw_tx).expect("submit tx");
            }

            let result = self.produce_block_from_mempool(max_txs);
            results.push(result);
        }

        results
    }

    pub fn run_blocks_with_registry(
        &mut self,
        num_blocks: u64,
        registry: &mut TxGeneratorRegistry,
    ) -> Vec<BlockResult> {
        let mut results = Vec::with_capacity(num_blocks as usize);
        let max_txs = self.sim.config().max_txs_per_block;

        for _ in 0..num_blocks {
            let height = self.sim.time().block_height();
            let txs = registry.generate_block(height, self, max_txs);

            for raw_tx in &txs {
                self.submit_raw_tx(raw_tx).expect("submit tx");
            }

            let result = self.produce_block_from_mempool(max_txs);
            results.push(result);
        }

        results
    }

    pub fn run_blocks_with_trace<F>(
        &mut self,
        num_blocks: u64,
        mut make_txs: F,
    ) -> (Vec<BlockResult>, ExecutionTrace)
    where
        F: FnMut(u64, &mut SimTestApp) -> Vec<Vec<u8>>,
    {
        let snapshot = StateSnapshot::from_data(
            self.sim.storage().snapshot().data,
            self.sim.time().block_height(),
            self.sim.time().now_ms(),
        );
        let mut builder = TraceBuilder::new(self.sim.seed_info().seed, snapshot);
        let mut results = Vec::with_capacity(num_blocks as usize);
        let max_txs = self.sim.config().max_txs_per_block;

        for _ in 0..num_blocks {
            let height = self.sim.time().block_height();
            let mut txs = make_txs(height, self);
            if txs.len() > max_txs {
                txs.truncate(max_txs);
            }

            for raw_tx in &txs {
                self.submit_raw_tx(raw_tx).expect("submit tx");
            }

            let selected = {
                let mut pool = self.mempool.blocking_write();
                pool.select(max_txs)
            };

            let tx_hashes: Vec<_> = selected.iter().map(|tx| tx.hash()).collect();
            let transactions: Vec<TxContext> =
                selected.into_iter().map(|tx| (*tx).clone()).collect();

            let block = Block::for_testing(height, transactions);
            let result = self.apply_block_with_trace(&block, &mut builder);
            results.push(result);

            if !tx_hashes.is_empty() {
                let mut pool = self.mempool.blocking_write();
                pool.remove_many(&tx_hashes);
            }

            self.sim.advance_block();
        }

        (results, builder.finish())
    }

    pub fn mint_atom(&mut self, recipient: AccountId, amount: u128) -> FungibleAsset {
        let atom_id = self.accounts.atom;
        self.system_exec_as(MINTER, |env| {
            let atom_token = TokenRef::from(atom_id);
            atom_token.mint(recipient, amount, env)?;
            Ok(FungibleAsset {
                asset_id: atom_token.0,
                amount,
            })
        })
        .unwrap()
    }

    pub fn go_to_height(&mut self, height: u64) {
        assert!(self.sim.time().block_height() < height);
        self.sim.set_block_height(height);
    }

    /// Create an EOA with a randomly generated Ethereum address.
    pub fn create_eoa(&mut self) -> AccountId {
        let signing_key = Self::generate_signing_key(&mut self.sim);
        let address = get_address(&signing_key);
        let account_id = self.create_eoa_with_address(address.into());
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    /// Create an EOA with a specific Ethereum address.
    pub fn create_eoa_with_address(&mut self, eth_address: [u8; 20]) -> AccountId {
        let account_id = address_to_account_id(alloy_primitives::Address::from(eth_address));
        Self::register_account_code_identifier(&mut self.sim, account_id, "EthEoaAccount");
        Self::init_eth_eoa_storage(&mut self.sim, account_id, eth_address);
        account_id
    }

    /// Create a signer without creating an EOA account in state.
    pub fn create_signer_without_account(&mut self) -> AccountId {
        let signing_key = Self::generate_signing_key(&mut self.sim);
        let address = get_address(&signing_key);
        let account_id = address_to_account_id(address);
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    /// Create a signer and register a non-EOA account code for it.
    pub fn create_signer_with_code(&mut self, code_id: &str) -> AccountId {
        let signing_key = Self::generate_signing_key(&mut self.sim);
        let address = get_address(&signing_key);
        let account_id = address_to_account_id(address);
        Self::register_account_code_identifier(&mut self.sim, account_id, code_id);
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    pub fn simulator(&self) -> &Simulator {
        &self.sim
    }

    pub fn simulator_mut(&mut self) -> &mut Simulator {
        &mut self.sim
    }

    pub fn accounts(&self) -> GenesisAccounts {
        self.accounts
    }
}

impl Default for SimTestApp {
    fn default() -> Self {
        Self::with_config(SimConfig::default(), 0)
    }
}
