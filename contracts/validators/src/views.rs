use soroban_sdk::Env;

/// Bit 0 — String validation: clients can rely on the visible-ASCII metadata
/// validators exposed by [`crate::normalize_visible_ascii`] and
/// [`crate::is_visible_ascii_metadata`].
pub const CAP_STRING_VALIDATION: u64 = 1 << 0;

/// Bit 1 — Amount validation: clients can use the positive/non-negative amount
/// preconditions exposed by [`crate::require_positive_amount`] and
/// [`crate::require_non_negative_amount`].
pub const CAP_AMOUNT_VALIDATION: u64 = 1 << 1;

/// Bit 2 — Checked arithmetic: clients can use the overflow-safe checked
/// addition helper [`crate::checked_add_amount`].
pub const CAP_CHECKED_ARITHMETIC: u64 = 1 << 2;

/// Bit 3 — Range validation: clients can use the inclusive range checks from
/// [`crate::require_in_range`].
pub const CAP_RANGE_VALIDATION: u64 = 1 << 3;

/// Bits 4–63 are reserved for future validator capabilities and remain clear.
pub const ALL_CAPABILITIES: u64 =
    CAP_STRING_VALIDATION | CAP_AMOUNT_VALIDATION | CAP_CHECKED_ARITHMETIC | CAP_RANGE_VALIDATION;

/// Return the validator capability bitmap.
///
/// This is a pure read-only view and does not inspect or mutate storage. The
/// returned bitmask is stable for the current validator feature set so clients
/// can detect capability deltas across upgrades.
pub fn capabilities(_env: &Env) -> u64 {
    ALL_CAPABILITIES
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn capabilities_equals_all_mask() {
        let env = Env::default();
        assert_eq!(capabilities(&env), ALL_CAPABILITIES);
    }

    #[test]
    fn reserved_bits_are_clear() {
        let env = Env::default();
        let caps = capabilities(&env);
        assert_eq!(caps & !((1u64 << 4) - 1), 0);
    }
}
