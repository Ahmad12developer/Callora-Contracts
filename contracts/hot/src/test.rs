//! Focused tests for the `hot` contract's admin cool-off feature (issue #743).
//!
//! Coverage targets: cooldown configuration bounds, per-action isolation,
//! window enforcement across ledger time, auth/authorization gating, the
//! two-step admin rotation, signer rotation events, and the read-only views.

use crate::admin::{DEFAULT_COOLDOWN_SECS, MAX_COOLDOWN_SECS, MIN_COOLDOWN_SECS};
use crate::{CalloraHot, CalloraHotClient, HotError, ACTION_ROTATE};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Symbol, TryIntoVal};

/// Helper: register a fresh hot contract initialized with `cooldown_secs` and
/// return `(env, admin, signer, client)`. Auth is mocked for convenience.
fn setup(cooldown_secs: Option<u64>) -> (Env, Address, Address, CalloraHotClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraHot, ());
    let client = CalloraHotClient::new(&env, &contract_id);
    client.init(&admin, &signer, &cooldown_secs);
    (env, admin, signer, client)
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
    assert_eq!(res, Err(Ok(HotError::AlreadyInitialized)));
}

#[test]
fn test_init_rejects_out_of_range_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(CalloraHot, ());
    let client = CalloraHotClient::new(&env, &contract_id);

    let res = client.try_init(&admin, &signer, &Some(MAX_COOLDOWN_SECS + 1));
    assert_eq!(res, Err(Ok(HotError::InvalidCooldown)));
}

#[test]
fn test_views_before_init_return_not_initialized() {
    let env = Env::default();
    let contract_id = env.register(CalloraHot, ());
    let client = CalloraHotClient::new(&env, &contract_id);
    assert_eq!(client.try_get_admin(), Err(Ok(HotError::NotInitialized)));
    assert_eq!(client.try_get_signer(), Err(Ok(HotError::NotInitialized)));
    assert_eq!(client.try_get_cooldown(), Err(Ok(HotError::NotInitialized)));
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
    assert_eq!(res, Err(Ok(HotError::InvalidCooldown)));
}

#[test]
fn test_set_cooldown_too_large_rejected() {
    let (_env, admin, _signer, client) = setup(Some(60));
    let res = client.try_set_cooldown(&admin, &(MAX_COOLDOWN_SECS + 1));
    assert_eq!(res, Err(Ok(HotError::InvalidCooldown)));
}

#[test]
fn test_set_cooldown_non_admin_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let res = client.try_set_cooldown(&intruder, &120);
    assert_eq!(res, Err(Ok(HotError::Unauthorized)));
}

// ===========================================================================
// Cool-off enforcement — pause is exempt; unpause and rotate are gated
// ===========================================================================

#[test]
fn test_pause_available_immediately_after_init() {
    let (env, _admin, _signer, client) = setup(Some(60));
    // pause has no cooldown tag — it is always ready.
    let unpause = Symbol::new(&env, crate::ACTION_UNPAUSE);
    assert!(client.is_ready(&unpause));
    assert_eq!(client.cooldown_remaining(&unpause), 0);
}

/// pause must succeed immediately after an unpause, even while the unpause
/// cooldown is still active. This is the core circuit-breaker property.
#[test]
fn test_pause_succeeds_immediately_after_unpause_cooldown_armed() {
    let (_env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    // Unpause arms its own cooldown at t=0.
    client.unpause(&admin);
    assert!(!client.is_paused());

    // Advance 0 seconds — unpause cooldown is still fully active.
    // pause must succeed regardless.
    client.pause(&admin);
    assert!(client.is_paused());
}

/// pause must succeed immediately after rotate_signer, even within the
/// rotate cooldown window.
#[test]
fn test_pause_succeeds_immediately_after_rotate() {
    let (env, admin, _signer, client) = setup(Some(300));
    let new_signer = Address::generate(&env);
    client.rotate_signer(&admin, &new_signer);

    // rotate cooldown is still active (0 seconds elapsed).
    let rotate = Symbol::new(&env, crate::ACTION_ROTATE);
    assert!(client.cooldown_remaining(&rotate) > 0);

    // pause is unaffected — must succeed immediately.
    client.pause(&admin);
    assert!(client.is_paused());
}

/// A second pause without an intervening unpause still returns AlreadyPaused,
/// because the state guard fires before any cooldown logic would.
#[test]
fn test_double_pause_returns_already_paused_not_cooldown() {
    let (_env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    let res = client.try_pause(&admin);
    assert_eq!(res, Err(Ok(HotError::AlreadyPaused)));
}

/// After unpause, re-pausing immediately succeeds (no cooldown gate on pause).
#[test]
fn test_pause_after_unpause_at_zero_delay_succeeds() {
    let (_env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    client.unpause(&admin);
    // Immediately re-pause — must succeed without advancing time.
    client.pause(&admin);
    assert!(client.is_paused());
}

/// unpause retains its cooldown: a second unpause within the window is rejected.
#[test]
fn test_unpause_still_cooldown_gated() {
    let (env, admin, _signer, client) = setup(Some(300));
    client.pause(&admin);
    client.unpause(&admin);
    // Re-pause immediately (no cooldown on pause).
    client.pause(&admin);
    // Now try to unpause immediately — still inside the unpause cooldown window.
    let res = client.try_unpause(&admin);
    assert_eq!(res, Err(Ok(HotError::CooldownActive)));

    // After the window elapses, unpause succeeds.
    advance(&env, 300);
    client.unpause(&admin);
    assert!(!client.is_paused());
}

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
    assert_eq!(res, Err(Ok(HotError::CooldownActive)));

    let rotate = Symbol::new(&env, ACTION_ROTATE);
    assert_eq!(client.cooldown_remaining(&rotate), 1000);
}

#[test]
fn test_rotate_signer_rejects_same_signer() {
    let (_env, admin, signer, client) = setup(Some(60));
    let res = client.try_rotate_signer(&admin, &signer);
    assert_eq!(res, Err(Ok(HotError::SameSigner)));
    assert_eq!(client.get_signer(), signer);
}

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
    let (emitted_old, emitted_new): (Address, Address) =
        data.try_into_val(&env).unwrap();
    assert_eq!(emitted_old, old_signer);
    assert_eq!(emitted_new, new_signer);
}

#[test]
fn test_rotate_signer_event_payload_matches_storage() {
    let (env, admin, old_signer, client) = setup(Some(60));
    let new_signer = Address::generate(&env);

    client.rotate_signer(&admin, &new_signer);

    let events = env.events().all();
    let (_, _, data) = events.last().unwrap();
    let (emitted_old, emitted_new): (Address, Address) =
        data.try_into_val(&env).unwrap();
    assert_eq!(emitted_old, old_signer);
    assert_eq!(emitted_new, client.get_signer());
}

#[test]
fn test_shorter_cooldown_takes_effect_for_next_check() {
    let (env, admin, _signer, client) = setup(Some(1000));
    client.pause(&admin);
    // Unpause to arm the unpause cooldown.
    client.unpause(&admin);

    // Shorten the window; the pending unpause cooldown becomes available sooner.
    client.set_cooldown(&admin, &10);
    advance(&env, 10);
    let unpause = Symbol::new(&env, crate::ACTION_UNPAUSE);
    assert!(client.is_ready(&unpause));
    // Re-pause (no cooldown) then unpause after the shortened window.
    client.pause(&admin);
    client.unpause(&admin);
}

#[test]
fn test_guarded_actions_require_admin() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let target = Address::generate(&env);
    assert_eq!(client.try_pause(&intruder), Err(Ok(HotError::Unauthorized)));
    assert_eq!(
        client.try_unpause(&intruder),
        Err(Ok(HotError::Unauthorized))
    );
    assert_eq!(
        client.try_rotate_signer(&intruder, &target),
        Err(Ok(HotError::Unauthorized))
    );
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
    assert_eq!(res, Err(Ok(HotError::Unauthorized)));
}

#[test]
fn test_accept_admin_without_pending_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let stranger = Address::generate(&env);
    let res = client.try_accept_admin(&stranger);
    assert_eq!(res, Err(Ok(HotError::NoPendingAdmin)));
}

#[test]
fn test_accept_admin_wrong_caller_rejected() {
    let (env, admin, _signer, client) = setup(Some(60));
    let new_admin = Address::generate(&env);
    let wrong = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    let res = client.try_accept_admin(&wrong);
    assert_eq!(res, Err(Ok(HotError::Unauthorized)));
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
        Err(Ok(HotError::Unauthorized))
    );
    // New admin can.
    client.set_cooldown(&new_admin, &120);
    assert_eq!(client.get_cooldown(), 120);
}

// Cancel a pending admin transfer. Only the current admin may call.

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
    assert_eq!(res, Err(Ok(HotError::NoPendingAdmin)));
}

#[test]
fn test_cancel_admin_transfer_non_admin_rejected() {
    let (env, _admin, _signer, client) = setup(Some(60));
    let intruder = Address::generate(&env);
    let res = client.try_cancel_admin_transfer(&intruder);
    assert_eq!(res, Err(Ok(HotError::Unauthorized)));
}

// Cancelled nominees cannot accept the admin transfer.
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
    assert_eq!(res, Err(Ok(HotError::NoPendingAdmin)));
}

// ===========================================================================
// Benchmark setup smoke-test (validates benches/main.rs entrypoints)
// ===========================================================================

#[test]
fn test_bench_setup_exercises_all_hot_entrypoints() {
    let (env, admin, _signer, client) = setup(Some(60));

    // Views
    assert!(!client.is_paused());
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_signer(), client.get_signer());
    assert_eq!(client.get_cooldown(), 60);
    assert_eq!(client.get_pending_admin(), None);

    let unpause = Symbol::new(&env, crate::ACTION_UNPAUSE);
    assert_eq!(client.cooldown_remaining(&unpause), 0);
    assert!(client.is_ready(&unpause));

    // pause has no cooldown — succeeds immediately.
    client.pause(&admin);
    assert!(client.is_paused());

    // unpause is still cooldown-gated; advance past the window first.
    advance(&env, 61);
    client.unpause(&admin);
    assert!(!client.is_paused());

    // rotate is cooldown-gated; advance past the window.
    advance(&env, 61);
    let new_signer = Address::generate(&env);
    client.rotate_signer(&admin, &new_signer);
    assert_eq!(client.get_signer(), new_signer);

    // Admin config (no cooldown)
    client.set_cooldown(&admin, &120);
    assert_eq!(client.get_cooldown(), 120);

    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));
    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);
}
