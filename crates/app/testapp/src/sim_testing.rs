use crate::eth_eoa::eth_eoa_account::EthEoaAccountRef;
use crate::{
    build_stf, default_gas_config, do_genesis, install_account_codes, CustomStf, GenesisAccounts,
    TestTx, MINTER, PLACEHOLDER_ACCOUNT,
};
use evolve_core::{
    encoding::Decodable, AccountId, BlockContext, Environment, FungibleAsset, InvokableMessage,
    InvokeRequest, ReadonlyKV, SdkResult,
};
use evolve_debugger::{ExecutionTrace, StateSnapshot, TraceBuilder};
use evolve_fungible_asset::TransferMsg;
use evolve_server::Block;
use evolve_simulator::{SimConfig, SimStorageAdapter, Simulator};
use evolve_stf::gas::GasCounter;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{Block as BlockTrait, Transaction};
use evolve_testing::server_mocks::AccountStorageMock;
use evolve_token::account::TokenRef;

pub struct SimTestApp {
    sim: Simulator,
    codes: AccountStorageMock,
    stf: CustomStf,
    accounts: GenesisAccounts,
}

pub trait TxGenerator {
    fn generate_tx(&mut self, height: u64, sim: &mut Simulator) -> Option<TestTx>;
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
        sim: &mut Simulator,
        max_txs: usize,
    ) -> Vec<TestTx> {
        if self.generators.is_empty() || max_txs == 0 {
            return Vec::new();
        }

        let total_weight: u32 = self.generators.iter().map(|g| g.weight).sum();
        if total_weight == 0 {
            return Vec::new();
        }

        let mut txs = Vec::with_capacity(max_txs);
        for _ in 0..max_txs {
            let roll = sim.rng().gen_range(0..total_weight);
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
                if let Some(tx) = self.generators[idx].generator.generate_tx(height, sim) {
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
    fn generate_tx(&mut self, _height: u64, sim: &mut Simulator) -> Option<TestTx> {
        if self.senders.is_empty() || self.recipients.is_empty() {
            return None;
        }
        if self.min_amount > self.max_amount {
            return None;
        }

        let sender_idx = sim.rng().gen_range(0..self.senders.len());
        let recipient_idx = sim.rng().gen_range(0..self.recipients.len());
        let amount = sim.rng().gen_range(self.min_amount..=self.max_amount);

        let sender = self.senders[sender_idx];
        let recipient = self.recipients[recipient_idx];

        let request = InvokeRequest::new(&TransferMsg {
            to: recipient,
            amount,
        })
        .ok()?;

        Some(TestTx {
            sender,
            recipient: self.token_account,
            request,
            gas_limit: self.gas_limit,
            funds: vec![],
        })
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
    fn generate_tx(&mut self, _height: u64, sim: &mut Simulator) -> Option<TestTx> {
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

        let sender_idx = sim.rng().gen_range(0..funded_accounts.len());
        let sender = funded_accounts[sender_idx];
        let sender_balance = self.balances.get(&sender).copied().unwrap_or(0);

        // Pick recipient different from sender
        let recipient = loop {
            let idx = sim.rng().gen_range(0..self.accounts.len());
            if self.accounts[idx] != sender {
                break self.accounts[idx];
            }
        };

        // Clamp amount to available balance
        let max_transfer = sender_balance.min(self.max_amount);
        if max_transfer < self.min_amount {
            return None;
        }
        let amount = sim.rng().gen_range(self.min_amount..=max_transfer);

        // Pre-update balances (optimistic)
        self.record_transfer(sender, recipient, amount);

        let request = InvokeRequest::new(&TransferMsg {
            to: recipient,
            amount,
        })
        .ok()?;

        Some(TestTx {
            sender,
            recipient: self.token_account,
            request,
            gas_limit: self.gas_limit,
            funds: vec![],
        })
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
    fn generate_tx(&mut self, _height: u64, sim: &mut Simulator) -> Option<TestTx> {
        if self.senders.is_empty() || self.recipients.is_empty() {
            return None;
        }

        let sender_idx = sim.rng().gen_range(0..self.senders.len());
        let recipient_idx = sim.rng().gen_range(0..self.recipients.len());
        let sender = self.senders[sender_idx];
        let recipient = self.recipients[recipient_idx];

        let request = InvokeRequest::new(&TransferMsg {
            to: recipient,
            amount: self.amount,
        })
        .ok()?;

        Some(TestTx {
            sender,
            recipient: self.token_account,
            request,
            gas_limit: self.gas_limit,
            funds: vec![],
        })
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
    fn generate_tx(&mut self, _height: u64, _sim: &mut Simulator) -> Option<TestTx> {
        if self.participants.len() < 2 {
            return None;
        }

        let sender = self.participants[self.current_sender_idx];
        let recipient_idx = (self.current_sender_idx + 1) % self.participants.len();
        let recipient = self.participants[recipient_idx];

        self.current_sender_idx = (self.current_sender_idx + 1) % self.participants.len();

        let request = InvokeRequest::new(&TransferMsg {
            to: recipient,
            amount: self.amount,
        })
        .ok()?;

        Some(TestTx {
            sender,
            recipient: self.token_account,
            request,
            gas_limit: self.gas_limit,
            funds: vec![],
        })
    }
}

impl SimTestApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: SimConfig, seed: u64) -> Self {
        let gas_config = default_gas_config();
        let bootstrap_stf = build_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
        let mut codes = AccountStorageMock::default();
        install_account_codes(&mut codes);

        let mut sim = Simulator::new(seed, config);
        let adapter = SimStorageAdapter::new(sim.storage());
        let (genesis_state, accounts) = do_genesis(&bootstrap_stf, &codes, &adapter).unwrap();
        sim.apply_state_changes(genesis_state.into_changes().unwrap())
            .unwrap();

        let stf = build_stf(gas_config, accounts.scheduler);
        Self {
            sim,
            codes,
            stf,
            accounts,
        }
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

    pub fn apply_block(&mut self, block: &Block<TestTx>) -> BlockResult {
        let adapter = SimStorageAdapter::new(self.sim.storage());
        let (result, state) = self.stf.apply_block(&adapter, &self.codes, block);
        self.sim
            .apply_state_changes(state.into_changes().expect("state changes"))
            .expect("apply state changes");
        result
    }

    pub fn apply_block_with_trace(
        &mut self,
        block: &Block<TestTx>,
        builder: &mut TraceBuilder,
    ) -> BlockResult {
        let height = block.context().height;
        let timestamp_ms = self.sim.time().now_ms();
        builder.block_start(height, timestamp_ms);

        for tx in block.txs() {
            builder.tx_start(tx.compute_identifier(), tx.sender, tx.recipient);
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
        F: FnMut(u64, &mut Simulator) -> Vec<TestTx>,
    {
        let mut results = Vec::with_capacity(num_blocks as usize);
        let max_txs = self.sim.config().max_txs_per_block;

        for _ in 0..num_blocks {
            let height = self.sim.time().block_height();
            let mut txs = make_txs(height, &mut self.sim);
            if txs.len() > max_txs {
                txs.truncate(max_txs);
            }

            let block = Block::for_testing(height, txs);
            let result = self.apply_block(&block);
            results.push(result);
            self.sim.advance_block();
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
            let txs = registry.generate_block(height, &mut self.sim, max_txs);
            let block = Block::for_testing(height, txs);
            let result = self.apply_block(&block);
            results.push(result);
            self.sim.advance_block();
        }

        results
    }

    pub fn run_blocks_with_trace<F>(
        &mut self,
        num_blocks: u64,
        mut make_txs: F,
    ) -> (Vec<BlockResult>, ExecutionTrace)
    where
        F: FnMut(u64, &mut Simulator) -> Vec<TestTx>,
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
            let mut txs = make_txs(height, &mut self.sim);
            if txs.len() > max_txs {
                txs.truncate(max_txs);
            }

            let block = Block::for_testing(height, txs);
            let result = self.apply_block_with_trace(&block, &mut builder);
            results.push(result);
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
        let mut eth_address = [0u8; 20];
        for byte in eth_address.iter_mut() {
            *byte = self.sim.rng().gen::<u8>();
        }
        self.create_eoa_with_address(eth_address)
    }

    /// Create an EOA with a specific Ethereum address.
    pub fn create_eoa_with_address(&mut self, eth_address: [u8; 20]) -> AccountId {
        let adapter = SimStorageAdapter::new(self.sim.storage());
        let block = BlockContext::new(self.sim.time().block_height(), 0);
        let (account_ref, state) = self
            .stf
            .system_exec(&adapter, &self.codes, block, |env| {
                Ok(EthEoaAccountRef::initialize(eth_address, env)?.0)
            })
            .unwrap();
        self.sim
            .apply_state_changes(state.into_changes().unwrap())
            .unwrap();
        account_ref.0
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
