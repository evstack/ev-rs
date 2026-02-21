//! Chain state persistence for dev mode.
//!
//! This module provides utilities for persisting chain state across restarts,
//! allowing dev nodes to resume from where they left off instead of re-running genesis.

use borsh::{BorshDeserialize, BorshSerialize};
use evolve_core::ReadonlyKV;
use evolve_storage::{Operation, Storage};

use crate::error::ServerError;

/// Key used to store chain state in storage.
pub const CHAIN_STATE_KEY: &[u8] = b"__evolve_chain_state__";

/// Persisted chain state - stored in storage to resume across restarts.
///
/// Generic over `G`, the genesis result type which must be serializable.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct ChainState<G> {
    /// Current block height.
    pub height: u64,
    /// Genesis result (accounts, tokens, etc. created during genesis).
    pub genesis_result: G,
}

/// Load existing chain state from storage.
///
/// Returns `None` if no chain state exists (genesis hasn't been run yet).
pub fn load_chain_state<G, S>(storage: &S) -> Option<ChainState<G>>
where
    G: BorshDeserialize,
    S: ReadonlyKV,
{
    storage
        .get(CHAIN_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|bytes| ChainState::<G>::try_from_slice(&bytes).ok())
}

/// Save chain state to storage.
///
/// This should be called on graceful shutdown to persist the current height.
pub async fn save_chain_state<G, S>(storage: &S, state: &ChainState<G>) -> Result<(), ServerError>
where
    G: BorshSerialize,
    S: Storage,
{
    let value =
        borsh::to_vec(state).map_err(|e| ServerError::Storage(format!("serialize: {e}")))?;
    storage
        .batch(vec![Operation::Set {
            key: CHAIN_STATE_KEY.to_vec(),
            value,
        }])
        .await
        .map_err(|e| ServerError::Storage(format!("batch: {:?}", e)))?;
    storage
        .commit()
        .await
        .map_err(|e| ServerError::Storage(format!("commit: {:?}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_storage::MockStorage;

    #[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize)]
    struct TestGenesis {
        account_id: u64,
    }

    #[test]
    fn test_chain_state_serialization() {
        let state = ChainState {
            height: 42,
            genesis_result: TestGenesis { account_id: 123 },
        };

        let bytes = borsh::to_vec(&state).unwrap();
        let decoded: ChainState<TestGenesis> = ChainState::try_from_slice(&bytes).unwrap();

        assert_eq!(decoded.height, 42);
        assert_eq!(decoded.genesis_result.account_id, 123);
    }

    #[tokio::test]
    async fn test_save_and_load_chain_state_roundtrip() {
        let storage = MockStorage::new();
        let state = ChainState {
            height: 77,
            genesis_result: TestGenesis { account_id: 999 },
        };

        save_chain_state(&storage, &state)
            .await
            .expect("save_chain_state should succeed");

        let loaded: Option<ChainState<TestGenesis>> = load_chain_state(&storage);
        let loaded = loaded.expect("chain state should be present");
        assert_eq!(loaded.height, 77);
        assert_eq!(loaded.genesis_result.account_id, 999);
    }

    #[tokio::test]
    async fn test_load_chain_state_returns_none_for_corrupted_bytes() {
        let storage = MockStorage::new();
        storage
            .batch(vec![Operation::Set {
                key: CHAIN_STATE_KEY.to_vec(),
                value: vec![0x01, 0x02, 0x03],
            }])
            .await
            .expect("mock batch should succeed");
        storage.commit().await.expect("mock commit should succeed");

        let loaded: Option<ChainState<TestGenesis>> = load_chain_state(&storage);
        assert!(loaded.is_none());
    }
}
