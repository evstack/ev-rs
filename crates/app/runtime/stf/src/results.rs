use evolve_core::events_api::Event;
use evolve_core::{InvokeResponse, SdkResult};
use evolve_server_core::StateChange;

#[derive(Debug)]
pub struct TxResult {
    pub events: Vec<Event>,
    pub gas_used: u64,
    pub response: SdkResult<InvokeResponse>,
}

#[derive(Debug)]
pub struct BlockResult {
    pub begin_block_events: Vec<Event>,
    pub state_changes: Vec<StateChange>,
    pub tx_results: Vec<TxResult>,
    pub end_block_events: Vec<Event>,
}
