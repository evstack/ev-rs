#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::{
    build_mempool_stf, default_gas_config, install_account_codes, GenesisAccounts, MINTER,
    PLACEHOLDER_ACCOUNT,
};
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Bytes, PrimitiveSignature, TxKind, U256};
use evolve_core::{
    encoding::Decodable, AccountId, Environment, FungibleAsset, InvokableMessage, ReadonlyKV,
    SdkResult,
};
use evolve_debugger::{ExecutionTrace, StateSnapshot, TraceBuilder};
use evolve_mempool::{new_shared_mempool, Mempool, MempoolError, MempoolTx, SharedMempool};
use evolve_server::Block;
use evolve_simulator::{
    apply_block_and_commit, current_block_context, generate_signing_key, init_eth_eoa_storage,
    query_stf, register_account_code_identifier, run_block_iterations, system_exec_as_and_commit,
    system_exec_genesis_and_commit, SimConfig, Simulator,
};
use evolve_stf::gas::GasCounter;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{Block as BlockTrait, Transaction};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::TokenRef;
use evolve_tx_eth::{
    derive_eth_eoa_account_id, derive_runtime_contract_address, register_runtime_contract_account,
};
use evolve_tx_eth::{EthGateway, TxContext};
use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey, VerifyingKey};
use std::collections::BTreeMap;
use tiny_keccak::{Hasher, Keccak};

pub struct SimTestApp {
    sim: Simulator,
    codes: AccountStorageMock,
    stf: crate::MempoolStf,
    accounts: GenesisAccounts,
    mempool: SharedMempool<Mempool<TxContext>>,
    gateway: EthGateway,
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
    let (sig, recovery_id): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
        .sign_prehash(hash.as_ref())
        .unwrap_or_else(|e| panic!("failed to sign prehash: {e}"));
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

/// Generates transfers while periodically creating and funding new EOA accounts.
/// New accounts are then included in future transfer traffic.
pub struct DynamicAccountTransferGenerator {
    token_account: AccountId,
    faucet: AccountId,
    participants: Vec<AccountId>,
    balances: BTreeMap<AccountId, u128>,
    min_amount: u128,
    max_amount: u128,
    funding_amount: u128,
    gas_limit: u64,
    create_every_n_blocks: u64,
    max_new_accounts: usize,
    created_accounts: usize,
}

impl DynamicAccountTransferGenerator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token_account: AccountId,
        faucet: AccountId,
        initial_balances: Vec<(AccountId, u128)>,
        min_amount: u128,
        max_amount: u128,
        funding_amount: u128,
        gas_limit: u64,
        create_every_n_blocks: u64,
        max_new_accounts: usize,
    ) -> Self {
        let participants = initial_balances
            .iter()
            .map(|(account, _)| *account)
            .collect();
        let balances = initial_balances.into_iter().collect();
        Self {
            token_account,
            faucet,
            participants,
            balances,
            min_amount,
            max_amount,
            funding_amount,
            gas_limit,
            create_every_n_blocks: create_every_n_blocks.max(1),
            max_new_accounts,
            created_accounts: 0,
        }
    }

    fn record_transfer(&mut self, from: AccountId, to: AccountId, amount: u128) {
        if let Some(balance) = self.balances.get_mut(&from) {
            *balance = balance.saturating_sub(amount);
        }
        *self.balances.entry(to).or_insert(0) += amount;
    }

    fn maybe_create_and_fund(&mut self, height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if self.created_accounts >= self.max_new_accounts {
            return None;
        }
        if height % self.create_every_n_blocks != 0 {
            return None;
        }

        let faucet_balance = self.balances.get(&self.faucet).copied().unwrap_or(0);
        if faucet_balance < self.funding_amount {
            return None;
        }

        let new_account = app.create_eoa();
        self.participants.push(new_account);
        self.balances.insert(new_account, 0);
        self.created_accounts = self.created_accounts.saturating_add(1);
        self.record_transfer(self.faucet, new_account, self.funding_amount);
        app.build_token_transfer_tx(
            self.faucet,
            self.token_account,
            new_account,
            self.funding_amount,
            self.gas_limit,
        )
    }
}

impl TxGenerator for DynamicAccountTransferGenerator {
    fn generate_tx(&mut self, height: u64, app: &mut SimTestApp) -> Option<Vec<u8>> {
        if let Some(tx) = self.maybe_create_and_fund(height, app) {
            return Some(tx);
        }

        if self.participants.len() < 2 || self.min_amount > self.max_amount {
            return None;
        }

        let funded: Vec<_> = self
            .participants
            .iter()
            .filter(|account| self.balances.get(account).copied().unwrap_or(0) >= self.min_amount)
            .copied()
            .collect();
        if funded.is_empty() {
            return None;
        }

        let sender = funded[app.sim.rng().gen_range(0..funded.len())];
        let sender_balance = self.balances.get(&sender).copied().unwrap_or(0);
        let max_transfer = sender_balance.min(self.max_amount);
        if max_transfer < self.min_amount {
            return None;
        }

        let mut recipient = self.participants[0];
        for _ in 0..self.participants.len() {
            let idx = app.sim.rng().gen_range(0..self.participants.len());
            if self.participants[idx] != sender {
                recipient = self.participants[idx];
                break;
            }
        }

        let amount = app.sim.rng().gen_range(self.min_amount..=max_transfer);
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

impl SimTestApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: SimConfig, seed: u64) -> Self {
        let gas_config = default_gas_config();
        let bootstrap_stf = build_mempool_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
        let mut codes = AccountStorageMock::default();
        install_account_codes(&mut codes);

        let mut sim = Simulator::new(seed, config.with_storage_backend_from_env());

        let alice_key = generate_signing_key(&mut sim, MAX_SIGNING_KEY_ATTEMPTS)
            .expect("failed to generate signing key");
        let bob_key = generate_signing_key(&mut sim, MAX_SIGNING_KEY_ATTEMPTS)
            .expect("failed to generate signing key");
        let alice_address = get_address(&alice_key);
        let bob_address = get_address(&bob_key);

        let alice_id = derive_eth_eoa_account_id(alice_address);
        let bob_id = derive_eth_eoa_account_id(bob_address);

        register_account_code_identifier(&mut sim, alice_id, "EthEoaAccount")
            .expect("register alice code");
        register_account_code_identifier(&mut sim, bob_id, "EthEoaAccount")
            .expect("register bob code");
        init_eth_eoa_storage(&mut sim, alice_id, alice_address.into()).expect("init alice eoa");
        init_eth_eoa_storage(&mut sim, bob_id, bob_address.into()).expect("init bob eoa");

        let run_genesis = |env: &mut dyn Environment| -> SdkResult<GenesisAccounts> {
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
            let _atom_eth_addr = register_runtime_contract_account(atom.0, env)?;

            let scheduler =
                evolve_scheduler::scheduler_account::SchedulerRef::initialize(vec![], vec![], env)?
                    .0;
            let _scheduler_eth_addr = register_runtime_contract_account(scheduler.0, env)?;
            scheduler.update_begin_blockers(vec![], env)?;

            Ok(GenesisAccounts {
                alice: alice_id,
                bob: bob_id,
                atom: atom.0,
                scheduler: scheduler.0,
            })
        };

        let accounts =
            system_exec_genesis_and_commit(&mut sim, &bootstrap_stf, &codes, run_genesis)
                .expect("genesis failed");

        let stf = build_mempool_stf(gas_config, accounts.scheduler);
        Self {
            sim,
            codes,
            stf,
            accounts,
            mempool: new_shared_mempool(),
            gateway: EthGateway::new(SIM_CHAIN_ID),
            chain_id: SIM_CHAIN_ID,
            signers: BTreeMap::from([(alice_id, alice_key), (bob_id, bob_key)]),
            nonces: BTreeMap::from([(alice_id, 0), (bob_id, 0)]),
        }
    }

    fn next_nonce(&mut self, sender: AccountId) -> u64 {
        let entry = self.nonces.entry(sender).or_insert(0);
        let nonce = *entry;
        *entry = nonce.saturating_add(1);
        nonce
    }

    fn get_storage_value(&self, key: &[u8]) -> SdkResult<Option<Vec<u8>>> {
        self.sim.readonly_kv().get(key)
    }

    pub fn build_token_transfer_tx(
        &mut self,
        sender: AccountId,
        token_account: AccountId,
        recipient: AccountId,
        amount: u128,
        gas_limit: u64,
    ) -> Option<Vec<u8>> {
        let nonce = self.next_nonce(sender);
        let signing_key = self.signers.get(&sender)?;
        let selector = compute_selector("transfer");
        let args = borsh::to_vec(&(recipient, amount)).ok()?;
        let mut calldata = Vec::with_capacity(4 + args.len());
        calldata.extend_from_slice(&selector);
        calldata.extend_from_slice(&args);

        let to = derive_runtime_contract_address(token_account);
        Some(create_signed_tx(
            signing_key,
            self.chain_id,
            nonce,
            to,
            U256::ZERO,
            Bytes::from(calldata),
            gas_limit,
        ))
    }

    pub fn build_token_transfer_tx_context(
        &mut self,
        sender: AccountId,
        token_account: AccountId,
        recipient: AccountId,
        amount: u128,
        gas_limit: u64,
    ) -> Option<TxContext> {
        let raw =
            self.build_token_transfer_tx(sender, token_account, recipient, amount, gas_limit)?;
        self.gateway.decode_and_verify(&raw).ok()
    }

    pub fn submit_raw_tx(&mut self, raw_tx: &[u8]) -> Result<alloy_primitives::B256, MempoolError> {
        let tx_context = self
            .gateway
            .decode_and_verify(raw_tx)
            .map_err(|e| MempoolError::DecodeFailed(e.to_string()))?;
        self.submit_tx_context(tx_context)
    }

    pub fn submit_tx_context(
        &mut self,
        tx_context: TxContext,
    ) -> Result<alloy_primitives::B256, MempoolError> {
        let mut pool = self.mempool.blocking_write();
        let tx_id = pool.add(tx_context)?;
        Ok(alloy_primitives::B256::from(tx_id))
    }

    fn take_mempool_batch(&mut self, max_txs: usize) -> (Vec<[u8; 32]>, Vec<TxContext>) {
        let selected = {
            let mut pool = self.mempool.blocking_write();
            pool.select(max_txs)
        };

        let mut tx_hashes = Vec::with_capacity(selected.len());
        let mut transactions = Vec::with_capacity(selected.len());
        for tx in selected {
            tx_hashes.push(tx.tx_id());
            transactions.push((*tx).clone());
        }
        (tx_hashes, transactions)
    }

    fn remove_many_from_mempool(&mut self, tx_hashes: &[[u8; 32]]) {
        if tx_hashes.is_empty() {
            return;
        }
        let mut pool = self.mempool.blocking_write();
        pool.remove_many(tx_hashes);
    }

    fn produce_block_internal(
        &mut self,
        height: u64,
        transactions: Vec<TxContext>,
        advance: bool,
    ) -> BlockResult {
        let block = Block::for_testing(height, transactions);
        let result = self.apply_block(&block);
        if advance {
            self.sim.advance_block();
        }
        result
    }

    pub fn produce_block_from_mempool(&mut self, max_txs: usize) -> BlockResult {
        let (tx_hashes, transactions) = self.take_mempool_batch(max_txs);
        let height = self.sim.time().block_height();
        let result = self.produce_block_internal(height, transactions, true);
        self.remove_many_from_mempool(&tx_hashes);
        result
    }

    pub fn produce_block_with_txs(&mut self, transactions: Vec<TxContext>) -> BlockResult {
        let height = self.sim.time().block_height();
        self.produce_block_internal(height, transactions, true)
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
        let block = current_block_context(&self.sim);
        system_exec_as_and_commit(
            &mut self.sim,
            &self.stf,
            &self.codes,
            block,
            impersonate,
            action,
        )
    }

    /// Queries an account and decodes the response.
    pub fn query<Req: InvokableMessage, Resp: Decodable>(
        &self,
        target: AccountId,
        request: &Req,
    ) -> SdkResult<Resp> {
        let gc = GasCounter::infinite();
        let response = query_stf(&self.sim, &self.stf, &self.codes, target, request, gc)?;
        response.get::<Resp>()
    }

    pub fn apply_block(&mut self, block: &Block<TxContext>) -> BlockResult {
        apply_block_and_commit(&mut self.sim, &self.stf, &self.codes, block)
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

        let storage = self.sim.readonly_kv();
        let (result, state) = self.stf.apply_block(&storage, &self.codes, block);
        let changes = state.into_changes().expect("state changes");

        for change in &changes {
            match change {
                evolve_stf_traits::StateChange::Set { key, value } => {
                    let old_value = self
                        .get_storage_value(key.as_slice())
                        .expect("storage read");
                    builder.state_change(key.clone(), old_value, Some(value.clone()));
                }
                evolve_stf_traits::StateChange::Remove { key } => {
                    let old_value = self
                        .get_storage_value(key.as_slice())
                        .expect("storage read");
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

        let state_hash = self.sim.app_hash().unwrap_or([0u8; 32]);
        builder.block_end(height, state_hash);
        result
    }

    pub fn next_block(&mut self) {
        self.sim.advance_block();
    }

    pub fn run_blocks_with<F>(&mut self, num_blocks: u64, mut make_txs: F) -> Vec<BlockResult>
    where
        F: FnMut(u64, &mut SimTestApp) -> Vec<Vec<u8>>,
    {
        let max_txs = self.sim.config().max_txs_per_block;
        run_block_iterations(
            self,
            num_blocks,
            |app| app.sim.time().block_height(),
            |height, app| {
                let mut txs = make_txs(height, app);
                if txs.len() > max_txs {
                    txs.truncate(max_txs);
                }
                for raw_tx in &txs {
                    app.submit_raw_tx(raw_tx).expect("submit tx");
                }
                let (tx_hashes, transactions) = app.take_mempool_batch(max_txs);
                let result = app.produce_block_internal(height, transactions, false);
                app.remove_many_from_mempool(&tx_hashes);
                result
            },
            |app| app.sim.advance_block(),
        )
    }

    pub fn run_blocks_with_registry(
        &mut self,
        num_blocks: u64,
        registry: &mut TxGeneratorRegistry,
    ) -> Vec<BlockResult> {
        let max_txs = self.sim.config().max_txs_per_block;
        run_block_iterations(
            self,
            num_blocks,
            |app| app.sim.time().block_height(),
            |height, app| {
                let txs = registry.generate_block(height, app, max_txs);
                for raw_tx in &txs {
                    app.submit_raw_tx(raw_tx).expect("submit tx");
                }
                let (tx_hashes, transactions) = app.take_mempool_batch(max_txs);
                let result = app.produce_block_internal(height, transactions, false);
                app.remove_many_from_mempool(&tx_hashes);
                result
            },
            |app| app.sim.advance_block(),
        )
    }

    pub fn run_blocks_with_trace<F>(
        &mut self,
        num_blocks: u64,
        mut make_txs: F,
    ) -> (Vec<BlockResult>, ExecutionTrace)
    where
        F: FnMut(u64, &mut SimTestApp) -> Vec<Vec<u8>>,
    {
        let snapshot = StateSnapshot::empty();
        let mut builder = TraceBuilder::new(self.sim.seed_info().seed, snapshot);
        let max_txs = self.sim.config().max_txs_per_block;
        let results = run_block_iterations(
            self,
            num_blocks,
            |app| app.sim.time().block_height(),
            |height, app| {
                let mut txs = make_txs(height, app);
                if txs.len() > max_txs {
                    txs.truncate(max_txs);
                }
                for raw_tx in &txs {
                    app.submit_raw_tx(raw_tx).expect("submit tx");
                }

                let (tx_hashes, transactions) = app.take_mempool_batch(max_txs);
                let block = Block::for_testing(height, transactions);
                let result = app.apply_block_with_trace(&block, &mut builder);

                app.remove_many_from_mempool(&tx_hashes);
                result
            },
            |app| app.sim.advance_block(),
        );

        (results, builder.finish())
    }

    pub fn mint_atom(&mut self, recipient: AccountId, amount: u128) -> FungibleAsset {
        let atom_id = self.accounts.atom;
        match self.system_exec_as(MINTER, |env| {
            let atom_token = TokenRef::from(atom_id);
            atom_token.mint(recipient, amount, env)?;
            Ok(FungibleAsset {
                asset_id: atom_token.0,
                amount,
            })
        }) {
            Ok(asset) => asset,
            Err(e) => panic!("mint_atom failed: {e:?}"),
        }
    }

    pub fn go_to_height(&mut self, height: u64) {
        assert!(self.sim.time().block_height() < height);
        self.sim.set_block_height(height);
    }

    /// Create an EOA with a randomly generated Ethereum address.
    pub fn create_eoa(&mut self) -> AccountId {
        let signing_key = generate_signing_key(&mut self.sim, MAX_SIGNING_KEY_ATTEMPTS)
            .expect("failed to generate signing key");
        let address = get_address(&signing_key);
        let account_id = self.create_eoa_with_address(address.into());
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    /// Create an EOA with a specific Ethereum address.
    pub fn create_eoa_with_address(&mut self, eth_address: [u8; 20]) -> AccountId {
        let account_id = derive_eth_eoa_account_id(alloy_primitives::Address::from(eth_address));
        register_account_code_identifier(&mut self.sim, account_id, "EthEoaAccount")
            .expect("register eoa code");
        init_eth_eoa_storage(&mut self.sim, account_id, eth_address).expect("init eoa storage");
        account_id
    }

    /// Create a signer without creating an EOA account in state.
    pub fn create_signer_without_account(&mut self) -> AccountId {
        let signing_key = generate_signing_key(&mut self.sim, MAX_SIGNING_KEY_ATTEMPTS)
            .expect("failed to generate signing key");
        let address = get_address(&signing_key);
        let account_id = derive_eth_eoa_account_id(address);
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    /// Create a signer and register a non-EOA account code for it.
    pub fn create_signer_with_code(&mut self, code_id: &str) -> AccountId {
        let signing_key = generate_signing_key(&mut self.sim, MAX_SIGNING_KEY_ATTEMPTS)
            .expect("failed to generate signing key");
        let address = get_address(&signing_key);
        let account_id = derive_eth_eoa_account_id(address);
        register_account_code_identifier(&mut self.sim, account_id, code_id)
            .expect("register account code");
        self.signers.insert(account_id, signing_key);
        self.nonces.entry(account_id).or_insert(0);
        account_id
    }

    pub fn accounts(&self) -> GenesisAccounts {
        self.accounts
    }

    pub fn signer_account_count(&self) -> usize {
        self.signers.len()
    }
}

impl Default for SimTestApp {
    fn default() -> Self {
        Self::with_config(SimConfig::default(), 0)
    }
}
