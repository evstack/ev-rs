//! Ethereum-compatible typed transaction support for Evolve.
//!
//! This crate provides EIP-2718 typed transaction envelopes with support for
//! both standard Ethereum transaction types and custom Evolve types.
//!
//! # Transaction Types
//!
//! ## Ethereum Standard Types
//! - **Legacy (0x00)**: Pre-EIP-2718 transactions, optionally with EIP-155 replay protection
//! - **EIP-1559 (0x02)**: Fee market transactions with base fee and priority fee
//!
//! ## Custom Evolve Types (planned)
//! - **Batch (0x80)**: Multi-message transactions (like Cosmos)
//! - **Sponsored (0x81)**: Gasless/meta-transactions
//! - **Scheduled (0x82)**: Delayed execution transactions
//!
//! # Usage
//!
//! ```ignore
//! use evolve_tx::{TxEnvelope, TypedTxDecoder, SignatureVerifierRegistry};
//! use evolve_stf_traits::TxDecoder;
//!
//! // Create decoder and verifier
//! let decoder = TypedTxDecoder::ethereum();
//! let verifier = SignatureVerifierRegistry::ethereum(1); // chain_id = 1
//!
//! // Decode a raw transaction
//! let raw_tx: &[u8] = &[...];
//! let tx = decoder.decode(&mut &raw_tx[..])?;
//!
//! // Verify the signature
//! verifier.verify(&tx)?;
//!
//! // Access transaction data
//! let sender = tx.sender();
//! let hash = tx.tx_hash();
//! ```
//!
//! # Architecture
//!
//! The crate uses a trait-based design allowing new transaction types to be added:
//!
//! 1. [`TypedTransaction`] - Core trait all transaction types implement
//! 2. [`TxEnvelope`] - Enum holding any supported transaction type
//! 3. [`SignatureVerifierRegistry`] - Maps tx types to their verifiers
//! 4. [`TypedTxDecoder`] - Decodes raw bytes with type filtering

pub mod decoder;
pub mod envelope;
pub mod error;
pub mod ethereum;
pub mod traits;
pub mod verifier;

// Re-export main types
pub use decoder::TypedTxDecoder;
pub use envelope::{tx_type, TxEnvelope};
pub use error::*;
pub use ethereum::{SignedEip1559Tx, SignedLegacyTx};
pub use traits::{account_id_to_address, address_to_account_id, TypedTransaction};
pub use verifier::{EcdsaVerifier, SignatureVerifierRegistry};
