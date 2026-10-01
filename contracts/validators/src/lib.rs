//! Reusable, semantic input validators for the Callora contracts.
//!
//! This crate exists to replace generic panics and opaque `Result<_, ()>`
//! error values in validation code with a stable, machine-readable
//! [`ValidatorError`] enum. Callers can branch on the numeric error codes
//! instead of parsing panic strings, and every rejection carries a specific
//! reason (empty input, out-of-range amount, arithmetic overflow, and so on).
//!
//! # Core invariant
//! For every input string `s`:
//! - `is_visible_ascii_metadata(s) == normalize_visible_ascii(s).is_ok()`
//! - Acceptance iff `s` is non-empty, length ≤ [`MAX_VALIDATED_STRING_LEN`],
//!   every byte is in `0x20..=0x7E`, and neither the first nor last byte is
//!   ASCII space (`0x20`).

#![no_std]

mod errors;
mod validators;
mod views;

pub use errors::ValidatorError;
pub use validators::{
    checked_add_amount, is_valid_offering_id, is_valid_offering_id_byte, is_visible_ascii_metadata,
    normalize_offering_id, normalize_visible_ascii, require_in_range, require_non_negative_amount,
    require_positive_amount, MAX_OFFERING_ID_LEN, MAX_VALIDATED_STRING_LEN,
};
pub use views::{
    capabilities, ALL_CAPABILITIES, CAP_AMOUNT_VALIDATION, CAP_CHECKED_ARITHMETIC,
    CAP_RANGE_VALIDATION, CAP_STRING_VALIDATION,
};
