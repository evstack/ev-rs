//! # Validation Module
//!
//! This module provides validation functions for various blockchain operations
//! including code ID validation and event validation. It enforces security
//! constraints and ensures data integrity.

use evolve_core::SdkResult;

use crate::{
    ERR_EVENT_CONTENT_TOO_LARGE, ERR_EVENT_NAME_TOO_LONG, ERR_INVALID_CODE_ID, MAX_CODE_ID_LENGTH,
    MAX_EVENT_CONTENT_SIZE, MAX_EVENT_NAME_LENGTH,
};

/// Validates a code identifier for account creation.
///
/// Ensures that the code ID meets security requirements:
/// - Non-empty and within length limits
/// - Contains only alphanumeric characters, dashes, underscores, and slashes
///
/// # Arguments
///
/// * `code_id` - The code identifier to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error if validation fails.
pub fn validate_code_id(code_id: &str) -> SdkResult<()> {
    // Validate code_id
    if code_id.is_empty() || code_id.len() > MAX_CODE_ID_LENGTH {
        return Err(ERR_INVALID_CODE_ID);
    }

    // Validate code_id contains only valid characters (alphanumeric, dash, underscore, slash)
    if !code_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/')
    {
        return Err(ERR_INVALID_CODE_ID);
    }

    Ok(())
}

/// Validates event data before emission.
///
/// Ensures that event data meets size and format requirements:
/// - Event name length within limits
/// - Event content size within limits
///
/// # Arguments
///
/// * `name` - The event name to validate
/// * `contents` - The event contents to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error if validation fails.
pub fn validate_event(name: &str, contents: &[u8]) -> SdkResult<()> {
    // Validate event name length
    if name.len() > MAX_EVENT_NAME_LENGTH {
        return Err(ERR_EVENT_NAME_TOO_LONG);
    }

    // Validate event content size
    if contents.len() > MAX_EVENT_CONTENT_SIZE {
        return Err(ERR_EVENT_CONTENT_TOO_LARGE);
    }

    Ok(())
}
