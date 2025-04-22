use crate::types::{extract_begin_end_block_events, TendermintBlock};
use evolve_cometbft_account_trait::consensus_account::{
    AbciValsetManagerAccountRef, Pubkey, ValidatorUpdate,
};
use evolve_core::{AccountId, SdkResult};
use evolve_server_core::{
    AccountsCodeStorage, BeginBlocker, EndBlocker, PostTxExecution, Transaction, TxDecoder,
    TxValidator, WritableAccountsCodeStorage, WritableKV,
};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::Stf;
use tendermint::abci::{request, response};
use tendermint::block::Height;
use tendermint::AppHash;

pub type InitChainer<Stf, Storage, AccountCodes> =
    for<'a> fn(&'a Stf, &'a Storage, &'a AccountCodes) -> SdkResult<ExecutionState<'a, Storage>>;

pub struct Consensus<A, T, D, Bb, Eb, TxVal, Pt, S> {
    height: Height,
    decoder: D,
    storage: S,
    account_codes: A,
    stf: Stf<T, TendermintBlock<T>, Bb, TxVal, Eb, Pt>,
    init_chainer: InitChainer<Stf<T, TendermintBlock<T>, Bb, TxVal, Eb, Pt>, S, A>,
}

impl<A: WritableAccountsCodeStorage, T, D, Bb, Eb, TxVal, Pt, S>
    Consensus<A, T, D, Bb, Eb, TxVal, Pt, S>
{
    pub fn new(
        decoder: D,
        storage: S,
        account_codes: A,
        stf: Stf<T, TendermintBlock<T>, Bb, TxVal, Eb, Pt>,
        init_chainer: InitChainer<Stf<T, TendermintBlock<T>, Bb, TxVal, Eb, Pt>, S, A>,
    ) -> Self {
        Self {
            height: Height::try_from(0u64).unwrap(),
            decoder,
            storage,
            account_codes,
            stf,
            init_chainer,
        }
    }
}
impl<A, T, D, Bb, Eb, TxVal, Pt, S> Clone for Consensus<A, T, D, Bb, Eb, TxVal, Pt, S>
where
    A: AccountsCodeStorage + Clone,
    Bb: BeginBlocker<TendermintBlock<T>> + Clone,
    D: TxDecoder<T> + Clone,
    Eb: EndBlocker + Clone,
    Pt: PostTxExecution<T> + Clone,
    S: WritableKV + Clone,
    T: Transaction,
    TxVal: TxValidator<T> + Clone,
{
    fn clone(&self) -> Self {
        Self {
            height: self.height,
            account_codes: self.account_codes.clone(),
            decoder: self.decoder.clone(),
            storage: self.storage.clone(),
            stf: self.stf.clone(),
            init_chainer: self.init_chainer,
        }
    }
}

impl<A, T, D, Bb, Eb, TxVal, Pt, S> Consensus<A, T, D, Bb, Eb, TxVal, Pt, S>
where
    A: AccountsCodeStorage,
    T: Transaction,
    D: TxDecoder<T>,
    Bb: BeginBlocker<TendermintBlock<T>>,
    Eb: EndBlocker,
    TxVal: TxValidator<T>,
    Pt: PostTxExecution<T>,
    S: WritableKV,
{
    pub(crate) fn finalize_block(
        &self,
        request: request::FinalizeBlock,
    ) -> response::FinalizeBlock {
        // convert block
        let mut block = TendermintBlock::from_request_finalize_block(&self.decoder, request);

        let (mut block_result, new_state) =
            self.stf
                .apply_block(&self.storage, &self.account_codes, &block);

        // extract valset changes
        let valset_changes = self
            .stf
            .run_with_ref(
                &new_state,
                &self.account_codes,
                AccountId::new(65539), // TODO fix me
                |valset_manager_account: AbciValsetManagerAccountRef, env| {
                    valset_manager_account.valset_changes(env)
                },
            )
            .unwrap();

        response::FinalizeBlock {
            events: extract_begin_end_block_events(&mut block_result),
            tx_results: block.extract_tx_results(block_result),
            validator_updates: valset_changes
                .into_iter()
                .map(valset_changes_to_validator_updates)
                .collect(),
            consensus_param_updates: None,
            app_hash: Default::default(),
        }
    }

    pub(crate) fn info(&self) -> response::Info {
        response::Info {
            data: "tower-abci-kvstore-example".to_string(),
            version: "0.1.0".to_string(),
            app_version: 1,
            last_block_height: self.height,
            last_block_app_hash: AppHash::try_from(Vec::<u8>::new()).unwrap(),
        }
    }

    pub(crate) fn query(&self, _query: request::Query) -> response::Query {
        todo!()
    }

    pub(crate) fn commit(&mut self) -> response::Commit {
        self.height = self.height.increment();
        response::Commit {
            data: Default::default(),
            retain_height: Default::default(),
        }
    }

    pub(crate) fn extend_vote(&self, _vote: request::ExtendVote) -> response::ExtendVote {
        response::ExtendVote {
            vote_extension: Default::default(),
        }
    }

    pub(crate) fn verify_vote(
        &self,
        _vote: request::VerifyVoteExtension,
    ) -> response::VerifyVoteExtension {
        response::VerifyVoteExtension::Accept
    }

    pub(crate) fn init_chain(&mut self, genesis: request::InitChain) -> response::InitChain {
        log::debug!("processing init chain: {:?}", genesis);

        let genesis_result: SdkResult<ExecutionState<_>> =
            (self.init_chainer)(&self.stf, &self.storage, &self.account_codes);
        let state_changes = genesis_result
            .expect("init chain failed")
            .into_changes()
            .expect("unable to get state changes");

        // commit
        self.storage
            .apply_changes(state_changes)
            .expect("changes application failed");

        response::InitChain {
            consensus_params: Option::from(genesis.consensus_params),
            validators: genesis.validators,
            app_hash: Default::default(),
        }
    }
}

/// Converts an account trait valset update into a proto validator update
fn valset_changes_to_validator_updates(val: ValidatorUpdate) -> tendermint::validator::Update {
    let pub_key = match val.pub_key {
        Pubkey::Ed25519(v) => {
            tendermint::crypto::ed25519::VerificationKey::try_from(v.as_slice()).unwrap()
        }
    };
    tendermint::validator::Update {
        pub_key: tendermint::public_key::PublicKey::Ed25519(pub_key),
        power: val.power.into(),
    }
}
