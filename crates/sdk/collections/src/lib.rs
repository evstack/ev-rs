use evolve_core::ErrorCode;

pub mod item;
pub mod map;

pub const ERR_NOT_FOUND: ErrorCode = ErrorCode::new(404, "object not found");
