use crate::ERR_EXEC_IN_QUERY;
use evolve_core::{ErrorCode, SdkResult};

pub const ERR_TOO_MANY_OBJECTS: ErrorCode = ErrorCode::new(20, "too many objects");

/// Maximum number of objects that can be created in a single transaction
const MAX_OBJECTS_PER_TRANSACTION: u64 = u16::MAX as u64;

/// Start index for block height in unique ID
const BLOCK_HEIGHT_START: usize = 0;
/// End index for block height in unique ID
const BLOCK_HEIGHT_END: usize = 8;
/// Start index for object ID in block scope unique ID  
const BLOCK_OBJECT_ID_START: usize = 8;
/// End index for object ID in block scope unique ID
const BLOCK_OBJECT_ID_END: usize = 16;
/// Start index for object ID in transaction ID (last 2 bytes)
const TX_OBJECT_ID_START: usize = 30;
/// End index for object ID in transaction ID
const TX_OBJECT_ID_END: usize = 32;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExecutionScope {
    BeginBlock(u64),
    EndBlock(u64),
    Transaction([u8; 32]),
    Query,
}

impl ExecutionScope {
    /// Creates a 32-byte unique ID for each scope.
    ///
    /// - **BeginBlock(height)**, **EndBlock(height)**:
    ///   `[ height(8 bytes) | object_id(8 bytes) | 16 zeros ]`
    ///
    /// - **Transaction(tx_id)**:
    ///   Copies the entire 32 bytes, then overwrites the *last 2 bytes* with
    ///   `object_id` as a `u16` (in big-endian). Errors if `object_id > 65535`.
    ///
    /// - **Query**: return an error.
    pub(crate) fn unique_id(&self, object_id: u64) -> SdkResult<[u8; 32]> {
        match self {
            // [ block_height (8 bytes), object_id (8 bytes), 16 zeros ]
            ExecutionScope::BeginBlock(height) | ExecutionScope::EndBlock(height) => {
                let mut result = [0u8; 32];
                result[BLOCK_HEIGHT_START..BLOCK_HEIGHT_END].copy_from_slice(&height.to_be_bytes());
                result[BLOCK_OBJECT_ID_START..BLOCK_OBJECT_ID_END]
                    .copy_from_slice(&object_id.to_be_bytes());
                // The last 16 bytes remain zero
                Ok(result)
            }

            // Transaction scope: overwrite last 2 bytes with object_id as u16
            ExecutionScope::Transaction(tx_id) => {
                if object_id > MAX_OBJECTS_PER_TRANSACTION {
                    return Err(ERR_TOO_MANY_OBJECTS);
                }
                let object_id_u16 = object_id as u16;
                let mut result = *tx_id;
                // Overwrite the final 2 bytes with object_id
                result[TX_OBJECT_ID_START..TX_OBJECT_ID_END]
                    .copy_from_slice(&object_id_u16.to_be_bytes());
                Ok(result)
            }

            // Query => error
            ExecutionScope::Query => Err(ERR_EXEC_IN_QUERY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_block_unique_id() {
        let scope = ExecutionScope::BeginBlock(42);
        let unique_id = scope.unique_id(100).unwrap();
        // First 8 bytes = block_height (42), next 8 bytes = object_id (100)
        assert_eq!(&unique_id[0..8], &42u64.to_be_bytes());
        assert_eq!(&unique_id[8..16], &100u64.to_be_bytes());
        // The remaining 16 bytes = zero
        assert!(unique_id[16..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_end_block_unique_id() {
        let scope = ExecutionScope::EndBlock(999);
        let unique_id = scope.unique_id(1234).unwrap();
        // 999 and 1234 in big-endian
        assert_eq!(&unique_id[0..8], &999u64.to_be_bytes());
        assert_eq!(&unique_id[8..16], &1234u64.to_be_bytes());
        // The remaining 16 bytes = zero
        assert!(unique_id[16..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_transaction_unique_id_success() {
        // Random example tx_id
        let tx_id = [0xAB; 32];
        let scope = ExecutionScope::Transaction(tx_id);
        let unique_id = scope.unique_id(42).unwrap();

        // The first 30 bytes should match the original tx_id
        assert_eq!(&unique_id[..30], &tx_id[..30]);

        // The last 2 bytes should be 42 in big-endian => 00 2A
        let last_two = &unique_id[30..32];
        assert_eq!(u16::from_be_bytes(last_two.try_into().unwrap()), 42);
    }

    #[test]
    fn test_transaction_unique_id_overflow() {
        // If object_id is bigger than 65535, we should return an error
        let tx_id = [0xAB; 32];
        let scope = ExecutionScope::Transaction(tx_id);
        let res = scope.unique_id(70000); // exceeds 65535
        assert!(
            res.is_err(),
            "Expected an overflow error for 2-byte object_id"
        );
    }

    #[test]
    fn test_query_unique_id() {
        let scope = ExecutionScope::Query;
        let res = scope.unique_id(42);
        assert!(res.is_err(), "Expected an error for Query scope");
    }
}
