//! Transaction-specific error types.

use evolve_core::define_error;

// Signature/verification errors (0x10-0x1F range - validation errors)
define_error!(ERR_INVALID_SIGNATURE, 0x10, "invalid transaction signature");
define_error!(
    ERR_SIGNATURE_RECOVERY,
    0x11,
    "failed to recover signer from signature"
);
define_error!(ERR_INVALID_CHAIN_ID, 0x12, "chain ID mismatch");
define_error!(
    ERR_UNSUPPORTED_TX_TYPE,
    0x13,
    "unsupported transaction type {arg}"
);
define_error!(ERR_TX_DECODE, 0x14, "failed to decode transaction");
define_error!(ERR_EMPTY_INPUT, 0x15, "empty transaction input");
define_error!(ERR_INVALID_TX_HASH, 0x16, "transaction hash mismatch");

// Nonce errors (0x18-0x1B range)
define_error!(ERR_NONCE_TOO_LOW, 0x18, "nonce too low");
define_error!(ERR_NONCE_TOO_HIGH, 0x19, "nonce too high");

// Gas errors (0x1C-0x1F range)
define_error!(ERR_GAS_LIMIT_TOO_LOW, 0x1C, "gas limit too low");
define_error!(
    ERR_GAS_LIMIT_TOO_HIGH,
    0x1D,
    "gas limit exceeds block limit"
);
