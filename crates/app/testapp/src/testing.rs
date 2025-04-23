use crate::{do_genesis, install_account_codes, CustomStf, MINTER, STF};
use evolve_block_info::account::BlockInfoRef;
use evolve_core::{AccountId, Environment, FungibleAsset, SdkResult};
use evolve_ns::resolve_as_ref;
use evolve_server_core::WritableKV;
use evolve_testing::server_mocks::{AccountStorageMock, StorageMock};
use evolve_token::account::TokenRef;

pub struct TestApp {
    block: u64,
    codes: AccountStorageMock,
    state: StorageMock,
    stf: CustomStf,
}

impl TestApp {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn sudo_as<R>(
        &mut self,
        impersonate: AccountId,
        action: impl Fn(&mut dyn Environment) -> SdkResult<R>,
    ) -> SdkResult<R> {
        let (resp, state) =
            self.stf
                .sudo_as(&self.state, &self.codes, self.block, impersonate, action)?;
        let changes = state.into_changes()?;
        self.state.apply_changes(changes)?;
        Ok(resp)
    }

    pub fn next_block(&mut self) {
        self.block += 1;
    }

    pub fn mint_atom(&mut self, recipient: AccountId, amount: u128) -> FungibleAsset {
        self.sudo_as(MINTER, |env| {
            let atom_token = resolve_as_ref::<TokenRef>("atom", env)?.unwrap();
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
        let state = self
            .stf
            .sudo(&self.state, &self.codes, height, |env| {
                let block_info = resolve_as_ref::<BlockInfoRef>("block_info", env)?.unwrap();
                block_info.set_block_info(height, 0, env)
            })
            .unwrap()
            .1;
        let changes = state.into_changes().unwrap();
        self.state.apply_changes(changes).unwrap();
    }
}

impl Default for TestApp {
    fn default() -> Self {
        // install stf
        let stf = STF;
        let mut codes = AccountStorageMock::default();
        // install codes
        install_account_codes(&mut codes);

        let mut state = StorageMock::default();

        let genesis_state = do_genesis(&stf, &codes, &state).unwrap();
        state
            .apply_changes(genesis_state.into_changes().unwrap())
            .unwrap();

        Self {
            block: 0,
            codes,
            state,
            stf: STF,
        }
    }
}
