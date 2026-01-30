//! Signature verification for transactions.
//!
//! This module provides extensible signature verification through a registry
//! pattern, allowing different transaction types to have different verification
//! logic.

mod ecdsa;
mod registry;

pub use ecdsa::EcdsaVerifier;
pub use registry::SignatureVerifierRegistry;
