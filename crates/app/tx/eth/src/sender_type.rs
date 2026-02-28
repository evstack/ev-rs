//! Sender type constants for signature verifier dispatch.
//!
//! Sender types are intentionally decoupled from Ethereum tx envelope types.
//! Multiple tx envelope formats can use the same sender authentication scheme.

/// Sender type identifier for Ethereum EOA secp256k1 signatures.
pub const EOA_SECP256K1: u16 = 0x0001;

/// Sender type identifier for custom account-authenticated payloads.
pub const CUSTOM: u16 = 0x8000;
