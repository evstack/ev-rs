//! Graceful shutdown handling.
//!
//! This module provides:
//! - Signal handling for SIGTERM/SIGINT
//! - Shutdown coordination across multiple components
//! - `ShutdownAware` trait for components that need cleanup

mod components;
mod coordinator;
mod signals;

pub use components::{FnComponent, ShutdownAware};
pub use coordinator::ShutdownCoordinator;
pub use signals::SignalHandler;
