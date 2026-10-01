extern crate std;

use crate::SettlementError;
use std::collections::BTreeSet;

#[test]
fn settlement_error_codes_are_stable_and_unique() {
    let mappings = [
        (1_u32, SettlementError::NotInitialized),
        (2, SettlementError::AlreadyInitialized),
        (3, SettlementError::Unauthorized),
        (4, SettlementError::AmountNotPositive),
        (5, SettlementError::DeveloperRequired),
        (6, SettlementError::DeveloperMustBeNone),
        (7, SettlementError::PoolOverflow),
        (8, SettlementError::DeveloperOverflow),
        (9, SettlementError::UsdcTokenNotConfigured),
        (10, SettlementError::InsufficientDeveloperBalance),
        (11, SettlementError::DeveloperBalanceUnderflow),
        (12, SettlementError::InsufficientContractBalance),
        (13, SettlementError::DailyWithdrawCapExceeded),
        (14, SettlementError::GasExhaustionRisk),
        (15, SettlementError::ReasonTooLong),
        (16, SettlementError::MigrationSameAddress),
        (17, SettlementError::InvalidMigrationTarget),
        (18, SettlementError::NoDeveloperBalance),
        (19, SettlementError::TimelockOverflow),
        (20, SettlementError::MigrationNotFound),
        (21, SettlementError::TimelockNotExpired),
        (22, SettlementError::MigrationBalanceChanged),
        (23, SettlementError::OverDraft),
        (24, SettlementError::InvalidClaimWindow),
        (25, SettlementError::ClaimWindowClosed),
        (26, SettlementError::MinBalanceViolation),
        (27, SettlementError::ReplayDetected),
        (28, SettlementError::BatchEmpty),
        (29, SettlementError::BatchTooLarge),
        (30, SettlementError::DeveloperFrozen),
        (31, SettlementError::DeveloperNotFrozen),
        (32, SettlementError::FreezeUnauthorized),
        (33, SettlementError::WriteRateLimitExceeded),
        (34, SettlementError::InvalidConfigDistinct),
        (35, SettlementError::InvalidConfigAdminContract),
        (36, SettlementError::InvalidConfigVaultContract),
        (37, SettlementError::InvalidUsdcToken),
        (38, SettlementError::InvalidRecipient),
        (39, SettlementError::NoAdminTransferPending),
        (40, SettlementError::InvalidVault),
        (41, SettlementError::NoVaultRotationPending),
        (42, SettlementError::BroadcastMessageTooLong),
        (43, SettlementError::CrossTenantBatch),
        (44, SettlementError::NoUpgradePending),
        (45, SettlementError::ZeroWasmHash),
        (46, SettlementError::UpgradeTimelockNotExpired),
        (47, SettlementError::UnsupportedToken),
        (48, SettlementError::DuplicateRequestId),
        (49, SettlementError::LengthMismatch),
        (50, SettlementError::InvalidCursor),
    ];

    let mut seen = BTreeSet::new();
    for (expected_code, variant) in mappings {
        assert_eq!(variant as u32, expected_code);
        assert!(
            seen.insert(expected_code),
            "duplicate settlement error code {expected_code}"
        );
    }

    assert_eq!(seen.len(), 50);
}

#[test]
fn error_code_docs_list_every_settlement_code() {
    let docs = include_str!("../../../docs/ERROR_CODES.md");
    let expected_lines = [
        "| 1 | `NotInitialized` | Settlement | A function was called before `init` |",
        "| 2 | `AlreadyInitialized` | Settlement | `init` was called more than once |",
        "| 3 | `Unauthorized` | Settlement | Caller is not the vault or current admin |",
        "| 4 | `AmountNotPositive` | Settlement | Amount must be greater than zero |",
        "| 5 | `DeveloperRequired` | Settlement | `to_pool=false` requires a developer address |",
        "| 6 | `DeveloperMustBeNone` | Settlement | `to_pool=true` forbids a developer address |",
        "| 7 | `PoolOverflow` | Settlement | Global pool credit would overflow `i128` |",
        "| 8 | `DeveloperOverflow` | Settlement | Developer balance credit would overflow `i128` |",
        "| 9 | `UsdcTokenNotConfigured` | Settlement | USDC token address is not configured |",
        "| 10 | `InsufficientDeveloperBalance` | Settlement | Developer balance is lower than the withdrawal |",
        "| 11 | `DeveloperBalanceUnderflow` | Settlement | Developer balance debit would underflow |",
        "| 12 | `InsufficientContractBalance` | Settlement | Contract USDC balance is lower than requested amount |",
        "| 13 | `DailyWithdrawCapExceeded` | Settlement | Daily developer withdrawal cap would be exceeded |",
        "| 14 | `GasExhaustionRisk` | Settlement | Full scan is too large; use paginated access |",
        "| 15 | `ReasonTooLong` | Settlement | Reason `Symbol` exceeds the allowed length |",
        "| 16 | `MigrationSameAddress` | Settlement | Migration source and target are identical |",
        "| 17 | `InvalidMigrationTarget` | Settlement | Migration target is the settlement contract |",
        "| 18 | `NoDeveloperBalance` | Settlement | Migration source has no positive balance |",
        "| 19 | `TimelockOverflow` | Settlement | Timelock timestamp addition overflowed |",
        "| 20 | `MigrationNotFound` | Settlement | No migration is pending for the source |",
        "| 21 | `TimelockNotExpired` | Settlement | Migration delay has not elapsed |",
        "| 22 | `MigrationBalanceChanged` | Settlement | Approved amount is no longer available |",
        "| 23 | `OverDraft` | Settlement | Withdrawal amount exceeds the developer's balance |",
        "| 24 | `InvalidClaimWindow` | Settlement | Claim window parameters are invalid |",
        "| 25 | `ClaimWindowClosed` | Settlement | Developer claim window is not currently open |",
        "| 26 | `MinBalanceViolation` | Settlement | Withdrawal would leave balance below the minimum |",
        "| 27 | `ReplayDetected` | Settlement | Settlement request reused or regressed the replay-guard ledger sequence |",
        "| 28 | `BatchEmpty` | Settlement | Batch operation received an empty vector |",
        "| 29 | `BatchTooLarge` | Settlement | Batch operation exceeded the maximum allowed size |",
        "| 30 | `DeveloperFrozen` | Settlement | Developer is frozen and cannot withdraw |",
        "| 31 | `DeveloperNotFrozen` | Settlement | Developer is not frozen; cannot unfreeze |",
        "| 32 | `FreezeUnauthorized` | Settlement | Caller is not authorized to freeze/unfreeze |",
        "| 33 | `WriteRateLimitExceeded` | Settlement | Admin wrote prices too frequently |",
        "| 34 | `InvalidConfigDistinct` | Settlement | Init config requires distinct admin and vault |",
        "| 35 | `InvalidConfigAdminContract` | Settlement | Init config forbids the admin being the contract |",
        "| 36 | `InvalidConfigVaultContract` | Settlement | Init config forbids the vault being the contract |",
        "| 37 | `InvalidUsdcToken` | Settlement | USDC token address is invalid |",
        "| 38 | `InvalidRecipient` | Settlement | Withdrawal recipient cannot be the contract |",
        "| 39 | `NoAdminTransferPending` | Settlement | No admin transfer is pending |",
        "| 40 | `InvalidVault` | Settlement | Vault address is invalid |",
        "| 41 | `NoVaultRotationPending` | Settlement | No vault rotation is pending |",
        "| 42 | `BroadcastMessageTooLong` | Settlement | Admin broadcast message exceeds the maximum length |",
        "| 43 | `CrossTenantBatch` | Settlement | Batch settlement mixed multiple developers |",
        "| 47 | `UnsupportedToken` | Settlement | Token is not enabled for settlement payments |",
        "| 48 | `DuplicateRequestId` | Settlement | Deduction request ID has already been recorded |",
        "| 49 | `LengthMismatch` | Settlement | Paired batch vectors have different lengths |",
        "| 50 | `InvalidCursor` | Settlement | Batch cursor is past the end or the limit is zero |",
    ];

    for line in expected_lines {
        assert!(docs.contains(line), "missing settlement docs line: {line}");
    }
}
