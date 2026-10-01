use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the Hot contract.
///
/// The numeric discriminants in this enum are part of the contract interface and
/// must remain stable over time. Callers and indexers may branch on these `u32`
/// codes instead of parsing panic strings.
///
/// | Code | Variant             | Meaning                                                      |
/// |------|---------------------|--------------------------------------------------------------|
/// | 1    | NotInitialized      | Contract has not been initialized yet                        |
/// | 2    | AlreadyInitialized  | `init` was called more than once                             |
/// | 3    | Unauthorized        | Caller is not authorized for this operation                  |
/// | 4    | CooldownActive      | A critical action is still inside its cool-off window        |
/// | 5    | InvalidCooldown     | Proposed cooldown value is outside the accepted range        |
/// | 6    | NoPendingAdmin      | No admin transfer is currently pending                       |
/// | 7    | Overflow            | Arithmetic overflow detected                                 |
/// | 8    | AlreadyPaused       | Contract is already paused; `pause` is a no-op               |
/// | 9    | NotPaused           | Contract is not paused; `unpause` is a no-op                 |
/// | 10   | SameSigner          | `rotate_signer` called with the current signer               |
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum HotError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// `init` was called more than once (code 2).
    AlreadyInitialized = 2,
    /// Caller is not authorized for this operation (code 3).
    Unauthorized = 3,
    /// A critical action is still inside its cool-off window (code 4).
    CooldownActive = 4,
    /// Proposed cooldown value is outside the accepted range (code 5).
    InvalidCooldown = 5,
    /// No admin transfer is currently pending (code 6).
    NoPendingAdmin = 6,
    /// Arithmetic overflow detected (code 7).
    Overflow = 7,
    /// Contract is already paused; `pause` is redundant (code 8).
    AlreadyPaused = 8,
    /// Contract is not currently paused; `unpause` is redundant (code 9).
    NotPaused = 9,
    /// `rotate_signer` was called with the current signer (code 10).
    SameSigner = 10,
}
