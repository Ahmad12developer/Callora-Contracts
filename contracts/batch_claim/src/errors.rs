use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the Callora Batch-Claim contract.
///
/// | Code | Variant              | Meaning                                           |
/// |------|----------------------|---------------------------------------------------|
/// | 1    | NotInitialized       | Contract has not been initialized yet             |
/// | 2    | AlreadyInitialized   | `init` was called more than once                  |
/// | 3    | Unauthorized         | Caller is not the admin                           |
/// | 4    | ClaimNotFound        | No claim record exists for the given claimant     |
/// | 5    | AlreadySettled       | Claim has already been collected                  |
/// | 6    | InvalidAmount        | Claim amount must be positive                     |
/// | 7    | Overflow             | Arithmetic overflow in pending-amount accumulation|
/// | 8    | ClaimIdAlreadyUsed   | Claim id already consumed or cancelled (tombstoned)|
/// | 9    | ClaimIdMismatch      | Provided claim_id does not match stored record    |
/// | 10   | InvalidClaimId       | Identifier is malformed (all-zero)                |
/// | 11   | BatchTooLarge        | Batch exceeds `MAX_PENDING_AMOUNTS` entries       |
/// | 12   | BatchEmpty           | Batch input must not be empty                     |
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BatchClaimError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// Contract has already been initialized (code 2).
    AlreadyInitialized = 2,
    /// Caller is not the admin (code 3).
    Unauthorized = 3,
    /// No claim record exists for the given claimant (code 4).
    ClaimNotFound = 4,
    /// Claim has already been collected (code 5).
    AlreadySettled = 5,
    /// Claim amount must be > 0 (code 6).
    InvalidAmount = 6,
    /// Arithmetic overflow in pending-amount accumulation (code 7).
    Overflow = 7,
    /// The claim identifier has already been consumed or cancelled; replay
    /// rejected (code 8).
    ///
    /// Cancelled claims leave a tombstone (`ClaimConsumed(id) = true`) so the
    /// id can never be reissued to the same or any other claimant.
    ClaimIdAlreadyUsed = 8,
    /// The provided claim_id does not match the stored record (code 9).
    ClaimIdMismatch = 9,
    /// The claim identifier is malformed (code 10).
    ///
    /// Issue #1044: the all-zero identifier is rejected because it is what an
    /// uninitialised buffer, a defaulted struct field, or a truncated encoding
    /// produces. Accepting it would let two unrelated bugs collide on the same
    /// "unique" id.
    InvalidClaimId = 10,
    /// The batch exceeds [`crate::MAX_PENDING_AMOUNTS`] entries (code 11).
    ///
    /// Issue #1044: bounds the work a single `batch_claim` invocation can do,
    /// so an oversized batch fails closed and cheaply instead of running the
    /// ledger out of resources part-way through a settlement.
    BatchTooLarge = 11,
    /// The batch input is empty (code 12).
    ///
    /// Issue #1044: an empty batch is rejected rather than silently succeeding,
    /// so client bugs surface as errors instead of wasting fees on a no-op
    /// settlement.
    BatchEmpty = 12,
}
