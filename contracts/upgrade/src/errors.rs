use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the Callora Upgrade contract.
///
/// The numeric discriminants in this enum are part of the contract interface and
/// must remain stable over time. Callers and indexers may branch on these `u32`
/// codes instead of parsing panic strings.
///
/// | Code | Variant                  | Meaning                                               |
/// |------|--------------------------|-------------------------------------------------------|
/// | 1    | NotInitialized           | Contract has not been initialized yet                 |
/// | 2    | AlreadyInitialized       | `init` was called more than once                      |
/// | 3    | Unauthorized             | Caller is not authorized for the operation            |
/// | 4    | InvalidWasmHash          | Provided WASM hash is zero or invalid                 |
/// | 5    | UpgradeNotAllowed        | Upgrade operation is currently disabled               |
/// | 6    | MigrationPending         | A migration or upgrade is already pending             |
/// | 7    | TimelockNotExpired       | Required timelock delay has not elapsed               |
/// | 8    | SameWasmHash             | New WASM hash is identical to current WASM hash       |
/// | 9    | SameVersion              | Proposed version matches current version              |
/// | 10   | InvalidVersion           | Proposed version number is invalid or non-increasing  |
/// | 11   | Overflow                 | Arithmetic calculation overflowed                     |
/// | 12   | AlreadyUpgraded          | Contract has already been upgraded to this state      |
/// | 13   | StaleNonce               | Transaction nonce is stale or invalid                 |
/// | 14   | MigrationSameAddress     | Target migration contract address matches source      |
/// | 15   | InvalidMigrationTarget   | Target migration contract address is invalid          |
/// | 16   | NoUpgradePending         | No pending upgrade was found to execute or cancel     |
/// | 17   | CooldownNotElapsed       | The cooldown period for upgrades has not yet elapsed  |
/// | 18   | InvalidCooldown          | Requested cooldown is outside the allowed bounds      |
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum UpgradeError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// Contract has already been initialized (code 2).
    AlreadyInitialized = 2,
    /// Caller is not authorized for this operation (code 3).
    Unauthorized = 3,
    /// Provided WASM hash is zero or invalid (code 4).
    InvalidWasmHash = 4,
    /// Upgrade operation is currently disabled (code 5).
    UpgradeNotAllowed = 5,
    /// A migration or upgrade is already pending (code 6).
    MigrationPending = 6,
    /// Required timelock delay has not elapsed (code 7).
    TimelockNotExpired = 7,
    /// New WASM hash is identical to current WASM hash (code 8).
    SameWasmHash = 8,
    /// Proposed version matches current version (code 9).
    SameVersion = 9,
    /// Proposed version number is invalid or non-increasing (code 10).
    InvalidVersion = 10,
    /// Arithmetic calculation overflowed (code 11).
    Overflow = 11,
    /// Contract has already been upgraded to this state (code 12).
    AlreadyUpgraded = 12,
    /// Transaction nonce is stale or invalid (code 13).
    StaleNonce = 13,
    /// Target migration contract address matches source (code 14).
    MigrationSameAddress = 14,
    /// Target migration contract address is invalid (code 15).
    InvalidMigrationTarget = 15,
    /// No pending upgrade was found to execute or cancel (code 16).
    NoUpgradePending = 16,
    /// The cooldown period for upgrades has not yet elapsed (code 17).
    CooldownNotElapsed = 17,
    /// The requested cooldown is outside
    /// `MIN_COOLDOWN_SECONDS..=MAX_COOLDOWN_SECONDS` (code 18).
    InvalidCooldown = 18,
}

/// Type alias for client-facing ContractError compatibility.
pub type ContractError = UpgradeError;
