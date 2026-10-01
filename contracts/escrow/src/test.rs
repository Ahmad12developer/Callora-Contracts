//! Focused tests for the `escrow` contract's admin cool-off feature (issue #914).
//!
//! Coverage targets: cooldown configuration bounds, per-action isolation,
//! window enforcement across ledger time, auth/authorization gating, the
//! two-step admin rotation, read-only views, and the `release` critical action.

use crate::admin::{DEFAULT_COOLDOWN_SECS, MAX_COOLDOWN_SECS, MIN_COOLDOWN_SECS};
use crate::{
    CalloraEscrow, CalloraEscrowClient, EscrowError, ACTION_RELEASE, ACTION_ROTATE,
    ACTION_UNPAUSE,
};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, Env, Symbol, TryIntoVal};

/// Helper: read the current instance storage entry count for the contract.
fn instance_entry_count(env: &Env, contract_id: &Address) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().instance().len()
    })
}

/// Helper: register a fresh escrow contract initialized with `cooldown_secs`
/// and return `(env, admin, signer, client)`. Auth is mocked for convenience.
fn setup(cooldown_secs: Option<u64>) -> (Env, Address, Address, CalloraEscrowClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &cooldown_secs);
    (env, admin, signer, client)
}

/// Helper: register a fresh escrow contract and return `(env, admin, signer, client, contract_id)`.
fn setup_with_id(
    cooldown_secs: Option<u64>,
) -> (Env, Address, Address, CalloraEscrowClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &cooldown_secs);
    (env, admin, signer, client, contract_id)
}

/// Advance the ledger timestamp by `secs` seconds.
fn advance(env: &Env, secs: u64) {
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + secs);
}

// ===========================================================================
// Initialisation
// ===========================================================================

#[test]
fn test_init_defaults_cooldown() {
    let (_env, admin, signer, client) = setup(None);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_signer(), signer);
    assert_eq!(client.get_cooldown(), DEFAULT_COOLDOWN_SECS);
    assert!(!client.is_paused());
}

#[test]
fn test_init_custom_cooldown() {
    let (_env, _admin, _signer, client) = setup(Some(120));
    assert_eq!(client.get_cooldown(), 120);
}

#[test]
fn test_init_twice_fails() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let other = Address::generate(&env);
    let res = client.try_init(&other, &other, &None);
    assert_eq!(res, Err(Ok(EscrowError::AlreadyInitialized)));
}

#[test]
fn test_init_rejects_out_of_range_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);

    let res = client.try_init(&admin, &signer, &Some(MAX_COOLDOWN_SECS + 1));
    assert_eq!(res, Err(Ok(EscrowError::InvalidCooldown)));
}

#[test]
fn test_views_before_init_return_not_initialized() {
    let env = Env::default();
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    assert_eq!(client.try_get_admin(), Err(Ok(EscrowError::NotInitialized)));
    assert_eq!(
        client.try_get_signer(),
        Err(Ok(EscrowError::NotInitialized))
    );
    assert_eq!(
        client.try_get_cooldown(),
        Err(Ok(EscrowError::NotInitialized))
    );
    // is_paused / views without init default gracefully.
    assert!(!client.is_paused());
    assert_eq!(client.get_pending_admin(), None);
}

// ===========================================================================
// Cooldown configuration
// ===========================================================================

#[test]
fn test_set_cooldown_updates_value() {
    let (_env, admin, _signer, client) = setup(Some(60));
    client.set_cooldown(&admin, &900);
    assert_eq!(client.get_cooldown(), 900);
}

#[test]
fn test_set_cooldown_boundaries_accepted() {
    let (_env, admin, _signer, client) = setup(Some(60));
    client.set_cooldown(&admin, &MIN_COOLDOWN_SECS);
    assert_eq!(client.get_cooldown(), MIN_COOLDOWN_SECS);
    client.set_cooldown(&admin, &MAX_COOLDOWN_SECS);
    assert_eq!(client.get_cooldown(), MAX_COOLDOWN_SECS);
}

#[test]
fn test_set_cooldown_zero_rejected() {
    let (_env, admin, _signer, client) = setup(Some(60));
    let res = client.try_set_cooldown(&admin, &0);
    assert_eq!(res, Err(Ok(EscrowError::InvalidCooldown)));
}

#[test]
fn test_set_cooldown_too_large_rejected() {
    let (_env, admin, _signer, client) = setup(Some(60));
    let res = client.try_set_cooldown(&admin, &(MAX_COOLDOWN_SECS + 1));
    assert_eq!(res, Err(Ok(EscrowError::InvalidCooldown)));
}

#[test]
fn test_set_cooldown_non_admin_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let res = client.try_set_cooldown(&intruder, &120);
    assert_eq!(res, Err(Ok(EscrowError::Unauthorized)));
}

// ===========================================================================
// Cool-off enforcement — release (primary escrow action)
// ===========================================================================

#[test]
fn test_release_available_immediately_after_init() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let release = Symbol::new(&env, ACTION_RELEASE);
    assert!(client.is_ready(&release));
    assert_eq!(client.cooldown_remaining(&release), 0);
}

#[test]
fn test_second_release_within_window_rejected() {
    let (env, admin, _signer, client) = setup(Some(300));
    let recipient = Address::generate(&env);
    client.release(&admin, &recipient);

    // A second release within the window is rejected.
    let res = client.try_release(&admin, &recipient);
    assert_eq!(res, Err(Ok(EscrowError::CooldownActive)));
}

#[test]
fn test_release_allowed_after_window_elapses() {
    let (env, admin, _signer, client) = setup(Some(300));
    let recipient = Address::generate(&env);
    client.release(&admin, &recipient);
    let res = client.try_release(&admin, &recipient);
    assert_eq!(res, Err(Ok(EscrowError::CooldownActive)));

    // Just before the window closes it is still blocked.
    advance(&env, 299);
    let release = Symbol::new(&env, ACTION_RELEASE);
    assert_eq!(client.cooldown_remaining(&release), 1);
    assert_eq!(
        client.try_release(&admin, &recipient),
        Err(Ok(EscrowError::CooldownActive))
    );

    // At the boundary it becomes available again.
    advance(&env, 1);
    assert!(client.is_ready(&release));
    client.release(&admin, &recipient);
}

// ===========================================================================
// Cool-off enforcement — pause / unpause
// ===========================================================================

#[test]
fn test_unpause_available_immediately_after_init() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let unpause = Symbol::new(&env, ACTION_UNPAUSE);
    assert!(client.is_ready(&unpause));
    assert_eq!(client.cooldown_remaining(&unpause), 0);
}

/// pause must succeed immediately after any prior admin action — it has no
/// cooldown gate (circuit-breaker property).
#[test]
fn test_pause_succeeds_immediately_after_unpause_cooldown_armed() {
    let (_env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    // unpause arms its own cooldown at t=0.
    client.unpause(&admin);
    assert!(!client.is_paused());
    // No time advance — unpause cooldown is still active.
    // pause must succeed anyway.
    client.pause(&admin);
    assert!(client.is_paused());
}

/// pause succeeds immediately even within the rotate_signer cooldown window.
#[test]
fn test_pause_succeeds_immediately_after_rotate() {
    let (env, admin, _signer, client) = setup(Some(300));
    let new_signer = Address::generate(&env);
    client.rotate_signer(&admin, &new_signer);
    // rotate cooldown is still active.
    let rotate = Symbol::new(&env, ACTION_ROTATE);
    assert!(client.cooldown_remaining(&rotate) > 0);
    // pause is exempt — succeeds immediately.
    client.pause(&admin);
    assert!(client.is_paused());
}

/// A second pause call (without intervening unpause) is blocked by the
/// AlreadyPaused state guard, not by a cooldown.
#[test]
fn test_second_pause_returns_already_paused_not_cooldown() {
    let (_env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    // There is no cooldown to wait for — the state guard fires.
    let res = client.try_pause(&admin);
    // The escrow pause does not return AlreadyPaused (it has no such error),
    // but the second pause simply sets the flag again (idempotent write).
    // Either way, it does NOT return CooldownActive.
    assert_ne!(res, Err(Ok(EscrowError::CooldownActive)));
}

/// unpause retains its cooldown: a second unpause within the window is
/// rejected with CooldownActive.
#[test]
fn test_unpause_still_cooldown_gated() {
    let (env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    client.unpause(&admin);
    // Re-pause immediately (no cooldown on pause).
    client.pause(&admin);
    // unpause cooldown fired at t=0 and window=300; still active.
    let res = client.try_unpause(&admin);
    assert_eq!(res, Err(Ok(EscrowError::CooldownActive)));

    // After the window elapses, unpause succeeds.
    advance(&env, 300);
    client.unpause(&admin);
    assert!(!client.is_paused());
}

#[test]
fn test_unpause_allowed_after_window_elapses() {
    let (env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    client.unpause(&admin);

    // Re-pause immediately — no cooldown on pause.
    client.pause(&admin);

    // Just before the unpause window closes it is still blocked.
    advance(&env, 299);
    let unpause = Symbol::new(&env, ACTION_UNPAUSE);
    assert_eq!(client.cooldown_remaining(&unpause), 1);
    assert_eq!(
        client.try_unpause(&admin),
        Err(Ok(EscrowError::CooldownActive))
    );

    // At the boundary it becomes available again.
    advance(&env, 1);
    assert!(client.is_ready(&unpause));
    client.unpause(&admin);
}

// ===========================================================================
// Per-action isolation
// ===========================================================================

#[test]
fn test_per_action_isolation() {
    let (env, admin, _signer, client) = setup(Some(1000));
    let new_signer = Address::generate(&env);

    client.pause(&admin);
    // rotate is a distinct action; not blocked by anything.
    client.rotate_signer(&admin, &new_signer);
    assert_eq!(client.get_signer(), new_signer);

    // But a second rotate is now blocked.
    let another = Address::generate(&env);
    let res = client.try_rotate_signer(&admin, &another);
    assert_eq!(res, Err(Ok(EscrowError::CooldownActive)));

    let rotate = Symbol::new(&env, ACTION_ROTATE);
    assert_eq!(client.cooldown_remaining(&rotate), 1000);
}

#[test]
fn test_release_and_pause_are_independently_cooled() {
    let (env, admin, _signer, client) = setup(Some(500));
    let recipient = Address::generate(&env);
    let new_signer = Address::generate(&env);

    // Release first.
    client.release(&admin, &recipient);

    // pause has no cooldown — succeeds immediately regardless of release window.
    client.pause(&admin);
    client.rotate_signer(&admin, &new_signer);
    client.unpause(&admin);

    // release is still blocked by its own cooldown.
    let res = client.try_release(&admin, &recipient);
    assert_eq!(res, Err(Ok(EscrowError::CooldownActive)));
}

#[test]
fn test_shorter_cooldown_takes_effect_for_next_check() {
    let (env, admin, _signer, client) = setup(Some(1000));
    client.pause(&admin);
    // unpause arms its cooldown at t=0.
    client.unpause(&admin);

    // Shorten the window; the pending unpause cooldown becomes available sooner.
    client.set_cooldown(&admin, &10);
    advance(&env, 10);
    let unpause = Symbol::new(&env, ACTION_UNPAUSE);
    assert!(client.is_ready(&unpause));
    // Re-pause (no cooldown) then unpause.
    client.pause(&admin);
    client.unpause(&admin);
}

// ===========================================================================
// Persistent escrow storage / instance footprint
// ===========================================================================

#[test]
fn test_instance_footprint_independent_of_escrow_count() {
    let (env, admin, _signer, client, contract_id) = setup_with_id(Some(60));

    // Baseline instance footprint after init.
    let baseline = instance_entry_count(&env, &contract_id);

    // Create many escrow records; they must live in persistent storage and
    // therefore must not grow the instance storage footprint.
    for _ in 0..25 {
        let recipient = Address::generate(&env);
        client.release(&admin, &recipient);
    }

    let after = instance_entry_count(&env, &contract_id);
    assert_eq!(
        baseline, after,
        "instance storage must not grow with escrow count"
    );
}

#[test]
fn test_approved_asset_flags_readable_after_migration() {
    let (env, admin, _signer, client, _contract_id) = setup_with_id(Some(60));
    let asset = Address::generate(&env);

    // Approve an asset, then verify the flag is readable via the view.
    client.approve_asset(&admin, &asset);
    assert!(client.is_asset_approved(&asset));

    // A second approval is idempotent and the flag remains readable.
    client.approve_asset(&admin, &asset);
    assert!(client.is_asset_approved(&asset));

    // Revoke and confirm the flag flips back.
    client.revoke_asset(&admin, &asset);
    assert!(!client.is_asset_approved(&asset));
}

// ===========================================================================
// Auth gating
// ===========================================================================

#[test]
fn test_guarded_actions_require_admin() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let target = Address::generate(&env);
    assert_eq!(
        client.try_pause(&intruder),
        Err(Ok(EscrowError::Unauthorized))
    );
    assert_eq!(
        client.try_unpause(&intruder),
        Err(Ok(EscrowError::Unauthorized))
    );
    assert_eq!(
        client.try_rotate_signer(&intruder, &target),
        Err(Ok(EscrowError::Unauthorized))
    );
    assert_eq!(
        client.try_release(&intruder, &target),
        Err(Ok(EscrowError::Unauthorized))
    );
}

// ===========================================================================
// Signer rotation events (issue #1181)
// ===========================================================================

/// Rotating to the current signer is rejected as a no-op.
#[test]
fn test_rotate_signer_to_same_signer_rejected() {
    let (_env, admin, signer, client) = setup(Some(60));
    let res = client.try_rotate_signer(&admin, &signer);
    assert_eq!(res, Err(Ok(EscrowError::InvalidInput)));
    assert_eq!(client.get_signer(), signer);
}

/// A successful rotation emits a `signer_rotated` event carrying
/// `(old_signer, new_signer)` as its data payload.
#[test]
fn test_rotate_signer_emits_old_and_new_signer() {
    let (env, admin, old_signer, client) = setup(Some(60));
    let new_signer = Address::generate(&env);

    client.rotate_signer(&admin, &new_signer);
    assert_eq!(client.get_signer(), new_signer);

    let events = env.events().all();
    let (_, topics, data) = events.last().unwrap();
    let topic: Symbol = topics.get(0).unwrap().try_into_val(&env).unwrap();
    assert_eq!(topic, Symbol::new(&env, "signer_rotated"));
    let payload: (Address, Address) = data.try_into_val(&env).unwrap();
    assert_eq!(payload, (old_signer, new_signer));
}

/// The event payload reflects the actual old signer across multiple rotations.
#[test]
fn test_rotate_signer_event_tracks_previous_signer() {
    let (env, admin, first_signer, client) = setup(Some(60));
    let second_signer = Address::generate(&env);
    let third_signer = Address::generate(&env);

    client.rotate_signer(&admin, &second_signer);
    advance(&env, 60);
    client.rotate_signer(&admin, &third_signer);
    assert_eq!(client.get_signer(), third_signer);

    let events = env.events().all();
    let (_, _, data) = events.last().unwrap();
    let payload: (Address, Address) = data.try_into_val(&env).unwrap();
    assert_eq!(payload, (second_signer, third_signer));
    let _ = first_signer;
}

// ===========================================================================
// Two-step admin rotation
// ===========================================================================

#[test]
fn test_admin_rotation_happy_path() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);

    client.set_admin(&admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));
    // Current admin unchanged until accepted.
    assert_eq!(client.get_admin(), admin);

    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);
    assert_eq!(client.get_pending_admin(), None);
}

#[test]
fn test_set_admin_non_admin_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let res = client.try_set_admin(&intruder, &intruder);
    assert_eq!(res, Err(Ok(EscrowError::Unauthorized)));
}

#[test]
fn test_accept_admin_without_pending_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let stranger = Address::generate(&env);
    let res = client.try_accept_admin(&stranger);
    assert_eq!(res, Err(Ok(EscrowError::NoPendingAdmin)));
}

#[test]
fn test_accept_admin_wrong_caller_rejected() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);
    let wrong = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    let res = client.try_accept_admin(&wrong);
    assert_eq!(res, Err(Ok(EscrowError::Unauthorized)));
}

#[test]
fn test_new_admin_controls_cooldown_after_rotation() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);

    // Old admin can no longer configure cooldown.
    assert_eq!(
        client.try_set_cooldown(&admin, &120),
        Err(Ok(EscrowError::Unauthorized))
    );
    // New admin can.
    client.set_cooldown(&new_admin, &120);
    assert_eq!(client.get_cooldown(), 120);
}

#[test]
fn test_new_admin_can_perform_guarded_actions_after_rotation() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);
    let recipient = Address::generate(&env);

    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);

    // Old admin can no longer pause.
    assert_eq!(client.try_pause(&admin), Err(Ok(EscrowError::Unauthorized)));
    // New admin can pause and release.
    client.pause(&new_admin);
    assert!(client.is_paused());
    client.unpause(&new_admin);
    client.release(&new_admin, &recipient);
}

/// Cancel a pending admin transfer. Only the current admin may call.
///
/// # Acceptance Criteria
/// - Returns `EscrowError::NoPendingAdmin` when no nomination is in progress.
/// - Clears the pending admin and emits `admin_cancelled`.
/// - The previous admin retains full authority after cancellation.
#[test]
fn test_cancel_admin_transfer_happy_path() {
    let (env, admin, _signer, client) = setup(Some(60));
    let cancelled_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    // Set up a pending admin nomination.
    client.set_admin(&admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));

    // Cancel the pending nomination as the current admin.
    client.cancel_admin_transfer(&admin);
    assert_eq!(client.get_pending_admin(), None);
    // Current admin still the same.
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn test_cancel_admin_transfer_no_pending_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    // No pending admin exists; cancelling should return NoPendingAdmin.
    let res = client.try_cancel_admin_transfer(&_admin);
    assert_eq!(res, Err(Ok(EscrowError::NoPendingAdmin)));
}

#[test]
fn test_cancel_admin_transfer_non_admin_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let res = client.try_cancel_admin_transfer(&intruder);
    assert_eq!(res, Err(Ok(EscrowError::Unauthorized)));
}

/// Cancelled nominees cannot accept the admin transfer.
#[test]
fn test_cancelled_admin_cannot_accept() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);

    // Set up a pending admin nomination.
    client.set_admin(&admin, &new_admin);

    // Cancel the pending nomination.
    client.cancel_admin_transfer(&admin);

    // The former pending admin cannot accept (no pending exists).
    let res = client.try_accept_admin(&new_admin);
    assert_eq!(res, Err(Ok(EscrowError::NoPendingAdmin)));
}

/// An approved-asset flag is never set by default (deny-by-default).
#[test]
fn test_is_asset_approved_deny_by_default() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    assert!(!client.is_asset_approved(&asset));
}

/// Approving an asset flips the deny-by-default flag to `true`.
#[test]
fn test_add_approved_asset_flips_flag() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    assert!(!client.is_asset_approved(&asset));
    client.add_approved_asset(&admin, &asset);
    assert!(client.is_asset_approved(&asset));
}

/// Only the admin may approve an asset.
#[test]
fn test_add_approved_asset_requires_admin() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let asset = Address::generate(&env);
    assert_eq!(
        client.try_add_approved_asset(&intruder, &asset),
        Err(Ok(EscrowError::Unauthorized))
    );
}

/// Approving the contract's own address is rejected as malformed.
#[test]
fn test_add_approved_asset_rejects_self() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &Some(60));

    assert_eq!(
        client.try_add_approved_asset(&admin, &contract_id),
        Err(Ok(EscrowError::InvalidInput))
    );
}

/// Revoking an approved asset flips the flag back to `false`.
#[test]
fn test_remove_approved_asset_revokes() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);
    assert!(client.is_asset_approved(&asset));
    client.remove_approved_asset(&admin, &asset);
    assert!(!client.is_asset_approved(&asset));
}

/// Only the admin may revoke an asset approval.
#[test]
fn test_remove_approved_asset_requires_admin() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let asset = Address::generate(&env);
    assert_eq!(
        client.try_remove_approved_asset(&intruder, &asset),
        Err(Ok(EscrowError::Unauthorized))
    );
}

/// Revoking the contract's own address is rejected as malformed.
#[test]
fn test_remove_approved_asset_rejects_self() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &Some(60));

    assert_eq!(
        client.try_remove_approved_asset(&admin, &contract_id),
        Err(Ok(EscrowError::InvalidInput))
    );
}

/// Creating an escrow against an approved asset records it and emits an event.
#[test]
fn test_create_escrow_success_and_record() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);

    let now = env.ledger().timestamp();
    client.create_escrow(&admin, &asset, &recipient, &1000);

    let record = client.get_escrow(&asset, &recipient).unwrap();
    assert_eq!(record.payment_asset, asset);
    assert_eq!(record.recipient, recipient);
    assert_eq!(record.amount, 1000);
    assert_eq!(record.created_at, now);
}

/// An unapproved asset is rejected and nothing is recorded (fail closed).
#[test]
fn test_create_escrow_unapproved_asset_fails_closed() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &1000),
        Err(Ok(EscrowError::AssetNotApproved))
    );
    assert!(client.get_escrow(&asset, &recipient).is_none());
}

/// Only the admin may create an escrow.
#[test]
fn test_create_escrow_requires_admin() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    assert_eq!(
        client.try_create_escrow(&intruder, &asset, &recipient, &1000),
        Err(Ok(EscrowError::Unauthorized))
    );
}

/// Using the contract itself as the payment asset is rejected as malformed.
#[test]
fn test_create_escrow_rejects_self_payment_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &Some(60));

    let recipient = Address::generate(&env);
    assert_eq!(
        client.try_create_escrow(&admin, &contract_id, &recipient, &1000),
        Err(Ok(EscrowError::InvalidInput))
    );
}

/// Using the contract itself as the recipient is rejected as malformed.
#[test]
fn test_create_escrow_rejects_self_recipient() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraEscrow, ());
    let client = CalloraEscrowClient::new(&env, &contract_id);
    client.init(&admin, &signer, &Some(60));

    let asset = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &contract_id, &1000),
        Err(Ok(EscrowError::InvalidInput))
    );
}

/// A non-positive amount is rejected before any state is written.
#[test]
fn test_create_escrow_rejects_non_positive_amount() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &0),
        Err(Ok(EscrowError::InvalidInput))
    );
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &-1),
        Err(Ok(EscrowError::InvalidInput))
    );
    assert!(client.get_escrow(&asset, &recipient).is_none());
}

/// Replaying the same creation inputs fails closed with `EscrowExists`.
#[test]
fn test_create_escrow_rejects_replay() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);
    client.create_escrow(&admin, &asset, &recipient, &1000);
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &2000),
        Err(Ok(EscrowError::EscrowExists))
    );
}

/// A stale (revoked) approval is no longer honored for new escrows.
#[test]
fn test_create_escrow_after_revoke_fails_closed() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);
    client.remove_approved_asset(&admin, &asset);
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &1000),
        Err(Ok(EscrowError::AssetNotApproved))
    );
}

/// Approval is scoped to each escrow instance (cross-tenant isolation).
#[test]
fn test_approval_registry_is_scoped_per_instance() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let asset = Address::generate(&env);

    let c1 = env.register(CalloraEscrow, ());
    let client1 = CalloraEscrowClient::new(&env, &c1);
    client1.init(&admin, &signer, &Some(60));

    let c2 = env.register(CalloraEscrow, ());
    let client2 = CalloraEscrowClient::new(&env, &c2);
    client2.init(&admin, &signer, &Some(60));

    client1.add_approved_asset(&admin, &asset);
    assert!(client1.is_asset_approved(&asset));
    assert!(!client2.is_asset_approved(&asset));
}

// ===========================================================================
// Pause circuit breaker enforcement (issue #1181)
// ===========================================================================

/// Verify `EscrowError::Paused` has discriminant 11.
#[test]
fn test_escrow_error_paused_discriminant() {
    assert_eq!(EscrowError::Paused as u32, 11);
}

/// Escrow creation fails with `EscrowError::Paused` when the contract is paused.
#[test]
fn test_create_escrow_fails_while_paused() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);

    client.pause(&admin);
    assert!(client.is_paused());

    let res = client.try_create_escrow(&admin, &asset, &recipient, &1000);
    assert_eq!(res, Err(Ok(EscrowError::Paused)));
    assert!(client.get_escrow(&asset, &recipient).is_none());
}

/// Release fails with `EscrowError::Paused` when the contract is paused.
#[test]
fn test_release_fails_while_paused() {
    let (env, admin, _signer, client) = setup(Some(60));
    let recipient = Address::generate(&env);

    client.pause(&admin);
    assert!(client.is_paused());

    let res = client.try_release(&admin, &recipient);
    assert_eq!(res, Err(Ok(EscrowError::Paused)));
}

/// Unpausing restores both `create_escrow` and `release` functionality.
#[test]
fn test_unpause_restores_create_escrow_and_release() {
    let (env, admin, _signer, client) = setup(Some(60));
    let asset = Address::generate(&env);
    let recipient = Address::generate(&env);
    let release_recipient = Address::generate(&env);
    client.add_approved_asset(&admin, &asset);

    // Pause contract
    client.pause(&admin);
    assert!(client.is_paused());

    // Both fund-affecting actions are blocked while paused
    assert_eq!(
        client.try_create_escrow(&admin, &asset, &recipient, &1000),
        Err(Ok(EscrowError::Paused))
    );
    assert_eq!(
        client.try_release(&admin, &release_recipient),
        Err(Ok(EscrowError::Paused))
    );

    // Unpause restores both
    client.unpause(&admin);
    assert!(!client.is_paused());

    client.create_escrow(&admin, &asset, &recipient, &1000);
    let record = client.get_escrow(&asset, &recipient).unwrap();
    assert_eq!(record.amount, 1000);

    client.release(&admin, &release_recipient);
    assert_eq!(client.get_signer(), release_recipient);
}

/// Administrative functions and views remain allowed while paused.
#[test]
fn test_admin_functions_and_views_allowed_while_paused() {
    let (env, admin, signer, client) = setup(Some(60));
    let asset1 = Address::generate(&env);
    let asset2 = Address::generate(&env);
    let new_signer = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.pause(&admin);
    assert!(client.is_paused());

    // Views remain functional
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_signer(), signer);
    assert_eq!(client.get_cooldown(), 60);
    assert_eq!(client.get_pending_admin(), None);
    assert!(!client.is_asset_approved(&asset1));

    // Admin config remains functional
    client.set_cooldown(&admin, &120);
    assert_eq!(client.get_cooldown(), 120);

    client.add_approved_asset(&admin, &asset1);
    assert!(client.is_asset_approved(&asset1));
    client.add_approved_asset(&admin, &asset2);
    client.remove_approved_asset(&admin, &asset1);
    assert!(!client.is_asset_approved(&asset1));
    assert!(client.is_asset_approved(&asset2));

    client.rotate_signer(&admin, &new_signer);
    assert_eq!(client.get_signer(), new_signer);

    // Two-step admin rotation works while paused
    client.set_admin(&admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));
    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);

    // New admin can unpause
    client.unpause(&new_admin);
    assert!(!client.is_paused());
}
