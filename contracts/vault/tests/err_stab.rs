//! Frozen snapshot of `VaultError` discriminant codes (buffer #4).
//!
//! These tests guard against accidental renumbering of error codes, which would
//! silently break off-chain integrators that branch on numeric error codes
//! returned by the contract.

extern crate std;

use callora_vault::VaultError;
use std::collections::BTreeSet;

/// Frozen snapshot of the error code mapping.
///
/// Every variant is paired with its **stable numeric discriminant**.  If a
/// discriminant changes (accidentally or intentionally), this test fails
/// with a diff-friendly message identifying the regressed variant.
const FROZEN_ERROR_SNAPSHOT: [(u32, VaultError); 49] = [
    (1, VaultError::NotInitialized),
    (2, VaultError::AlreadyInitialized),
    (3, VaultError::Unauthorized),
    (4, VaultError::Paused),
    (5, VaultError::InsufficientBalance),
    (6, VaultError::AmountNotPositive),
    (7, VaultError::ExceedsMaxDeduct),
    (8, VaultError::BelowMinDeposit),
    (9, VaultError::Overflow),
    // code 10 reserved (was InitialBalanceNegative)
    (11, VaultError::MinDepositNotPositive),
    (12, VaultError::MaxDeductNotPositive),
    (13, VaultError::MinDepositExceedsMaxDeduct),
    (14, VaultError::UsdcTokenCannotBeVault),
    (15, VaultError::RevenuePoolCannotBeVault),
    (16, VaultError::AuthorizedCallerCannotBeVault),
    (17, VaultError::InitialBalanceExceedsOnLedger),
    (18, VaultError::AlreadyPaused),
    (19, VaultError::NotPaused),
    (20, VaultError::SettlementNotSet),
    (21, VaultError::BatchEmpty),
    (22, VaultError::BatchTooLarge),
    // codes 23-24 reserved (were NewOwnerSameAsCurrent, NoOwnershipTransferPending)
    (25, VaultError::NoAdminTransferPending),
    // codes 26-27 reserved (were OfferingIdTooLong, MetadataTooLong)
    (28, VaultError::PriceParseError),
    (29, VaultError::DuplicateRequestId),
    (30, VaultError::OfferingIdInvalid),
    (31, VaultError::MetadataInvalid),
    (32, VaultError::StaleNonce),
    (33, VaultError::NewRevenuePoolSameAsCurrent),
    (34, VaultError::NoRevenuePoolTransferPending),
    (35, VaultError::Slippage),
    (36, VaultError::RateLimited),
    (37, VaultError::PausedState),
    (38, VaultError::InvalidHotBps),
    (39, VaultError::InvalidRebalanceThreshold),
    (40, VaultError::ColdSignersEmpty),
    (41, VaultError::InvalidColdThreshold),
    (42, VaultError::DuplicateColdSigner),
    (43, VaultError::ExceedsReserveCap),
    (44, VaultError::CallerNotInAllowlist),
    // codes 45-48 reserved (formerly ProposalNotFound..BelowMinTransferAmount)
    (49, VaultError::AdminCooldownActive),
    (50, VaultError::InvalidAdminCooldown),
    (51, VaultError::ProposalNotFound),
    (52, VaultError::TimelockNotExpired),
    (53, VaultError::TimelockOverflow),
    (54, VaultError::InvalidTimelockWindow),
    (55, VaultError::BelowMinTransferAmount),
    (56, VaultError::SettlementCannotBeVault),
    (57, VaultError::SettlementCannotBeToken),
    (58, VaultError::ProposalExpired),
];

/// Verify every error code in the snapshot maps to the expected discriminant.
#[test]
fn test_vault_error_codes_frozen_against_snapshot() {
    let mut seen = BTreeSet::new();

    for (expected_code, variant) in FROZEN_ERROR_SNAPSHOT {
        let code_as_u32 = variant as u32;
        assert_eq!(
            code_as_u32, expected_code,
            "Error code for variant {:?} does not match frozen snapshot discriminant",
            variant
        );

        assert!(
            seen.insert(expected_code),
            "Duplicate error code detected in snapshot: {expected_code}"
        );
    }

    assert_eq!(
        seen.len(),
        FROZEN_ERROR_SNAPSHOT.len(),
        "Snapshot size mismatch"
    );
}

/// Verify `NotInitialized` still has discriminant 1.
#[test]
fn test_not_initialized_is_code_1() {
    assert_eq!(VaultError::NotInitialized as u32, 1);
}

/// Verify `AlreadyInitialized` still has discriminant 2.
#[test]
fn test_already_initialized_is_code_2() {
    assert_eq!(VaultError::AlreadyInitialized as u32, 2);
}

/// Verify `Unauthorized` still has discriminant 3.
#[test]
fn test_unauthorized_is_code_3() {
    assert_eq!(VaultError::Unauthorized as u32, 3);
}

/// Verify `Paused` still has discriminant 4.
#[test]
fn test_paused_is_code_4() {
    assert_eq!(VaultError::Paused as u32, 4);
}

/// Verify `InsufficientBalance` still has discriminant 5.
#[test]
fn test_insufficient_balance_is_code_5() {
    assert_eq!(VaultError::InsufficientBalance as u32, 5);
}

/// Verify `AmountNotPositive` still has discriminant 6.
#[test]
fn test_amount_not_positive_is_code_6() {
    assert_eq!(VaultError::AmountNotPositive as u32, 6);
}

/// Verify `ExceedsMaxDeduct` still has discriminant 7.
#[test]
fn test_exceeds_max_deduct_is_code_7() {
    assert_eq!(VaultError::ExceedsMaxDeduct as u32, 7);
}

/// Verify `BelowMinDeposit` still has discriminant 8.
#[test]
fn test_below_min_deposit_is_code_8() {
    assert_eq!(VaultError::BelowMinDeposit as u32, 8);
}

/// Verify `Overflow` still has discriminant 9.
#[test]
fn test_overflow_is_code_9() {
    assert_eq!(VaultError::Overflow as u32, 9);
}

/// Verify `MinDepositNotPositive` still has discriminant 11.
#[test]
fn test_min_deposit_not_positive_is_code_11() {
    assert_eq!(VaultError::MinDepositNotPositive as u32, 11);
}

/// Verify `MaxDeductNotPositive` still has discriminant 12.
#[test]
fn test_max_deduct_not_positive_is_code_12() {
    assert_eq!(VaultError::MaxDeductNotPositive as u32, 12);
}

/// Verify `MinDepositExceedsMaxDeduct` still has discriminant 13.
#[test]
fn test_min_deposit_exceeds_max_deduct_is_code_13() {
    assert_eq!(VaultError::MinDepositExceedsMaxDeduct as u32, 13);
}

/// Verify `UsdcTokenCannotBeVault` still has discriminant 14.
#[test]
fn test_usdc_token_cannot_be_vault_is_code_14() {
    assert_eq!(VaultError::UsdcTokenCannotBeVault as u32, 14);
}

/// Verify `RevenuePoolCannotBeVault` still has discriminant 15.
#[test]
fn test_revenue_pool_cannot_be_vault_is_code_15() {
    assert_eq!(VaultError::RevenuePoolCannotBeVault as u32, 15);
}

/// Verify `AuthorizedCallerCannotBeVault` still has discriminant 16.
#[test]
fn test_authorized_caller_cannot_be_vault_is_code_16() {
    assert_eq!(VaultError::AuthorizedCallerCannotBeVault as u32, 16);
}

/// Verify `InitialBalanceExceedsOnLedger` still has discriminant 17.
#[test]
fn test_initial_balance_exceeds_on_ledger_is_code_17() {
    assert_eq!(VaultError::InitialBalanceExceedsOnLedger as u32, 17);
}

/// Verify `AlreadyPaused` still has discriminant 18.
#[test]
fn test_already_paused_is_code_18() {
    assert_eq!(VaultError::AlreadyPaused as u32, 18);
}

/// Verify `NotPaused` still has discriminant 19.
#[test]
fn test_not_paused_is_code_19() {
    assert_eq!(VaultError::NotPaused as u32, 19);
}

/// Verify `SettlementNotSet` still has discriminant 20.
#[test]
fn test_settlement_not_set_is_code_20() {
    assert_eq!(VaultError::SettlementNotSet as u32, 20);
}

/// Verify `BatchEmpty` still has discriminant 21.
#[test]
fn test_batch_empty_is_code_21() {
    assert_eq!(VaultError::BatchEmpty as u32, 21);
}

/// Verify `BatchTooLarge` still has discriminant 22.
#[test]
fn test_batch_too_large_is_code_22() {
    assert_eq!(VaultError::BatchTooLarge as u32, 22);
}

/// Verify `NoAdminTransferPending` still has discriminant 25.
#[test]
fn test_no_admin_transfer_pending_is_code_25() {
    assert_eq!(VaultError::NoAdminTransferPending as u32, 25);
}

/// Verify `PriceParseError` still has discriminant 28.
#[test]
fn test_price_parse_error_is_code_28() {
    assert_eq!(VaultError::PriceParseError as u32, 28);
}

/// Verify `DuplicateRequestId` still has discriminant 29.
#[test]
fn test_duplicate_request_id_is_code_29() {
    assert_eq!(VaultError::DuplicateRequestId as u32, 29);
}

/// Verify `OfferingIdInvalid` still has discriminant 30.
#[test]
fn test_offering_id_invalid_is_code_30() {
    assert_eq!(VaultError::OfferingIdInvalid as u32, 30);
}

/// Verify `MetadataInvalid` still has discriminant 31.
#[test]
fn test_metadata_invalid_is_code_31() {
    assert_eq!(VaultError::MetadataInvalid as u32, 31);
}

/// Verify `StaleNonce` still has discriminant 32.
#[test]
fn test_stale_nonce_is_code_32() {
    assert_eq!(VaultError::StaleNonce as u32, 32);
}

/// Verify `NewRevenuePoolSameAsCurrent` still has discriminant 33.
#[test]
fn test_new_revenue_pool_same_as_current_is_code_33() {
    assert_eq!(VaultError::NewRevenuePoolSameAsCurrent as u32, 33);
}

/// Verify `NoRevenuePoolTransferPending` still has discriminant 34.
#[test]
fn test_no_revenue_pool_transfer_pending_is_code_34() {
    assert_eq!(VaultError::NoRevenuePoolTransferPending as u32, 34);
}

/// Verify `Slippage` still has discriminant 35.
#[test]
fn test_slippage_is_code_35() {
    assert_eq!(VaultError::Slippage as u32, 35);
}

/// Verify `RateLimited` still has discriminant 36.
#[test]
fn test_rate_limited_is_code_36() {
    assert_eq!(VaultError::RateLimited as u32, 36);
}

/// Verify `PausedState` still has discriminant 37.
#[test]
fn test_paused_state_is_code_37() {
    assert_eq!(VaultError::PausedState as u32, 37);
}

/// Verify `InvalidHotBps` still has discriminant 38.
#[test]
fn test_invalid_hot_bps_is_code_38() {
    assert_eq!(VaultError::InvalidHotBps as u32, 38);
}

/// Verify `InvalidRebalanceThreshold` still has discriminant 39.
#[test]
fn test_invalid_rebalance_threshold_is_code_39() {
    assert_eq!(VaultError::InvalidRebalanceThreshold as u32, 39);
}

/// Verify `ColdSignersEmpty` still has discriminant 40.
#[test]
fn test_cold_signers_empty_is_code_40() {
    assert_eq!(VaultError::ColdSignersEmpty as u32, 40);
}

/// Verify `InvalidColdThreshold` still has discriminant 41.
#[test]
fn test_invalid_cold_threshold_is_code_41() {
    assert_eq!(VaultError::InvalidColdThreshold as u32, 41);
}

/// Verify `DuplicateColdSigner` still has discriminant 42.
#[test]
fn test_duplicate_cold_signer_is_code_42() {
    assert_eq!(VaultError::DuplicateColdSigner as u32, 42);
}

/// Verify `ExceedsReserveCap` still has discriminant 43.
#[test]
fn test_exceeds_reserve_cap_is_code_43() {
    assert_eq!(VaultError::ExceedsReserveCap as u32, 43);
}

/// Verify `CallerNotInAllowlist` still has discriminant 44.
#[test]
fn test_caller_not_in_allowlist_is_code_44() {
    assert_eq!(VaultError::CallerNotInAllowlist as u32, 44);
}

/// Verify `AdminCooldownActive` still has discriminant 49.
#[test]
fn test_admin_cooldown_active_is_code_49() {
    assert_eq!(VaultError::AdminCooldownActive as u32, 49);
}

/// Verify `InvalidAdminCooldown` still has discriminant 50.
#[test]
fn test_invalid_admin_cooldown_is_code_50() {
    assert_eq!(VaultError::InvalidAdminCooldown as u32, 50);
}

/// Verify `ProposalNotFound` still has discriminant 51.
#[test]
fn test_proposal_not_found_is_code_51() {
    assert_eq!(VaultError::ProposalNotFound as u32, 51);
}

/// Verify `TimelockNotExpired` still has discriminant 52.
#[test]
fn test_timelock_not_expired_is_code_52() {
    assert_eq!(VaultError::TimelockNotExpired as u32, 52);
}

/// Verify `TimelockOverflow` still has discriminant 53.
#[test]
fn test_timelock_overflow_is_code_53() {
    assert_eq!(VaultError::TimelockOverflow as u32, 53);
}

/// Verify `InvalidTimelockWindow` still has discriminant 54.
#[test]
fn test_invalid_timelock_window_is_code_54() {
    assert_eq!(VaultError::InvalidTimelockWindow as u32, 54);
}

/// Verify `BelowMinTransferAmount` still has discriminant 55.
#[test]
fn test_below_min_transfer_amount_is_code_55() {
    assert_eq!(VaultError::BelowMinTransferAmount as u32, 55);
}

/// Verify `ProposalExpired` still has discriminant 58.
#[test]
fn test_proposal_expired_is_code_56() {
    assert_eq!(VaultError::ProposalExpired as u32, 58);
}
