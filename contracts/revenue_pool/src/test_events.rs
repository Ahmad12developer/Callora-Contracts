//! Focused event tests for revenue_pool structured events
//!
//! Tests ensuring all 23 lifecycle events are properly structured.

extern crate std;

use crate::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol, TryFromVal, Val};

#[test]
fn all_event_constructors_return_correct_symbols() {
    let env = Env::default();

    // Test all 23 event constructor functions return correct string symbols
    assert_eq!(events::event_init(&env), Symbol::new(&env, "init"));
    assert_eq!(
        events::event_admin_changed(&env),
        Symbol::new(&env, "admin_changed")
    );
    assert_eq!(
        events::event_admin_transfer_started(&env),
        Symbol::new(&env, "admin_transfer_started")
    );
    assert_eq!(
        events::event_admin_transfer_completed(&env),
        Symbol::new(&env, "admin_transfer_completed")
    );
    assert_eq!(
        events::event_admin_cancelled(&env),
        Symbol::new(&env, "admin_cancelled")
    );
    assert_eq!(
        events::event_pause_guardian_set(&env),
        Symbol::new(&env, "pause_guardian_set")
    );
    assert_eq!(
        events::event_pause_guardian_cleared(&env),
        Symbol::new(&env, "pause_guardian_cleared")
    );
    assert_eq!(
        events::event_pause_set(&env),
        Symbol::new(&env, "pause_set")
    );
    assert_eq!(
        events::event_receive_payment(&env),
        Symbol::new(&env, "receive_payment")
    );
    assert_eq!(
        events::event_yield_deposited(&env),
        Symbol::new(&env, "yield_deposited")
    );
    assert_eq!(
        events::event_treasury_transfer_started(&env),
        Symbol::new(&env, "treasury_transfer_started")
    );
    assert_eq!(
        events::event_treasury_transfer_completed(&env),
        Symbol::new(&env, "treasury_transfer_completed")
    );
    assert_eq!(
        events::event_treasury_cancelled(&env),
        Symbol::new(&env, "treasury_cancelled")
    );
    assert_eq!(
        events::event_set_max_distribute(&env),
        Symbol::new(&env, "set_max_distribute")
    );
    assert_eq!(
        events::event_distribute(&env),
        Symbol::new(&env, "distribute")
    );
    assert_eq!(
        events::event_batch_distribute(&env),
        Symbol::new(&env, "batch_distribute")
    );
    assert_eq!(
        events::event_distribute_started(&env),
        Symbol::new(&env, "distribute_started")
    );
    assert_eq!(
        events::event_distribute_completed(&env),
        Symbol::new(&env, "distribute_completed")
    );
    assert_eq!(events::event_upgraded(&env), Symbol::new(&env, "upgraded"));
    assert_eq!(
        events::event_admin_broadcast(&env),
        Symbol::new(&env, "admin_broadcast")
    );
    assert_eq!(
        events::event_emergency_drain_proposed(&env),
        Symbol::new(&env, "emergency_drain_proposed")
    );
    assert_eq!(
        events::event_emergency_drain_executed(&env),
        Symbol::new(&env, "emergency_drain_executed")
    );
    assert_eq!(
        events::event_emergency_drain_cancelled(&env),
        Symbol::new(&env, "emergency_drain_cancelled")
    );
}

#[test]
fn init_event_structure_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &usdc_addr);

    let events = env.events().all();
    let event = events.last().unwrap();

    // Verify event has correct topic structure [event_name, caller]
    let topics = &event.1;
    assert_eq!(topics.len(), 2);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    let topic1: Address = topics.get(1).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "init"));
    assert_eq!(topic1, admin);

    // Verify event data contains usdc address
    let data: Address = event.2.into_val(&env);
    assert_eq!(data, usdc_addr);
}

#[test]
fn distribute_lifecycle_events_have_stable_topics_and_payloads() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let developer = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(admin.clone());
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &asset.address());
    token::StellarAssetClient::new(&env, &asset.address()).mint(&pool_addr, &1_000_000);
    client.distribute(&admin, &developer, &1_000_000);

    let lifecycle_events: std::vec::Vec<_> = env
        .events()
        .all()
        .iter()
        .filter(|event| {
            event
                .1
                .get(0)
                .and_then(|value| Symbol::try_from_val(&env, &value).ok())
                .map(|topic| {
                    topic == Symbol::new(&env, "distribute_started")
                        || topic == Symbol::new(&env, "distribute_completed")
                })
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(lifecycle_events.len(), 2);
    assert_eq!(lifecycle_events[0].1.len(), 3);
    assert_eq!(
        Symbol::try_from_val(&env, &lifecycle_events[0].1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "distribute_started")
    );
    assert_eq!(
        Symbol::try_from_val(&env, &lifecycle_events[1].1.get(0).unwrap()).unwrap(),
        Symbol::new(&env, "distribute_completed")
    );

    for event in lifecycle_events {
        let caller = Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap();
        let recipient = Address::try_from_val(&env, &event.1.get(2).unwrap()).unwrap();
        let payload = events::DistributionLifecycleEvent::try_from_val(&env, &event.2).unwrap();

        assert_eq!(caller, admin);
        assert_eq!(recipient, developer);
        assert_eq!(payload.version, events::DISTRIBUTION_EVENT_VERSION);
        assert_eq!(payload.amount, 1_000_000);
        assert_eq!(payload.mode, events::DistributionMode::Single);
        assert_eq!(payload.batch_index, 0);
        assert_eq!(payload.batch_size, 1);
        assert_eq!(payload.ledger_sequence, env.ledger().sequence());
        assert_eq!(payload.timestamp, env.ledger().timestamp());
    }
}

#[test]
fn pause_guardian_events_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &usdc_addr);
    client.set_pause_guardian(&admin, &guardian);

    // Find pause_guardian_set event
    let events = env.events().all();
    let set_event = events
        .iter()
        .rev()
        .find(|e| {
            if let Ok(symbol) = Symbol::try_from_val(&env, &e.1.get(0).unwrap()) {
                symbol == Symbol::new(&env, "pause_guardian_set")
            } else {
                false
            }
        })
        .unwrap();

    let topics = &set_event.1;
    let caller: Address = topics.get(1).unwrap().into_val(&env);
    assert_eq!(caller, admin);
    let guardian_data: Address = set_event.2.into_val(&env);
    assert_eq!(guardian_data, guardian);
}

// ---------------------------------------------------------------------------
// Two-step admin rotation events (Issue #1163)
//
// `set_admin` nominates a successor; it must not announce an admin change
// that has not happened yet. `admin_changed` is published only when the
// nominee accepts, and a cancelled transfer must leave no change event behind.
// ---------------------------------------------------------------------------

/// One recorded event: `(contract, topics, data)`.
type RecordedEvent = (Address, soroban_sdk::Vec<Val>, Val);

/// Drop the events recorded so far, so the next [`recorded_events`] call only
/// contains the events published by the action under test.
fn drain_events(env: &Env) {
    env.events().all();
}

/// The events published since the last [`drain_events`] call.
fn recorded_events(env: &Env) -> std::vec::Vec<RecordedEvent> {
    env.events().all().iter().collect()
}

/// Count the events in `events` whose topic 0 matches `name`.
fn count_topic(env: &Env, events: &[RecordedEvent], name: &str) -> usize {
    let expected = Symbol::new(env, name);
    events
        .iter()
        .filter(|event| {
            event
                .1
                .get(0)
                .and_then(|value| Symbol::try_from_val(env, &value).ok())
                .map(|topic| topic == expected)
                .unwrap_or(false)
        })
        .count()
}

#[test]
fn set_admin_nomination_emits_events_without_admin_changed() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &usdc_addr);

    drain_events(&env);
    client.set_admin(&admin, &new_admin);
    let emitted = recorded_events(&env);

    assert_eq!(
        emitted.len(),
        1,
        "nomination must publish exactly one event, got {}",
        emitted.len()
    );

    let event = &emitted[0];
    let topic0: Symbol = event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "admin_transfer_started"));

    let topic1: Address = event.1.get(1).unwrap().into_val(&env);
    assert_eq!(topic1, admin, "nominator must be topic 1");

    let pending: Address = event.2.into_val(&env);
    assert_eq!(pending, new_admin, "nominee must be the event data");

    assert_eq!(
        count_topic(&env, &emitted, "admin_changed"),
        0,
        "set_admin must not announce an admin change before acceptance"
    );
}

#[test]
fn accept_admin_emits_events_recording_the_final_admin_change() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &usdc_addr);
    drain_events(&env);
    client.set_admin(&admin, &new_admin);
    drain_events(&env);

    client.claim_admin(&new_admin);
    let emitted = recorded_events(&env);

    assert_eq!(
        emitted.len(),
        2,
        "acceptance must publish admin_changed then admin_transfer_completed, got {}",
        emitted.len()
    );

    // 1. admin_changed — the actual change, recorded when it happens.
    let changed = &emitted[0];
    let changed_topic0: Symbol = changed.1.get(0).unwrap().into_val(&env);
    assert_eq!(changed_topic0, Symbol::new(&env, "admin_changed"));

    let changed_topic1: Address = changed.1.get(1).unwrap().into_val(&env);
    assert_eq!(
        changed_topic1, admin,
        "admin_changed topic 1 must be the outgoing admin"
    );

    let (previous, updated): (Address, Address) = changed.2.into_val(&env);
    assert_eq!(
        previous, admin,
        "admin_changed data[0] must be the old admin"
    );
    assert_eq!(
        updated, new_admin,
        "admin_changed data[1] must be the new admin"
    );

    // 2. admin_transfer_completed — emitted last, after the slot is updated.
    let completed = &emitted[1];
    let completed_topic0: Symbol = completed.1.get(0).unwrap().into_val(&env);
    assert_eq!(
        completed_topic0,
        Symbol::new(&env, "admin_transfer_completed")
    );

    let completed_topic1: Address = completed.1.get(1).unwrap().into_val(&env);
    assert_eq!(completed_topic1, new_admin);

    let _: () = completed.2.into_val(&env);

    assert_eq!(
        count_topic(&env, &emitted, "admin_changed"),
        1,
        "exactly one admin_changed event must be published for a completed transfer"
    );
}

#[test]
fn cancelled_admin_transfer_emits_events_without_admin_changed() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);

    client.init(&admin, &usdc_addr);
    drain_events(&env);
    client.set_admin(&admin, &new_admin);
    drain_events(&env);

    client.cancel_admin_transfer(&admin);
    let emitted = recorded_events(&env);

    assert_eq!(
        emitted.len(),
        1,
        "cancellation must publish exactly one event, got {}",
        emitted.len()
    );

    let event = &emitted[0];
    let topic0: Symbol = event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "admin_cancelled"));

    let topic1: Address = event.1.get(1).unwrap().into_val(&env);
    assert_eq!(topic1, admin);
    let topic2: Address = event.1.get(2).unwrap().into_val(&env);
    assert_eq!(topic2, new_admin);
    let _: () = event.2.into_val(&env);

    assert_eq!(
        count_topic(&env, &emitted, "admin_changed"),
        0,
        "a cancelled transfer must leave no admin_changed event behind"
    );
    assert_eq!(client.get_pending_admin(), None);
}
