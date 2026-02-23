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
pub mod error;
pub mod ethereum;
pub mod gateway;
pub mod mempool;
pub mod traits;
pub mod verifier;

// Re-export main types
pub use decoder::TypedTxDecoder;
pub use envelope::{tx_type, TxEnvelope};
pub use error::*;
pub use ethereum::{SignedEip1559Tx, SignedLegacyTx};
pub use gateway::{EthGateway, GatewayError};
pub use mempool::TxContext;
pub use traits::{account_id_to_address, address_to_account_id, TypedTransaction};
pub use verifier::{EcdsaVerifier, SignatureVerifierRegistry};
