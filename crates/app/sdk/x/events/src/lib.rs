//! # Events Module (Deprecated)
//!
//! Event emission is now built into the `Environment` trait directly.
//! Use `env.emit_event(name, data)` instead of `EventsEmitter`.
//!
//! This module is kept for backwards compatibility and re-exports `Event` from core.

#[deprecated(
    since = "0.2.0",
    note = "Use env.emit_event(name, data) directly instead of EventsEmitter"
)]
pub use evolve_core::events_api::Event;
