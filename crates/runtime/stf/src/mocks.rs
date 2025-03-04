use crate::Stf;
use evolve_core::{
    AccountId, Environment, FungibleAsset, InvokeRequest, InvokeResponse, SdkResult,
};
use evolve_server_core::{
    BeginBlocker, Block, EndBlocker, PostTxExecution, Transaction, TxValidator,
};

pub(crate) struct MockTx;

impl Transaction for MockTx {
    fn sender(&self) -> AccountId {
        todo!()
    }

    fn recipient(&self) -> AccountId {
        todo!()
    }

    fn request(&self) -> &InvokeRequest {
        todo!()
    }

    fn gas_limit(&self) -> u64 {
        todo!()
    }

    fn funds(&self) -> &[FungibleAsset] {
        todo!()
    }
}

pub(crate) struct MockBlock(u64, Vec<MockTx>);

impl Block<MockTx> for MockBlock {
    fn height(&self) -> u64 {
        self.0
    }

    fn txs(&self) -> &[MockTx] {
        self.1.as_slice()
    }
}

pub(crate) struct NoOpBeginBlocker;

impl BeginBlocker<MockBlock> for NoOpBeginBlocker {
    fn begin_block(_block: &MockBlock, _env: &mut dyn Environment) {}
}

pub(crate) struct NoOpTxValidator;

impl TxValidator<MockTx> for NoOpTxValidator {
    fn validate_tx(_tx: &MockTx, _env: &mut dyn Environment) -> SdkResult<()> {
        Ok(())
    }
}

pub(crate) struct NoOpEndBlocker;

impl EndBlocker for NoOpEndBlocker {
    fn end_block(env: &mut dyn Environment) {}
}

pub(crate) struct NoOpPostTx;

impl PostTxExecution<MockTx> for NoOpPostTx {
    fn after_tx_executed(
        _tx: &MockTx,
        _gas_consumed: u64,
        _tx_result: SdkResult<InvokeResponse>,
        _env: &mut dyn Environment,
    ) -> SdkResult<()> {
        Ok(())
    }
}

pub(crate) type TestStf =
Stf<MockTx, MockBlock, NoOpBeginBlocker, NoOpTxValidator, NoOpEndBlocker, NoOpPostTx>;
