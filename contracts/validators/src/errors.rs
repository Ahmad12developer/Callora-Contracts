use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for Callora input validators.
///
/// These variants replace generic panics and opaque `Result<_, ()>` values in
/// validation code with semantic, self-describing errors. The numeric
/// discriminants below are part of the public interface and **must remain
/// stable over time**: callers may branch on these `u32` codes instead of
/// parsing panic strings, and the code-stability test in
/// `test_errors.rs` guards against accidental renumbering.
///
/// | Code | Variant             | Meaning                                                             |
/// |------|---------------------|---------------------------------------------------------------------|
/// | 1    | Empty               | Input string is empty                                               |
/// | 2    | TooLong             | Input exceeds the maximum allowed length                            |
/// | 3    | LeadingWhitespace   | Input has leading whitespace                                        |
/// | 4    | TrailingWhitespace  | Input has trailing whitespace                                       |
/// | 5    | NonVisibleAscii     | Input contains non-visible or non-ASCII bytes                       |
/// | 6    | AmountNotPositive   | Numeric amount must be greater than zero                            |
/// | 7    | AmountNegative      | Numeric amount must be non-negative                                 |
/// | 8    | Overflow            | Arithmetic overflow was detected                                    |
/// | 9    | OutOfRange          | Value falls outside the allowed inclusive `[min, max]` range        |
/// | 10   | InvalidOfferingId   | Offering id is empty, too long, or uses bytes outside `[a-z0-9_-]`  |
#[contracterror]
#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ValidatorError {
    /// Input string is empty (code 1).
    Empty = 1,
    /// Input exceeds the maximum allowed length (code 2).
    TooLong = 2,
    /// Input has leading whitespace (code 3).
    LeadingWhitespace = 3,
    /// Input has trailing whitespace (code 4).
    TrailingWhitespace = 4,
    /// Input contains non-visible or non-ASCII bytes such as C0/DEL controls,
    /// zero-width characters, bidi overrides, or Unicode confusables (code 5).
    NonVisibleAscii = 5,
    /// Numeric amount must be strictly greater than zero (code 6).
    AmountNotPositive = 6,
    /// Numeric amount must be non-negative, i.e. greater than or equal to zero
    /// (code 7).
    AmountNegative = 7,
    /// Arithmetic overflow was detected while combining amounts (code 8).
    Overflow = 8,
    /// Value falls outside the allowed inclusive `[min, max]` range (code 9).
    OutOfRange = 9,
    /// Offering id is empty, longer than `MAX_OFFERING_ID_LEN`, has leading or
    /// trailing spaces, or contains a byte outside `[a-z0-9_-]` (code 10).
    InvalidOfferingId = 10,
}
