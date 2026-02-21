// Automaton uses SystemTime for block timestamps in propose().
// This is acceptable in a consensus proposer context (non-deterministic timestamp
// is validated by verifiers and consensus).
#![allow(clippy::disallowed_types)]

pub mod automaton;
pub mod block;
pub mod config;
pub mod relay;
pub mod reporter;

pub use automaton::EvolveAutomaton;
pub use block::ConsensusBlock;
pub use config::ConsensusConfig;
pub use relay::EvolveRelay;
pub use reporter::EvolveReporter;
