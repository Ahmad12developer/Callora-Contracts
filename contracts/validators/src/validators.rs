//! Pure, stateless validators that return semantic [`ValidatorError`] values.
//!
//! These helpers centralize the input-validation policy shared across Callora
//! contracts. Every rejection returns a specific [`ValidatorError`] variant
//! instead of a generic panic or opaque `()` error, so callers can react to the
//! precise failure reason. All arithmetic is overflow-safe and no production
//! path uses `unwrap()`/`expect()`.

use crate::errors::ValidatorError;
use soroban_sdk::String;

/// Maximum byte length accepted by [`normalize_visible_ascii`].
pub const MAX_VALIDATED_STRING_LEN: u32 = 256;

/// Normalize a bounded metadata string to its canonical on-chain form.
///
/// Accepted strings are non-empty visible ASCII (bytes `0x20..=0x7e`) with no
/// leading or trailing spaces. This rejects C0/DEL controls, zero-width and
/// bidi controls, and Unicode confusables by construction. Because ASCII has no
/// decomposed forms, the returned buffer is NFC-normalized and byte-stable.
///
/// # Errors
///
/// - [`ValidatorError::Empty`] if the string has zero length.
/// - [`ValidatorError::TooLong`] if the string exceeds
///   [`MAX_VALIDATED_STRING_LEN`] bytes.
/// - [`ValidatorError::LeadingWhitespace`] if the first byte is a space.
/// - [`ValidatorError::TrailingWhitespace`] if the last byte is a space.
/// - [`ValidatorError::NonVisibleAscii`] if any byte is outside the visible
///   ASCII range.
///
/// On success the full fixed-size buffer is returned; only the first `s.len()`
/// bytes are meaningful.
pub fn normalize_visible_ascii(
    s: &String,
) -> Result<[u8; MAX_VALIDATED_STRING_LEN as usize], ValidatorError> {
    let len = s.len();
    if len == 0 {
        return Err(ValidatorError::Empty);
    }
    if len > MAX_VALIDATED_STRING_LEN {
        return Err(ValidatorError::TooLong);
    }

    let mut buf = [0u8; MAX_VALIDATED_STRING_LEN as usize];
    s.copy_into_slice(&mut buf[..len as usize]);
    let bytes = &buf[..len as usize];

    if bytes[0] == b' ' {
        return Err(ValidatorError::LeadingWhitespace);
    }
    if bytes[len as usize - 1] == b' ' {
        return Err(ValidatorError::TrailingWhitespace);
    }

    for &b in bytes {
        if !(0x20..=0x7e).contains(&b) {
            return Err(ValidatorError::NonVisibleAscii);
        }
    }

    Ok(buf)
}

/// Return whether a bounded metadata string is accepted by the on-chain policy.
///
/// This is a convenience wrapper over [`normalize_visible_ascii`] for call
/// sites that only need a boolean verdict and not the normalized buffer.
pub fn is_visible_ascii_metadata(s: &String) -> bool {
    normalize_visible_ascii(s).is_ok()
}

/// Maximum byte length accepted for an offering id.
pub const MAX_OFFERING_ID_LEN: u32 = 64;

/// Return whether a byte is an allowed offering-id character.
///
/// Offering ids are restricted to lowercase ASCII letters, digits, and the
/// `_`/`-` separators. This excludes spaces, C0/DEL control bytes, bidi and
/// zero-width controls, and Unicode confusables by construction.
pub fn is_valid_offering_id_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
}

/// Normalize and validate an offering id.
///
/// Offering ids are used as keys by settlement pricing and off-chain routing.
/// To prevent visually confusable ids from spoofing another developer's offering,
/// ids are restricted to non-empty lowercase ASCII letters, digits, and the
/// `_`/`-` separators, with no leading or trailing spaces.
///
/// # Errors
///
/// Always returns [`ValidatorError::InvalidOfferingId`] on rejection so callers
/// can distinguish id failures from metadata failures. Rejections include:
///
/// - empty ids,
/// - ids longer than [`MAX_OFFERING_ID_LEN`] bytes,
/// - ids having leading or trailing spaces,
/// - ids having any byte outside `[a-z0-9_-]`.
///
/// On success the full fixed-size buffer is returned; only the first `s.len()`
/// bytes are meaningful.
pub fn normalize_offering_id(
    s: &String,
) -> Result<[u8; MAX_OFFERING_ID_LEN as usize], ValidatorError> {
    let len = s.len();
    if len == 0 || len > MAX_OFFERING_ID_LEN {
        return Err(ValidatorError::InvalidOfferingId);
    }

    let mut buf = [0u8; MAX_OFFERING_ID_LEN as usize];
    s.copy_into_slice(&mut buf[..len as usize]);
    let bytes = &buf[..len as usize];

    if bytes[0] == b' ' || bytes[len as usize - 1] == b' ' {
        return Err(ValidatorError::InvalidOfferingId);
    }

    for &b in bytes {
        if !is_valid_offering_id_byte(b) {
            return Err(ValidatorError::InvalidOfferingId);
        }
    }

    Ok(buf)
}

/// Return whether an offering id is accepted by the on-chain policy.
///
/// This is a convenience wrapper over [`normalize_offering_id`] for call
/// sites that only need a boolean verdict and not the normalized buffer.
pub fn is_valid_offering_id(s: &String) -> bool {
    normalize_offering_id(s).is_ok()
}

/// Require that `amount` is strictly greater than zero.
///
/// # Errors
///
/// Returns [`ValidatorError::AmountNotPositive`] when `amount <= 0`.
pub fn require_positive_amount(amount: i128) -> Result<i128, ValidatorError> {
    if amount <= 0 {
        return Err(ValidatorError::AmountNotPositive);
    }
    Ok(amount)
}

/// Require that `amount` is non-negative (greater than or equal to zero).
///
/// # Errors
///
/// Returns [`ValidatorError::AmountNegative`] when `amount < 0`.
pub fn require_non_negative_amount(amount: i128) -> Result<i128, ValidatorError> {
    if amount < 0 {
        return Err(ValidatorError::AmountNegative);
    }
    Ok(amount)
}

/// Add two amounts using overflow-safe checked arithmetic.
///
/// # Errors
///
/// Returns [`ValidatorError::Overflow`] if the addition would overflow `i128`.
pub fn checked_add_amount(a: i128, b: i128) -> Result<i128, ValidatorError> {
    a.checked_add(b).ok_or(ValidatorError::Overflow)
}

/// Require that `value` lies within the inclusive range `[min, max]`.
///
/// # Errors
///
/// Returns [`ValidatorError::OutOfRange`] when `value < min` or `value > max`.
/// If `min > max` the range is empty and every value is rejected.
pub fn require_in_range(value: i128, min: i128, max: i128) -> Result<i128, ValidatorError> {
    if value < min || value > max {
        return Err(ValidatorError::OutOfRange);
    }
    Ok(value)
}
