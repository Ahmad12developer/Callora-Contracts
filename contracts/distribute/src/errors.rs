use soroban_sdk::contracterror;

/// Typed, machine-readable error codes for the Callora Distribute contract.
///
/// Numeric discriminants are part of the contract interface and must remain
/// stable over time.  Callers and indexers may branch on these `u32` codes
/// instead of parsing panic strings.
///
/// | Code | Variant                  | Meaning                                            |
/// |------|--------------------------|----------------------------------------------------|
/// | 1    | NotInitialized           | A function was called before `init`                |
/// | 2    | AlreadyInitialized       | `init` was called more than once                   |
/// | 3    | Unauthorized             | Caller is not the admin or authorized caller       |
/// | 4    | Paused                   | Contract is currently paused                       |
/// | 5    | AccountLimitExceeded     | Reserved legacy code; not emitted by entrypoints   |
/// | 6    | AccountStateEmpty        | Reserved legacy code; not emitted by entrypoints   |
/// | 7    | BatchEmpty               | Batch operation received an empty items list       |
/// | 8    | BatchTooLarge            | Batch exceeds `MAX_BATCH_SIZE`                     |
/// | 9    | Overflow                 | Arithmetic overflow detected                       |
/// | 10   | CapNotPositive           | Global cap must be greater than zero                |
/// | 11   | NewAdminSameAsCurrent    | Nominated admin is the same as current admin       |
/// | 12   | NoAdminTransferPending   | No admin transfer is pending to cancel             |
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum DistributeError {
    /// A function was called before `init` (code 1).
    NotInitialized = 1,
    /// `init` was called more than once (code 2).
    AlreadyInitialized = 2,
    /// Caller is not the admin or an authorized caller (code 3).
    Unauthorized = 3,
    /// Contract is currently paused (code 4).
    Paused = 4,
    /// Reserved legacy state-counter error (code 5); retained for code stability.
    AccountLimitExceeded = 5,
    /// Reserved legacy state-counter error (code 6); retained for code stability.
    AccountStateEmpty = 6,
    /// Batch operation received an empty items list (code 7).
    BatchEmpty = 7,
    /// Batch exceeds `MAX_BATCH_SIZE` (code 8).
    BatchTooLarge = 8,
    /// Arithmetic overflow detected (code 9).
    Overflow = 9,
    /// Global cap must be greater than zero (code 10).
    CapNotPositive = 10,
    /// Nominated admin is the same as current admin (code 11).
    NewAdminSameAsCurrent = 11,
    /// No admin transfer is pending to cancel (code 12).
    NoAdminTransferPending = 12,

    InvalidConfig = 13,
    InvalidRecipient = 14,
    AmountNotPositive = 15,
    AmountExceedsMaxDistribute = 16,
    InsufficientBalance = 17,
    AlreadyPaused = 18,
    NotPaused = 19,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that every error discriminant is unique and sequential.
    #[test]
    fn error_codes_are_unique_and_sequential() {
        let codes: [u32; 19] = [
            DistributeError::NotInitialized as u32,
            DistributeError::AlreadyInitialized as u32,
            DistributeError::Unauthorized as u32,
            DistributeError::Paused as u32,
            DistributeError::AccountLimitExceeded as u32,
            DistributeError::AccountStateEmpty as u32,
            DistributeError::BatchEmpty as u32,
            DistributeError::BatchTooLarge as u32,
            DistributeError::Overflow as u32,
            DistributeError::CapNotPositive as u32,
            DistributeError::NewAdminSameAsCurrent as u32,
            DistributeError::NoAdminTransferPending as u32,
            DistributeError::InvalidConfig as u32,
            DistributeError::InvalidRecipient as u32,
            DistributeError::AmountNotPositive as u32,
            DistributeError::AmountExceedsMaxDistribute as u32,
            DistributeError::InsufficientBalance as u32,
            DistributeError::AlreadyPaused as u32,
            DistributeError::NotPaused as u32,
        ];
        let mut seen = [false; 20];
        for (i, &code) in codes.iter().enumerate() {
            assert_eq!(
                code,
                (i + 1) as u32,
                "code for variant {i} should be {}",
                i + 1
            );
            assert!(!seen[code as usize], "duplicate discriminant {code}");
            seen[code as usize] = true;
        }
    }
}
