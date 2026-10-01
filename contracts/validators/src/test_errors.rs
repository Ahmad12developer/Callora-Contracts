extern crate std;

use crate::ValidatorError;
use std::collections::BTreeSet;

/// The numeric discriminants are part of the public interface. This test pins
/// each variant to its code and asserts uniqueness so that accidental
/// renumbering (which would silently break on-chain callers) fails CI.
#[test]
fn validator_error_codes_are_stable_and_unique() {
    let mappings = [
        (1_u32, ValidatorError::Empty),
        (2, ValidatorError::TooLong),
        (3, ValidatorError::LeadingWhitespace),
        (4, ValidatorError::TrailingWhitespace),
        (5, ValidatorError::NonVisibleAscii),
        (6, ValidatorError::AmountNotPositive),
        (7, ValidatorError::AmountNegative),
        (8, ValidatorError::Overflow),
        (9, ValidatorError::OutOfRange),
        (10, ValidatorError::InvalidOfferingId),
    ];

    let mut seen = BTreeSet::new();
    for (expected_code, variant) in mappings {
        assert_eq!(variant as u32, expected_code);
        assert!(
            seen.insert(expected_code),
            "duplicate validator error code {expected_code}"
        );
    }

    assert_eq!(seen.len(), 10);
}

#[test]
fn validator_error_is_copy_and_comparable() {
    let a = ValidatorError::Empty;
    let b = a; // Copy
    assert_eq!(a, b);
    assert_ne!(ValidatorError::Empty, ValidatorError::TooLong);
    assert!(ValidatorError::Empty < ValidatorError::TooLong);
}
