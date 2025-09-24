use crate::{
    build_stf, default_gas_config, do_genesis, install_account_codes, CustomStf, GenesisAccounts,
    MINTER, PLACEHOLDER_ACCOUNT,
};
use evolve_core::{AccountId, BlockContext, Environment, FungibleAsset, SdkResult};
use evolve_stf_traits::WritableKV;
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};
use evolve_token::account::TokenRef;

pub struct TestApp {
    block: u64,
    codes: AccountStorageMock,
    state: StorageMock,
    stf: CustomStf,
    accounts: GenesisAccounts,
}

impl TestApp {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn accounts(&self) -> GenesisAccounts {
        self.accounts
    }

    pub fn system_exec_as<R>(
        &mut self,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let block = BlockContext::new(self.block, 0);
        let (resp, state) =
            self.stf
                .system_exec_as(&self.state, &self.codes, block, impersonate, action)?;
        let changes = state.into_changes()?;
        self.state.apply_changes(changes)?;
        Ok(resp)
    }

    pub fn next_block(&mut self) {
        self.block += 1;
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
        assert!(self.block < height);
        self.block = height;
    }
}

impl Default for TestApp {
    fn default() -> Self {
        let gas_config = default_gas_config();
        // install stf
        let bootstrap_stf = build_stf(gas_config.clone(), PLACEHOLDER_ACCOUNT);
        let mut codes = AccountStorageMock::default();
        // install codes
        install_account_codes(&mut codes);

        let mut state = StorageMock::default();

        let (genesis_state, accounts) = do_genesis(&bootstrap_stf, &codes, &state).unwrap();
        state
            .apply_changes(genesis_state.into_changes().unwrap())
            .unwrap();

        Self {
            block: 0,
            codes,
            state,
            stf: build_stf(gas_config, accounts.scheduler),
            accounts,
        }
    }
}
