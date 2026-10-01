use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the settlement contract.
///
/// The numeric discriminants in this enum are part of the contract interface and
/// must remain stable over time. Callers and indexers may branch on these `u32`
/// codes instead of parsing panic strings.
///
/// | Code | Variant                      | Meaning                                              |
/// |------|------------------------------|------------------------------------------------------|
/// | 1    | NotInitialized               | A function was called before `init`                  |
/// | 2    | AlreadyInitialized           | `init` was called more than once                     |
/// | 3    | Unauthorized                 | Caller is not the vault or current admin             |
/// | 4    | AmountNotPositive            | Amount must be greater than zero                     |
/// | 5    | DeveloperRequired            | `to_pool=false` requires a developer address         |
/// | 6    | DeveloperMustBeNone          | `to_pool=true` forbids a developer address           |
/// | 7    | PoolOverflow                 | Global pool credit would overflow `i128`             |
/// | 8    | DeveloperOverflow            | Developer balance credit would overflow `i128`       |
/// | 9    | UsdcTokenNotConfigured       | USDC token address is not configured                 |
/// | 10   | InsufficientDeveloperBalance | Developer balance is lower than the withdrawal       |
/// | 11   | DeveloperBalanceUnderflow    | Developer balance debit would underflow              |
/// | 12   | InsufficientContractBalance  | Contract USDC balance is lower than requested amount |
/// | 13   | DailyWithdrawCapExceeded     | Daily developer withdrawal cap would be exceeded     |
/// | 14   | GasExhaustionRisk            | Full scan is too large; use paginated access         |
/// | 15   | ReasonTooLong                | Reason `Symbol` exceeds the allowed length           |
/// | 16   | MigrationSameAddress         | Migration source and target are identical            |
/// | 17   | InvalidMigrationTarget       | Migration target is the settlement contract          |
/// | 18   | NoDeveloperBalance           | Migration source has no positive balance             |
/// | 19   | TimelockOverflow             | Timelock timestamp addition overflowed                |
/// | 20   | MigrationNotFound            | No migration is pending for the source                |
/// | 21   | TimelockNotExpired           | Migration delay has not elapsed                       |
/// | 22   | MigrationBalanceChanged      | Approved amount is no longer available                |
/// | 23   | OverDraft                    | Withdrawal amount exceeds the developer's balance     |
/// | 24   | InvalidClaimWindow            | Claim window parameters are invalid                    |
/// | 25   | ClaimWindowClosed             | Developer claim window is not currently open           |
/// | 26   | MinBalanceViolation           | Withdrawal would leave balance below the minimum       |
/// | 27   | ReplayDetected                | Settlement claim was replayed or out of order        |
/// | 28   | BatchEmpty                    | Batch operation received an empty vector             |
/// | 29   | BatchTooLarge                 | Batch operation exceeded the maximum allowed size    |
/// | 30   | DeveloperFrozen              | Developer is frozen and cannot withdraw              |
/// | 31   | DeveloperNotFrozen            | Developer is not frozen; cannot unfreeze             |
/// | 32   | FreezeUnauthorized            | Caller is not authorized to freeze/unfreeze           |
/// | 33   | WriteRateLimitExceeded       | Admin wrote prices too frequently                    |
/// | 34   | InvalidConfigDistinct        | Init config requires distinct admin and vault        |
/// | 35   | InvalidConfigAdminContract   | Init config forbids the admin being the contract     |
/// | 36   | InvalidConfigVaultContract   | Init config forbids the vault being the contract     |
/// | 37   | InvalidUsdcToken             | USDC token address is invalid                        |
/// | 38   | InvalidRecipient             | Withdrawal recipient cannot be the contract          |
/// | 39   | NoAdminTransferPending       | No admin transfer is pending                         |
/// | 40   | InvalidVault                 | Vault address is invalid                             |
/// | 41   | NoVaultRotationPending       | No vault rotation is pending                         |
/// | 42   | BroadcastMessageTooLong      | Admin broadcast message exceeds the maximum length   |
/// | 43   | CrossTenantBatch             | Batch settlement mixes developers from different tenants |
/// | 44   | NoUpgradePending             | No upgrade proposal is currently pending             |
/// | 45   | ZeroWasmHash                 | Proposed WASM hash is all-zero (rejected)            |
/// | 46   | UpgradeTimelockNotExpired    | Upgrade timelock delay has not yet elapsed           |
/// | 47   | UnsupportedToken             | Token is not enabled for settlement payments         |
/// | 48   | DuplicateRequestId           | Deduction request ID has already been recorded       |
/// | 49   | LengthMismatch               | Paired batch vectors have different lengths          |
/// | 50   | InvalidCursor                | Batch cursor is past the end or the limit is zero    |
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum SettlementError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    AmountNotPositive = 4,
    DeveloperRequired = 5,
    DeveloperMustBeNone = 6,
    PoolOverflow = 7,
    DeveloperOverflow = 8,
    UsdcTokenNotConfigured = 9,
    InsufficientDeveloperBalance = 10,
    DeveloperBalanceUnderflow = 11,
    InsufficientContractBalance = 12,
    DailyWithdrawCapExceeded = 13,
    GasExhaustionRisk = 14,
    ReasonTooLong = 15,
    MigrationSameAddress = 16,
    InvalidMigrationTarget = 17,
    NoDeveloperBalance = 18,
    TimelockOverflow = 19,
    MigrationNotFound = 20,
    TimelockNotExpired = 21,
    MigrationBalanceChanged = 22,
    OverDraft = 23,
    InvalidClaimWindow = 24,
    ClaimWindowClosed = 25,
    MinBalanceViolation = 26,
    ReplayDetected = 27,
    BatchEmpty = 28,
    BatchTooLarge = 29,
    /// Developer is frozen and cannot withdraw.
    DeveloperFrozen = 30,
    /// Developer is not frozen; cannot unfreeze.
    DeveloperNotFrozen = 31,
    /// Caller is not authorized to freeze/unfreeze developers.
    FreezeUnauthorized = 32,
    /// Admin attempted a price write before the minimum interval elapsed.
    WriteRateLimitExceeded = 33,
    InvalidConfigDistinct = 34,
    InvalidConfigAdminContract = 35,
    InvalidConfigVaultContract = 36,
    InvalidUsdcToken = 37,
    InvalidRecipient = 38,
    NoAdminTransferPending = 39,
    InvalidVault = 40,
    NoVaultRotationPending = 41,
    /// Admin broadcast message exceeds the maximum allowed length.
    BroadcastMessageTooLong = 42,
    /// Batch settlement mixes developers from different tenants.
    CrossTenantBatch = 43,
    /// No upgrade proposal is currently pending.
    NoUpgradePending = 44,
    /// Proposed WASM hash is all-zero bytes (rejected as invalid).
    ZeroWasmHash = 45,
    /// Upgrade timelock delay has not yet elapsed.
    UpgradeTimelockNotExpired = 46,
    /// Token is not enabled for settlement payments.
    UnsupportedToken = 47,
    /// Deduction request ID has already been recorded.
    DuplicateRequestId = 48,
    /// #1135: `developers.len() != amounts.len()` in a paired batch.
    LengthMismatch = 49,
    /// #1135: `cursor > developers.len()` or `limit == 0`.
    InvalidCursor = 50,
}
