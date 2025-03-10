use std::fmt::{Debug, Display};

#[derive(Debug, PartialEq, Eq)]
pub struct ErrorCode {
    human: &'static str,
    code: u64,
}

impl ErrorCode {
    pub fn code(&self) -> u64 {
        self.code
    }
}

impl ErrorCode {
    pub const fn new(code: u64, human: &'static str) -> ErrorCode {
        ErrorCode { human, code }
    }
}
