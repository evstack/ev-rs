//! Implementation of EvnodeStfExecutor for MempoolStf from testapp.
//!
//! This module provides the integration between the evnode gRPC service
//! and the testapp STF implementation.

use alloy_primitives::B256;
use evolve_core::{BlockContext, ReadonlyKV};
use evolve_stf::execution_state::ExecutionState;
use evolve_stf::results::BlockResult;
use evolve_stf_traits::{AccountsCodeStorage, StateChange};
use evolve_testapp::{do_genesis_inner, MempoolStf};

use crate::error::EvnodeError;
use crate::service::{compute_state_root, EvnodeStfExecutor, ExecutorBlock};

impl<S, Codes> EvnodeStfExecutor<S, Codes> for MempoolStf
where
    S: ReadonlyKV,
    Codes: AccountsCodeStorage,
{
    fn execute_block<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        block: &ExecutorBlock,
    ) -> (BlockResult, ExecutionState<'a, S>) {
        self.apply_block(storage, codes, block)
    }

    fn run_genesis<'a>(
        &self,
        storage: &'a S,
        codes: &'a Codes,
        initial_height: u64,
        genesis_time: u64,
    ) -> Result<(Vec<StateChange>, B256), EvnodeError> {
        let genesis_block = BlockContext::new(initial_height, genesis_time);

        let (accounts, state) = self
            .system_exec(storage, codes, genesis_block, do_genesis_inner)
            .map_err(|e| EvnodeError::Genesis(format!("genesis execution failed: {:?}", e)))?;

        let changes = state.into_changes().map_err(|e| {
            EvnodeError::Genesis(format!("failed to get genesis state changes: {:?}", e))
        })?;

        // Compute a simple state root from changes
        let state_root = compute_state_root(&changes);

        tracing::info!(
            "Genesis completed: scheduler={:?}, alice={:?}, bob={:?}",
            accounts.scheduler,
            accounts.alice,
            accounts.bob
        );

        Ok((changes, state_root))
    }
}
