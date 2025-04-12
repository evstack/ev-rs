pub mod mocks; // TODO: make test

use evolve_core::{
    AccountCode, AccountId, Environment, ErrorCode, FungibleAsset, InvokeRequest, InvokeResponse,
    ReadonlyKV, SdkResult,
};

pub trait Transaction {
    fn sender(&self) -> AccountId;
    fn recipient(&self) -> AccountId;
    fn request(&self) -> &InvokeRequest;
    fn gas_limit(&self) -> u64;
    fn funds(&self) -> &[FungibleAsset];
}

pub trait Block<Tx> {
    fn height(&self) -> u64;
    fn txs(&self) -> &[Tx];
}

pub trait TxValidator<Tx> {
    fn validate_tx(tx: &Tx, env: &mut dyn Environment) -> SdkResult<()>;
}

pub trait PostTxExecution<Tx> {
    fn after_tx_executed(
        tx: &Tx,
        gas_consumed: u64,
        tx_result: SdkResult<InvokeResponse>,
        env: &mut dyn Environment,
    ) -> SdkResult<()>;
}

pub trait BeginBlocker<B> {
    fn begin_block(block: &B, env: &mut dyn Environment);
}

pub trait EndBlocker {
    fn end_block(env: &mut dyn Environment);
}

/// Stores account code.
pub trait AccountsCodeStorage {
    /// TODO: this probably needs to consume gas, and should accept gas.
    fn with_code<F, R>(&self, identifier: &str, f: F) -> Result<R, ErrorCode>
    where
        F: FnOnce(Option<&dyn AccountCode>) -> R;
}

#[derive(Debug)]
pub enum StateChange {
    Set { key: Vec<u8>, value: Vec<u8> },
    Remove { key: Vec<u8> },
}

pub trait WritableKV: ReadonlyKV {
    fn apply_changes(&mut self, changes: Vec<StateChange>) -> Result<(), ErrorCode>;
}
