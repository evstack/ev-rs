//! Ethereum-compatible typed transaction support for Evolve.
//!
//! This crate provides EIP-2718 typed transaction envelopes with support for
//! standard Ethereum transaction types.
//!
//! # Transaction Types
//!
//! - **Legacy (0x00)**: Pre-EIP-2718 transactions, optionally with EIP-155 replay protection
//! - **EIP-1559 (0x02)**: Fee market transactions with base fee and priority fee
//!
//! # Usage
//!
//! ```text
//! use evolve_tx_eth::{EthGateway, TxContext};
//!
//! // Create a gateway for transaction validation
//! let gateway = EthGateway::new(1337);
//!
//! // Decode and verify a raw transaction
//! let raw_tx: &[u8] = &[...];
//! let tx_context = gateway.decode_and_verify(raw_tx)?;
//!
//! // Add to mempool
//! mempool.add(tx_context)?;
//! ```
//!
//! # Architecture
//!
//! The crate uses a trait-based design:
//!
//! 1. [`TypedTransaction`] - Core trait all transaction types implement
//! 2. [`TxEnvelope`] - Enum holding any supported transaction type
//! 3. [`EthGateway`] - Decodes and verifies raw transactions
//! 4. [`TxContext`] - Verified transaction implementing `MempoolTx`

pub mod decoder;
pub mod envelope;
pub mod eoa_registry;
pub mod error;
pub mod ethereum;
pub mod gateway;
pub mod mempool;
pub mod payload;
pub mod sender_type;
pub mod traits;
pub mod verifier;

// Re-export main types
pub use decoder::TypedTxDecoder;
pub use envelope::{tx_type, TxEnvelope};
pub use eoa_registry::{
    lookup_account_id_in_env, lookup_account_id_in_storage, lookup_address_in_env,
    lookup_address_in_storage, lookup_contract_account_id_in_env,
    lookup_contract_account_id_in_storage, register_runtime_contract_account,
    resolve_or_create_eoa_account,
};
pub use error::*;
pub use ethereum::{SignedEip1559Tx, SignedLegacyTx};
pub use gateway::{EthGateway, GatewayError};
pub use mempool::{TxContext, TxContextMeta};
pub use payload::{EthIntentPayload, TxPayload};
pub use sender_type as sender_types;
pub use traits::{
    derive_eth_eoa_account_id, derive_runtime_contract_account_id, derive_runtime_contract_address,
    derive_system_account_id, TypedTransaction,
};
pub use verifier::{EcdsaVerifier, SignatureVerifierDyn, SignatureVerifierRegistry};
