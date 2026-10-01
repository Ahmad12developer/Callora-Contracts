//! Tests for the Callora Upgrade crate.
//!
//! Covers:
//! - default and configured cooldown values
//! - cooldown enforcement (not-elapsed, exact-boundary, elapsed)
//! - event emission: `cooldown_set`, `upgrade_started`, `upgrade_recorded`
#![cfg(test)]
extern crate std;
use crate::admin::{check_and_record_upgrade, get_cooldown, set_cooldown, DEFAULT_COOLDOWN_SECONDS};
use crate::errors::UpgradeError;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, IntoVal, Symbol,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return all events published by any contract in the environment, filtered
/// to those whose first topic is the given string.
fn events_with_topic<'a>(
    env: &'a Env,
    topic: &str,
) -> std::vec::Vec<(
    soroban_sdk::Vec<soroban_sdk::Val>,
    soroban_sdk::Val,
)> {
    let needle = Symbol::new(env, topic);
    env.events()
        .all()
        .iter()
        .filter(|e| {
            !e.1.is_empty() && {
                let t0: Symbol = e.1.get(0).unwrap().into_val(env);
                t0 == needle
            }
        })
        .map(|e| (e.1, e.2))
        .collect()
}

// ===========================================================================
// Existing cooldown behaviour tests
// ===========================================================================

#[test]
fn test_default_cooldown() {
    let env = Env::default();
    let cooldown = get_cooldown(&env);
    assert_eq!(cooldown, DEFAULT_COOLDOWN_SECONDS);
}

#[test]
fn test_set_and_get_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    set_cooldown(&env, &caller, 3600);
    assert_eq!(get_cooldown(&env), 3600);
}

#[test]
fn test_check_and_record_upgrade_first_time() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    // First time should always succeed
    let res = check_and_record_upgrade(&env, &caller);
    assert!(res.is_ok());

    // Last upgrade time should be set to current ledger timestamp (0 in tests by default)
    let last_time = env
        .storage()
        .instance()
        .get::<_, u64>(&Symbol::new(&env, "last_upg_tm"))
        .unwrap();
    assert_eq!(last_time, env.ledger().timestamp());
}

#[test]
fn test_check_and_record_upgrade_cooldown_not_elapsed() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    // Set timestamp to 100
    env.ledger().set_timestamp(100);

    // First time succeeds, records timestamp 100
    assert!(check_and_record_upgrade(&env, &caller).is_ok());

    // Set timestamp to 100 + DEFAULT_COOLDOWN_SECONDS - 1
    env.ledger()
        .set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS - 1);

    // Should fail because cooldown hasn't elapsed
    let res = check_and_record_upgrade(&env, &caller);
    assert_eq!(res, Err(UpgradeError::CooldownNotElapsed));
}

#[test]
fn test_check_and_record_upgrade_cooldown_elapsed() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    env.ledger().set_timestamp(100);

    // First time succeeds
    assert!(check_and_record_upgrade(&env, &caller).is_ok());

    // Set timestamp exactly to cooldown
    env.ledger().set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS);

    // Should succeed now
    assert!(check_and_record_upgrade(&env, &caller).is_ok());

    // Last upgrade time should be updated to 100 + DEFAULT_COOLDOWN_SECONDS
    let last_time = env
        .storage()
        .instance()
        .get::<_, u64>(&Symbol::new(&env, "last_upg_tm"))
        .unwrap();
    assert_eq!(last_time, 100 + DEFAULT_COOLDOWN_SECONDS);
}

// ===========================================================================
// Event emission tests
// ===========================================================================

/// `set_cooldown` must publish exactly one `cooldown_set` event.
///
/// Shape: topics `(cooldown_set, caller)`, data `new_cooldown_secs`.
#[test]
fn set_cooldown_emits_cooldown_set_event() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);
    let new_cooldown: u64 = 3_600;

    set_cooldown(&env, &caller, new_cooldown);

    let matches = events_with_topic(&env, "cooldown_set");
    assert_eq!(matches.len(), 1, "expected exactly one cooldown_set event");

    let (topics, data) = &matches[0];
    assert_eq!(topics.len(), 2, "cooldown_set must carry exactly 2 topics");

    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "cooldown_set"));

    let topic1: Address = topics.get(1).unwrap().into_val(&env);
    assert_eq!(topic1, caller, "topic[1] must be the caller address");

    let payload: u64 = data.into_val(&env);
    assert_eq!(payload, new_cooldown, "data must be the new cooldown value");
}

/// `set_cooldown` called twice emits two separate `cooldown_set` events,
/// one for each invocation.
#[test]
fn set_cooldown_emits_separate_events_on_repeated_calls() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    set_cooldown(&env, &caller, 1_800);
    set_cooldown(&env, &caller, 7_200);

    let matches = events_with_topic(&env, "cooldown_set");
    assert_eq!(matches.len(), 2, "expected two cooldown_set events");

    let payload_a: u64 = matches[0].1.into_val(&env);
    let payload_b: u64 = matches[1].1.into_val(&env);
    assert_eq!(payload_a, 1_800);
    assert_eq!(payload_b, 7_200);
}

/// A successful `check_and_record_upgrade` (first call, no prior record) must
/// emit `upgrade_started` followed by `upgrade_recorded`.
///
/// `upgrade_started` shape:
///   topics `(upgrade_started, caller)`, data `(current_timestamp, cooldown)`.
/// `upgrade_recorded` shape:
///   topics `(upgrade_recorded, caller)`, data `recorded_timestamp`.
#[test]
fn check_and_record_upgrade_first_call_emits_started_and_recorded() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);
    env.ledger().set_timestamp(500);

    check_and_record_upgrade(&env, &caller).unwrap();

    // ── upgrade_started ──────────────────────────────────────────────────
    let started = events_with_topic(&env, "upgrade_started");
    assert_eq!(started.len(), 1, "expected exactly one upgrade_started event");

    let (topics_s, data_s) = &started[0];
    assert_eq!(topics_s.len(), 2);
    let s_topic0: Symbol = topics_s.get(0).unwrap().into_val(&env);
    assert_eq!(s_topic0, Symbol::new(&env, "upgrade_started"));
    let s_topic1: Address = topics_s.get(1).unwrap().into_val(&env);
    assert_eq!(s_topic1, caller, "upgrade_started topic[1] must be caller");

    let (ts, cooldown): (u64, u64) = data_s.into_val(&env);
    assert_eq!(ts, 500, "upgrade_started data[0] must be current_timestamp");
    assert_eq!(
        cooldown,
        DEFAULT_COOLDOWN_SECONDS,
        "upgrade_started data[1] must be the cooldown window"
    );

    // ── upgrade_recorded ─────────────────────────────────────────────────
    let recorded = events_with_topic(&env, "upgrade_recorded");
    assert_eq!(
        recorded.len(),
        1,
        "expected exactly one upgrade_recorded event"
    );

    let (topics_r, data_r) = &recorded[0];
    assert_eq!(topics_r.len(), 2);
    let r_topic0: Symbol = topics_r.get(0).unwrap().into_val(&env);
    assert_eq!(r_topic0, Symbol::new(&env, "upgrade_recorded"));
    let r_topic1: Address = topics_r.get(1).unwrap().into_val(&env);
    assert_eq!(r_topic1, caller, "upgrade_recorded topic[1] must be caller");

    let recorded_ts: u64 = data_r.into_val(&env);
    assert_eq!(
        recorded_ts, 500,
        "upgrade_recorded data must be the recorded timestamp"
    );
}

/// When the cooldown has elapsed, a second `check_and_record_upgrade` call
/// also emits both events with the updated timestamp.
#[test]
fn check_and_record_upgrade_second_call_after_cooldown_emits_events() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    env.ledger().set_timestamp(100);
    check_and_record_upgrade(&env, &caller).unwrap();

    // Advance past the default cooldown.
    env.ledger().set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS);
    check_and_record_upgrade(&env, &caller).unwrap();

    // Two pairs of events should have been emitted in total.
    let started = events_with_topic(&env, "upgrade_started");
    let recorded = events_with_topic(&env, "upgrade_recorded");
    assert_eq!(started.len(), 2);
    assert_eq!(recorded.len(), 2);

    // The second upgrade_started data must carry the new timestamp.
    let (ts2, _): (u64, u64) = started[1].1.into_val(&env);
    assert_eq!(ts2, 100 + DEFAULT_COOLDOWN_SECONDS);

    let recorded_ts2: u64 = recorded[1].1.into_val(&env);
    assert_eq!(recorded_ts2, 100 + DEFAULT_COOLDOWN_SECONDS);
}

/// When the cooldown has NOT elapsed, `check_and_record_upgrade` returns
/// `Err(CooldownNotElapsed)` and must NOT emit any events.
#[test]
fn check_and_record_upgrade_cooldown_not_elapsed_emits_no_events() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);

    env.ledger().set_timestamp(100);
    check_and_record_upgrade(&env, &caller).unwrap();

    // Advance to just before the cooldown window expires.
    env.ledger()
        .set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS - 1);
    let res = check_and_record_upgrade(&env, &caller);
    assert_eq!(res, Err(UpgradeError::CooldownNotElapsed));

    // Only the first call's events should be present (one each).
    let started = events_with_topic(&env, "upgrade_started");
    let recorded = events_with_topic(&env, "upgrade_recorded");
    assert_eq!(
        started.len(),
        1,
        "failed call must not emit additional upgrade_started events"
    );
    assert_eq!(
        recorded.len(),
        1,
        "failed call must not emit additional upgrade_recorded events"
    );
}

/// `upgrade_started` is emitted BEFORE `upgrade_recorded` within the same call.
/// We verify ordering by comparing their positions in `env.events().all()`.
#[test]
fn upgrade_started_precedes_upgrade_recorded_in_event_order() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);
    env.ledger().set_timestamp(1_000);

    check_and_record_upgrade(&env, &caller).unwrap();

    let started_sym = Symbol::new(&env, "upgrade_started");
    let recorded_sym = Symbol::new(&env, "upgrade_recorded");

    let all = env.events().all();
    let mut started_pos: Option<usize> = None;
    let mut recorded_pos: Option<usize> = None;

    for (i, event) in all.iter().enumerate() {
        if event.1.is_empty() {
            continue;
        }
        let t0: Symbol = event.1.get(0).unwrap().into_val(&env);
        if t0 == started_sym {
            started_pos = Some(i);
        } else if t0 == recorded_sym {
            recorded_pos = Some(i);
        }
    }

    assert!(
        started_pos.is_some(),
        "upgrade_started event not found in event log"
    );
    assert!(
        recorded_pos.is_some(),
        "upgrade_recorded event not found in event log"
    );
    assert!(
        started_pos.unwrap() < recorded_pos.unwrap(),
        "upgrade_started must appear before upgrade_recorded in the event log"
    );
}

/// `check_and_record_upgrade` with a custom cooldown reports the custom window
/// in the `upgrade_started` data payload.
#[test]
fn upgrade_started_data_reflects_custom_cooldown() {
    let env = Env::default();
    env.mock_all_auths();
    let caller = Address::generate(&env);
    let custom_cooldown: u64 = 7_200;

    set_cooldown(&env, &caller, custom_cooldown);
    env.ledger().set_timestamp(300);
    check_and_record_upgrade(&env, &caller).unwrap();

    let started = events_with_topic(&env, "upgrade_started");
    assert_eq!(started.len(), 1);
    let (ts, cw): (u64, u64) = started[0].1.into_val(&env);
    assert_eq!(ts, 300);
    assert_eq!(
        cw, custom_cooldown,
        "upgrade_started data must report the active cooldown window"
    );
}
