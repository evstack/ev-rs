use evolve_core::define_error;

pub mod item;
pub mod map;
pub mod unordered_map;
pub mod vector;

#[cfg(test)]
mod mocks;
#[cfg(test)]
mod prop_tests;
pub mod queue;

define_error!(ERR_NOT_FOUND, 0x01, "object not found");
define_error!(ERR_EMPTY, 0x02, "queue is empty");
define_error!(ERR_DATA_CORRUPTION, 0x03, "data corruption");
define_error!(ERR_VALUE_MISSING, 0x04, "value missing for key");
define_error!(ERR_KEY_MISSING, 0x05, "key missing in vector");
