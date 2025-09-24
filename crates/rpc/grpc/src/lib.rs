//! gRPC server for the Evolve execution client.
//!
//! This crate provides a gRPC server that mirrors the Ethereum JSON-RPC API
//! but uses binary protobuf encoding for efficiency. It reuses the same
//! `StateProvider` trait and `SubscriptionManager` as the JSON-RPC server.

pub mod conversion;
pub mod error;
pub mod server;
pub mod services;

// Re-export generated protobuf types
pub mod proto {
    pub mod evolve {
        pub mod v1 {
            include!("generated/evolve.v1.rs");
        }
    }
}

// Re-export key types
pub use error::GrpcError;
pub use server::{GrpcServer, GrpcServerConfig};
