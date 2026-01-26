//! Error handling system with compact error codes and compile-time registration.
//!
//! ## Error ID Conventions
//!
//! Each error has a 16-bit ID composed of:
//! - Upper 8 bits: Crate namespace (auto-generated from crate name)
//! - Lower 8 bits: Local error ID (0x00-0xFF)
//!
//! ### Recommended Local ID Ranges:
//! - 0x00-0x3F: Validation errors (invalid input, missing data)
//! - 0x40-0x7F: System errors (internal failures, resource limits)
//! - 0x80-0xBF: Business logic errors (unauthorized, insufficient funds)
//! - 0xC0-0xFF: Reserved for future use

use core::fmt;

#[cfg(feature = "error-decode")]
extern crate alloc;
#[cfg(feature = "error-decode")]
use linkme::distributed_slice;

/* ───────────────────────────── Runtime handle ──────────────────────────── */

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode {
    pub id: u16,
    pub arg: u16,
}

impl ErrorCode {
    pub const fn new(id: u16) -> Self {
        Self { id, arg: 0 }
    }
    pub const fn new_with_arg(id: u16, arg: u16) -> Self {
        Self { id, arg }
    }
    pub const fn code(self) -> u64 {
        self.id as u64
    }
    pub const fn arg(self) -> u16 {
        self.arg
    }
}

/* ───────────────────────── Templates (host only) ───────────────────────── */

#[cfg(feature = "error-decode")]
#[derive(Debug)]
pub struct ErrorTemplate {
    pub id: u16,
    pub flags: u8,
    pub text: &'static str,
}

#[cfg(feature = "error-decode")]
#[distributed_slice]
pub static ERROR_TEMPLATES: [ErrorTemplate] = [..];

/* ───────────────────────── Local decoder (host) ────────────────────────── */

#[cfg(feature = "error-decode")]
pub type DecodedError = ErrorCode;

#[cfg(feature = "error-decode")]
fn decode_error(handle: ErrorCode) -> String {
    let tmpl = ERROR_TEMPLATES
        .iter()
        .find(|t| t.id == handle.id)
        .map(|t| t.text)
        .unwrap_or("<unknown error>");

    if let Some(pos) = tmpl.find("{arg}") {
        // Limit arg string length to prevent excessive memory allocation
        const MAX_ARG_DISPLAY_LEN: usize = 32;
        let arg_str = handle.arg.to_string();
        let arg_display = if arg_str.len() > MAX_ARG_DISPLAY_LEN {
            format!("{}...", &arg_str[..MAX_ARG_DISPLAY_LEN])
        } else {
            arg_str
        };

        let mut out = String::with_capacity(tmpl.len() + arg_display.len());
        out.push_str(&tmpl[..pos]);
        out.push_str(&arg_display);
        out.push_str(&tmpl[pos + 5..]);
        out
    } else {
        tmpl.into()
    }
}

/* ───────────────────── Display / Debug implementations ─────────────────── */

#[cfg(feature = "error-decode")]
impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&decode_error(*self))
    }
}

#[cfg(feature = "error-decode")]
impl fmt::Debug for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(not(feature = "error-decode"))]
impl fmt::Debug for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ErrorCode {{ id: {}, arg: {} }}", self.id, self.arg)
    }
}

/* ───────────────────── define_error! helper macro ──────────────────────── */

#[macro_export]
macro_rules! define_error {
    ($ident:ident, $local:expr, $text:expr) => {
        /* ---------- public constant (runtime) ---------------------------- */
        #[allow(dead_code)]
        pub const $ident: $crate::error::ErrorCode = {
            const LOCAL_ID: u16 = $local;
            // Compile-time assertion to ensure local ID fits in u8
            const _: () = assert!(
                LOCAL_ID <= 0xFF,
                "Local error ID must be <= 0xFF to fit in u8"
            );
            $crate::error::ErrorCode::new(
                (($crate::error::crate_namespace() as u16) << 8) | (LOCAL_ID & 0xFF),
            )
        };

        /* ---------- host template registration --------------------------- */
        #[cfg(feature = "error-decode")]
        const _: () = {
            use $crate::error::{ErrorTemplate, ERROR_TEMPLATES};

            #[linkme::distributed_slice(ERROR_TEMPLATES)]
            static ENTRY: ErrorTemplate = ErrorTemplate {
                id: $ident.id,
                flags: 0,
                text: $text,
            };
        };
    };
}

/* ────────────────── Per-crate namespace (compile-time CRC-8) ───────────── */

pub const fn crate_namespace() -> u8 {
    // Use FNV-1a hash algorithm for better distribution
    const fn fnv1a_hash(bytes: &[u8]) -> u8 {
        const FNV_PRIME: u64 = 0x100000001b3;
        const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;

        let mut hash = FNV_OFFSET_BASIS;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            i += 1;
        }
        // Take the lower 8 bits for namespace
        (hash & 0xFF) as u8
    }

    // Use CARGO_PKG_NAME for more unique identification
    fnv1a_hash(env!("CARGO_PKG_NAME").as_bytes())
}

/* ───────────────────────────── Unit tests ──────────────────────────────── */

#[cfg(all(test, feature = "error-decode"))]
#[allow(clippy::disallowed_types)]
mod test_duplicate_detection {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_error_ids() {
        let mut seen_ids = HashSet::new();
        let mut duplicates = Vec::new();

        for template in ERROR_TEMPLATES.iter() {
            if !seen_ids.insert(template.id) {
                duplicates.push((template.id, template.text));
            }
        }

        assert!(
            duplicates.is_empty(),
            "Found duplicate error IDs: {duplicates:?}"
        );
    }
}

#[cfg(test)]
mod tests_handle {
    use super::*;
    use core::mem;
    #[test]
    fn constructors() {
        let h = ErrorCode::new(0xABCD);
        assert_eq!(h.id, 0xABCD);
        assert_eq!(h.arg, 0);
        let h2 = ErrorCode::new_with_arg(0xABCD, 7);
        assert_eq!(h2.arg(), 7);
    }
    #[test]
    fn size_is_four_bytes() {
        assert_eq!(mem::size_of::<ErrorCode>(), 4);
    }
}

#[cfg(test)]
mod tests_macro {
    use super::*;
    define_error!(ERR_FOO, 0xF0, "foo {arg}");

    #[test]
    fn id_has_namespace() {
        let expected = ((crate_namespace() as u16) << 8) | 0xF0;
        assert_eq!(ERR_FOO.id, expected);
    }

    #[test]
    fn new_with_arg_retains_id() {
        let e = ErrorCode::new_with_arg(ERR_FOO.id, 42);
        assert_eq!(e.id, ERR_FOO.id);
        assert_eq!(e.arg, 42);
    }

    #[cfg(feature = "error-decode")]
    mod decode {
        use super::*;
        #[test]
        fn template_registered() {
            assert!(ERROR_TEMPLATES.iter().any(|t| t.id == ERR_FOO.id));
        }
        #[test]
        fn display_renders() {
            let e = ErrorCode::new_with_arg(ERR_FOO.id, 17);
            assert_eq!(format!("{e}"), "foo 17");
        }
    }
}
