use crate::encoding::Encodable;
pub use crate::message::{InvokeRequest, InvokeResponse, Message};
use borsh::{BorshDeserialize, BorshSerialize};

// Re-export macros so modules don't need to import evolve_macros directly
pub use evolve_macros::{account_impl, skip_storage, storage, AccountState};

pub mod encoding;
pub mod error;
pub mod events_api;
pub mod fungible_asset;
pub mod low_level;
pub mod message;
pub mod runtime_api;
pub mod schema;
pub mod storage_api;

pub use error::ErrorCode;
pub use fungible_asset::FungibleAsset;

define_error!(ERR_ENCODING, 0x01, "encoding error");
define_error!(ERR_UNKNOWN_FUNCTION, 0x02, "unknown function");
define_error!(ERR_ACCOUNT_NOT_INIT, 0x03, "account not initialized");
define_error!(ERR_UNAUTHORIZED, 0x04, "unauthorized");
define_error!(ERR_NOT_PAYABLE, 0x05, "not payable");
define_error!(ERR_ONE_COIN, 0x06, "one coin");
define_error!(ERR_INCOMPATIBLE_FA, 0x07, "incompatible fungible asset");
define_error!(ERR_INSUFFICIENT_BALANCE, 0x08, "insufficient balance");
define_error!(ERR_OVERFLOW, 0x09, "amount overflow");

/// Maximum storage key payload size (in bytes).
///
/// Keys are encoded with a 2-byte length prefix into a fixed 256-byte storage key.
pub const MAX_STORAGE_KEY_SIZE: usize = 254;

#[cfg(feature = "error-decode")]
pub type SdkResult<T> = Result<T, crate::error::DecodedError>;
#[cfg(not(feature = "error-decode"))]
pub type SdkResult<T> = Result<T, ErrorCode>;

/// Canonical 32-byte account identity.
///
/// Byte representation is the only canonical form. No numeric interpretation is
/// required for correctness.
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, BorshSerialize, BorshDeserialize,
)]
pub struct AccountId([u8; 32]);

impl AccountId {
    /// Reserved invalid sentinel (all 0xFF).
    pub const fn invalid() -> AccountId {
        AccountId([0xFF; 32])
    }

    /// Construct from raw canonical bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return raw canonical bytes.
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Borrow raw canonical bytes.
    pub const fn as_bytes(&self) -> [u8; 32] {
        self.0
    }

    /// Backward-compatible constructor from `u128`.
    ///
    /// Encodes into the lower 16 bytes in big-endian order.
    pub const fn new(u: u128) -> Self {
        let mut out = [0u8; 32];
        let bytes = u.to_be_bytes();
        out[16] = bytes[0];
        out[17] = bytes[1];
        out[18] = bytes[2];
        out[19] = bytes[3];
        out[20] = bytes[4];
        out[21] = bytes[5];
        out[22] = bytes[6];
        out[23] = bytes[7];
        out[24] = bytes[8];
        out[25] = bytes[9];
        out[26] = bytes[10];
        out[27] = bytes[11];
        out[28] = bytes[12];
        out[29] = bytes[13];
        out[30] = bytes[14];
        out[31] = bytes[15];
        Self(out)
    }

    /// Legacy extractor for compatibility where a numeric ID is still required.
    pub const fn inner(&self) -> u128 {
        u128::from_be_bytes([
            self.0[16], self.0[17], self.0[18], self.0[19], self.0[20], self.0[21], self.0[22],
            self.0[23], self.0[24], self.0[25], self.0[26], self.0[27], self.0[28], self.0[29],
            self.0[30], self.0[31],
        ])
    }

    /// Increment account ID bytes in big-endian order (wraps on overflow).
    pub fn increase(&self) -> Self {
        let mut out = self.0;
        for i in (0..32).rev() {
            let (next, carry) = out[i].overflowing_add(1);
            out[i] = next;
            if !carry {
                break;
            }
        }
        Self(out)
    }
}

/// Block context available to all modules during execution.
///
/// Contains metadata about the current block. Accessed via `env.block()`.
#[derive(Copy, Clone, Debug, Default)]
pub struct BlockContext {
    pub height: u64,
    pub time: u64,
}

impl BlockContext {
    pub const fn new(height: u64, time: u64) -> Self {
        Self { height, time }
    }
}

pub trait EnvironmentQuery {
    fn whoami(&self) -> AccountId;
    fn sender(&self) -> AccountId;
    fn funds(&self) -> &[FungibleAsset];
    fn block(&self) -> BlockContext;
    fn do_query(&mut self, to: AccountId, data: &InvokeRequest) -> SdkResult<InvokeResponse>;
}

pub trait Environment: EnvironmentQuery {
    fn do_exec(
        &mut self,
        to: AccountId,
        data: &InvokeRequest,
        funds: Vec<FungibleAsset>,
    ) -> SdkResult<InvokeResponse>;

    /// Emits an event with the given name and data.
    ///
    /// Events are diagnostic data that get included in transaction/block results.
    /// They do not affect state and are used for indexing and observability.
    fn emit_event(&mut self, name: &str, data: &[u8]) -> SdkResult<()>;

    /// Generates a unique 32-byte identifier.
    ///
    /// The ID is deterministic and unique within the current execution context:
    /// - In transactions: derived from tx hash + per-tx counter
    /// - In begin/end block: derived from block height + per-block counter
    ///
    /// Returns an error if called during a query (queries cannot generate unique IDs).
    fn unique_id(&mut self) -> SdkResult<[u8; 32]>;
}

/// Defines some arbitrary code that can handle account execution logic.
pub trait AccountCode: Send + Sync {
    fn identifier(&self) -> String;
    fn schema(&self) -> schema::AccountSchema;
    fn init(&self, env: &mut dyn Environment, request: &InvokeRequest)
        -> SdkResult<InvokeResponse>;
    fn execute(
        &self,
        env: &mut dyn Environment,
        request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
    fn query(
        &self,
        env: &mut dyn EnvironmentQuery,
        request: &InvokeRequest,
    ) -> SdkResult<InvokeResponse>;
}

pub trait ReadonlyKV {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ErrorCode>;
}

pub trait InvokableMessage: Encodable + Clone {
    const FUNCTION_IDENTIFIER: u64;
    const FUNCTION_IDENTIFIER_NAME: &'static str;
}

#[macro_export]
macro_rules! ensure {
    ($cond:expr, $err:expr) => {
        if !$cond {
            return Err($err.into());
        }
    };
}

pub fn one_coin(env: &dyn EnvironmentQuery) -> SdkResult<FungibleAsset> {
    let funds = env.funds();
    ensure!(funds.len() == 1, ERR_ONE_COIN);
    Ok(FungibleAsset {
        asset_id: funds[0].asset_id,
        amount: funds[0].amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use borsh::{BorshDeserialize, BorshSerialize};
    use std::collections::BTreeMap;

    struct TestEnv {
        whoami: AccountId,
        sender: AccountId,
        funds: Vec<FungibleAsset>,
    }

    impl EnvironmentQuery for TestEnv {
        fn whoami(&self) -> AccountId {
            self.whoami
        }

        fn sender(&self) -> AccountId {
            self.sender
        }

        fn funds(&self) -> &[FungibleAsset] {
            &self.funds
        }

        fn block(&self) -> BlockContext {
            BlockContext::new(0, 0)
        }

        fn do_query(&mut self, _to: AccountId, _data: &InvokeRequest) -> SdkResult<InvokeResponse> {
            Err(ERR_UNKNOWN_FUNCTION)
        }
    }

    impl Environment for TestEnv {
        fn do_exec(
            &mut self,
            _to: AccountId,
            _data: &InvokeRequest,
            _funds: Vec<FungibleAsset>,
        ) -> SdkResult<InvokeResponse> {
            Err(ERR_UNKNOWN_FUNCTION)
        }

        fn emit_event(&mut self, _name: &str, _data: &[u8]) -> SdkResult<()> {
            Ok(())
        }

        fn unique_id(&mut self) -> SdkResult<[u8; 32]> {
            Ok([0u8; 32])
        }
    }

    #[derive(Clone, BorshSerialize, BorshDeserialize)]
    struct DummyMessage {
        value: u64,
    }

    impl InvokableMessage for DummyMessage {
        const FUNCTION_IDENTIFIER: u64 = 42;
        const FUNCTION_IDENTIFIER_NAME: &'static str = "dummy";
    }

    #[test]
    fn test_message_roundtrip() {
        let msg = DummyMessage { value: 123 };
        let encoded = msg.encode().unwrap();
        let decoded = borsh::from_slice::<DummyMessage>(&encoded).unwrap();
        assert_eq!(decoded.value, 123);
    }

    #[test]
    fn test_one_coin_success() {
        let env = TestEnv {
            whoami: AccountId::new(1),
            sender: AccountId::new(2),
            funds: vec![FungibleAsset {
                asset_id: AccountId::new(10),
                amount: 100,
            }],
        };

        let coin = one_coin(&env).unwrap();
        assert_eq!(coin.asset_id, AccountId::new(10));
        assert_eq!(coin.amount, 100);
    }

    #[test]
    fn test_one_coin_error() {
        let env = TestEnv {
            whoami: AccountId::new(1),
            sender: AccountId::new(2),
            funds: vec![],
        };

        let err = one_coin(&env).unwrap_err();
        assert_eq!(err, ERR_ONE_COIN);
    }

    #[test]
    fn test_storage_key_size_constant() {
        assert_eq!(MAX_STORAGE_KEY_SIZE, 254);
    }

    #[test]
    fn test_account_id_u128_compat() {
        let id = AccountId::new(42u128);
        assert_eq!(id.inner(), 42u128);

        let mut expected = [0u8; 32];
        expected[31] = 42;
        assert_eq!(id.as_bytes(), expected);
    }

    #[test]
    fn test_account_id_increase() {
        let a = AccountId::from_bytes([0u8; 32]);
        let b = a.increase();
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(b.as_bytes(), expected);
    }

    #[test]
    fn test_ensure_macro() {
        fn test_fn(val: i32) -> SdkResult<()> {
            ensure!(val > 10, ERR_UNAUTHORIZED);
            Ok(())
        }

        assert!(test_fn(11).is_ok());
        assert_eq!(test_fn(5).unwrap_err(), ERR_UNAUTHORIZED);
    }

    #[test]
    fn test_message_btreemap_roundtrip() {
        let mut map = BTreeMap::new();
        map.insert("a".to_string(), 1u64);
        map.insert("b".to_string(), 2u64);

        let msg = Message::new(&map).unwrap();
        let decoded: BTreeMap<String, u64> = msg.get().unwrap();

        assert_eq!(decoded.get("a"), Some(&1u64));
        assert_eq!(decoded.get("b"), Some(&2u64));
    }
}
