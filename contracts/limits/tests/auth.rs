//! Auth-context tests for Callora limits-related entrypoints.
//!
//! Verifies that limits-related events on Settlement and RevenuePool correctly
//! capture signer identity under both `mock_all_auths()` (mock-auth harness)
//! and real `require_auth()` checks.
//!
//! # Properties under test
//!
//! 1. **Signer capture under mock-auth** — with `mock_all_auths()`, every
//!    caller address passed to a limits entrypoint appears as topic[1] in the
//!    emitted event.
//! 2. **Signer capture under real auth** — with real `require_auth()`, the
//!    authorized signer appears as topic[1]; a non-signing caller is rejected.
//! 3. **Topic shape stability** — topic[0] is the event name Symbol,
//!    topic[1] is the subject address.
//! 4. **Auth failure → no event** — when `require_auth` rejects a caller,
//!    no event is emitted.
//! 5. **Read-only entrypoints** — view functions are callable without auth.

extern crate std;

use callora_revenue_pool::{RevenuePool, RevenuePoolClient};
use callora_settlement::{CalloraSettlement, CalloraSettlementClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, Env, IntoVal, Symbol, TryFromVal, Val, Vec as SorobanVec};

// ---------------------------------------------------------------------------
// Helpers — decode event topics into Rust types for assertion.
// ---------------------------------------------------------------------------

/// Extract topic[0] as a Symbol.
fn topic_symbol(env: &Env, topics: &SorobanVec<Val>) -> Symbol {
    Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap()
}

/// Extract topic[N] as an Address.
fn topic_address(env: &Env, topics: &SorobanVec<Val>, n: u32) -> Address {
    Address::try_from_val(env, &topics.get(n).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Settlement test helpers
// ---------------------------------------------------------------------------

fn setup_settlement(env: &Env) -> (Address, CalloraSettlementClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &addr);
    client.init(&admin, &vault);
    (admin, client)
}

// ---------------------------------------------------------------------------
// Revenue pool test helpers
// ---------------------------------------------------------------------------

fn setup_pool(env: &Env) -> (Address, RevenuePoolClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(env, &addr);
    client.init(&admin, &usdc);
    (admin, client)
}

// ===========================================================================
// Tests — Settlement limits entrypoints
// ===========================================================================

/// Test 1: mock-auth captures signer for `set_daily_withdraw_cap`.
#[test]
fn mock_auth_captures_signer_in_daily_withdraw_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let (admin, client) = setup_settlement(&env);
    let developer = Address::generate(&env);

    client.set_daily_withdraw_cap(&admin, &developer, &10_000);

    let all_events = env.events().all();
    let cap_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(_, topics, _)| {
            topic_symbol(&env, topics) == Symbol::new(&env, "daily_withdraw_cap_changed")
        })
        .collect();

    assert_eq!(
        cap_events.len(),
        1,
        "must emit exactly 1 daily_withdraw_cap_changed event"
    );

    let (_emitter, topics, _data) = &cap_events[0];
    assert_eq!(
        topic_symbol(&env, &topics),
        Symbol::new(&env, "daily_withdraw_cap_changed"),
        "topic[0] must be the event name Symbol"
    );
    assert_eq!(
        topic_address(&env, &topics, 1),
        admin,
        "topic[1] must be the signer (admin) address under mock-auth"
    );
}

/// Test 2: real auth captures signer for `set_daily_withdraw_cap`.
#[test]
fn real_auth_captures_signer_in_daily_withdraw_cap() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let outsider = Address::generate(&env);

    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract_id);

    // Init with mock auth.
    env.mock_all_auths();
    client.init(&admin, &vault);
    // Drain init events.
    let _ = env.events().all();

    // Real auth: admin calls set_daily_withdraw_cap.
    let args: SorobanVec<Val> = (&admin, &developer, &10_000i128).into_val(&env);
    client
        .mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &admin,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_daily_withdraw_cap",
                args,
                sub_invokes: &[],
            },
        }])
        .set_daily_withdraw_cap(&admin, &developer, &10_000);

    let all_events = env.events().all();
    let cap_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(_, topics, _)| {
            topic_symbol(&env, topics) == Symbol::new(&env, "daily_withdraw_cap_changed")
        })
        .collect();

    assert_eq!(
        cap_events.len(),
        1,
        "must emit daily_withdraw_cap_changed under real auth"
    );

    let (_emitter, topics, _data) = &cap_events[0];
    assert_eq!(
        topic_symbol(&env, &topics),
        Symbol::new(&env, "daily_withdraw_cap_changed"),
        "topic[0] must be the event name Symbol under real auth"
    );
    assert_eq!(
        topic_address(&env, &topics, 1),
        admin,
        "topic[1] must be the authorized admin address under real auth"
    );

    // Outsider trying to set cap must be rejected.
    let fail_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_daily_withdraw_cap(&outsider, &developer, &5_000);
    }));
    assert!(
        fail_result.is_err(),
        "non-admin caller must be rejected under real auth"
    );
}

/// Test 3: mock-auth captures signer for `set_developer_min_balance`.
#[test]
fn mock_auth_captures_signer_in_developer_min_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let (admin, client) = setup_settlement(&env);
    let developer = Address::generate(&env);

    client.set_developer_min_balance(&admin, &developer, &1_000);

    let all_events = env.events().all();
    let min_balance_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(_, topics, _)| {
            topic_symbol(&env, topics) == Symbol::new(&env, "developer_min_balance_changed")
        })
        .collect();

    assert_eq!(
        min_balance_events.len(),
        1,
        "must emit exactly 1 developer_min_balance_changed event"
    );

    let (_emitter, topics, _data) = &min_balance_events[0];
    assert_eq!(
        topic_symbol(&env, &topics),
        Symbol::new(&env, "developer_min_balance_changed"),
        "topic[0] must be the event name Symbol"
    );
    assert_eq!(
        topic_address(&env, &topics, 1),
        developer,
        "topic[1] must be the developer address for min_balance events"
    );
}

/// Test 4: auth failure path emits no event.
#[test]
fn auth_failure_on_limits_emits_no_event() {
    let env = Env::default();
    env.mock_all_auths();

    let (_admin, client) = setup_settlement(&env);
    let developer = Address::generate(&env);
    let intruder = Address::generate(&env);

    // Drain init events.
    let _ = env.events().all();

    // Intruder tries to set daily withdraw cap.
    let result = client.try_set_daily_withdraw_cap(&intruder, &developer, &5_000);
    assert!(result.is_err(), "intruder must be rejected");

    // No events should have been emitted by the failed call.
    let events_after = env.events().all();
    assert_eq!(
        events_after.len(),
        0,
        "failed auth must not emit any events"
    );
}

/// Test 5: read-only limits views work without auth.
#[test]
fn read_only_limits_views_work_without_auth() {
    let env = Env::default();
    let (_admin, client) = setup_settlement(&env);
    let developer = Address::generate(&env);

    env.set_auths(&[]);
    let min_balance = client.get_developer_min_balance(&developer);
    assert_eq!(min_balance, 0, "unset min balance defaults to 0");

    let daily_cap = client.get_daily_withdraw_cap(&developer);
    assert_eq!(daily_cap, 0, "unset daily cap defaults to 0 (unlimited)");

    let withdrawal_today = client.get_withdrawal_today(&developer);
    assert_eq!(withdrawal_today, 0, "no withdrawal today defaults to 0");
}

// ===========================================================================
// Tests — Revenue pool distribute cap
// ===========================================================================

/// Test 6: mock-auth captures signer for `set_max_distribute`.
#[test]
fn mock_auth_captures_signer_in_set_max_distribute() {
    let env = Env::default();
    env.mock_all_auths();

    let (admin, client) = setup_pool(&env);

    client.set_max_distribute(&admin, &42_000);

    let all_events = env.events().all();
    let max_dist_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(_, topics, _)| {
            topic_symbol(&env, topics) == Symbol::new(&env, "set_max_distribute")
        })
        .collect();

    assert_eq!(
        max_dist_events.len(),
        1,
        "must emit exactly 1 set_max_distribute event"
    );

    let (_emitter, topics, _data) = &max_dist_events[0];
    assert_eq!(
        topic_symbol(&env, &topics),
        Symbol::new(&env, "set_max_distribute"),
        "topic[0] must be the event name Symbol"
    );
    assert_eq!(
        topic_address(&env, &topics, 1),
        admin,
        "topic[1] must be the signer (admin) address under mock-auth"
    );
}

/// Test 7: real auth captures signer for `set_max_distribute`.
#[test]
fn real_auth_captures_signer_in_set_max_distribute() {
    let env = Env::default();

    let admin = Address::generate(&env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_id = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &contract_id);

    // Init with mock auth.
    env.mock_all_auths();
    client.init(&admin, &usdc);
    // Drain init events.
    let _ = env.events().all();

    // Real auth: admin calls set_max_distribute.
    let args: SorobanVec<Val> = (&admin, &42_000i128).into_val(&env);
    client
        .mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &admin,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_max_distribute",
                args,
                sub_invokes: &[],
            },
        }])
        .set_max_distribute(&admin, &42_000);

    let all_events = env.events().all();
    let max_dist_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(_, topics, _)| {
            topic_symbol(&env, topics) == Symbol::new(&env, "set_max_distribute")
        })
        .collect();

    assert_eq!(
        max_dist_events.len(),
        1,
        "must emit set_max_distribute under real auth"
    );

    let (_emitter, topics, _data) = &max_dist_events[0];
    assert_eq!(
        topic_symbol(&env, &topics),
        Symbol::new(&env, "set_max_distribute"),
        "topic[0] must be the event name Symbol under real auth"
    );
    assert_eq!(
        topic_address(&env, &topics, 1),
        admin,
        "topic[1] must be the authorized admin address under real auth"
    );

    // Outsider trying to set max distribute must be rejected.
    let outsider = Address::generate(&env);
    let fail_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_max_distribute(&outsider, &10_000);
    }));
    assert!(
        fail_result.is_err(),
        "non-admin caller must be rejected under real auth for set_max_distribute"
    );
}

/// Test 8: read-only revenue pool limits view works without auth.
#[test]
fn read_only_max_distribute_works_without_auth() {
    let env = Env::default();
    let (_admin, client) = setup_pool(&env);

    env.set_auths(&[]);
    let max_dist = client.get_max_distribute();
    assert_eq!(max_dist, i128::MAX, "default max distribute is i128::MAX");
}
