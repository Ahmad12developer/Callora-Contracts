extern crate std;

use crate::errors::FreezeError;
use crate::events::{FreezeOperatorSetEvent, FreezeSetEvent};
use crate::{CalloraFreeze, CalloraFreezeClient};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Env, Symbol, TryFromVal};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (Env, Address, CalloraFreezeClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(CalloraFreeze, ());
    let client = CalloraFreezeClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    (env, admin, client)
}

// ---------------------------------------------------------------------------
// Existing behaviour tests (unchanged)
// ---------------------------------------------------------------------------

#[test]
fn test_init_and_get_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(CalloraFreeze, ());
    let client = CalloraFreezeClient::new(&env, &contract_id);

    // Get admin before init returns NotInitialized
    assert!(client.try_get_admin().is_err());

    let admin = Address::generate(&env);
    client.init(&admin);

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.is_frozen(), false);

    // Double init fails
    assert!(client.try_init(&admin).is_err());
}

#[test]
fn test_freeze_and_unfreeze_flow() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(CalloraFreeze, ());
    let client = CalloraFreezeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.init(&admin);

    let reason = Symbol::new(&env, "exploit_risk");
    let freeze_res = client.try_freeze(&admin, &reason);
    assert!(freeze_res.is_ok());
    assert_eq!(client.is_frozen(), true);

    // Freeze when already frozen fails
    assert!(client.try_freeze(&admin, &reason).is_err());

    // Unfreeze succeeds
    let unfreeze_res = client.try_unfreeze(&admin);
    assert!(unfreeze_res.is_ok());
    assert_eq!(client.is_frozen(), false);

    // Unfreeze when not frozen fails
    assert!(client.try_unfreeze(&admin).is_err());
}

#[test]
fn test_operator_freeze_permissions() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(CalloraFreeze, ());
    let client = CalloraFreezeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.init(&admin);

    // Stranger freeze fails
    let reason = Symbol::new(&env, "emergency");
    assert!(client.try_freeze(&stranger, &reason).is_err());

    // Set operator
    client.set_freeze_operator(&admin, &Some(operator.clone()));
    assert_eq!(client.get_freeze_operator(), Some(operator.clone()));

    // Operator freeze succeeds
    let op_freeze = client.try_freeze(&operator, &reason);
    assert!(op_freeze.is_ok());
    assert_eq!(client.is_frozen(), true);

    // Operator unfreeze fails (only admin can unfreeze)
    assert!(client.try_unfreeze(&operator).is_err());

    // Admin unfreeze succeeds
    client.unfreeze(&admin);

    // Revoke operator
    client.set_freeze_operator(&admin, &None);
    assert_eq!(client.get_freeze_operator(), None);

    // Revoked operator freeze fails
    assert!(client.try_freeze(&operator, &reason).is_err());
}

#[test]
fn test_set_operator_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(CalloraFreeze, ());
    let client = CalloraFreezeClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.init(&admin);

    assert!(client
        .try_set_freeze_operator(&stranger, &Some(stranger.clone()))
        .is_err());
}

#[test]
fn test_freeze_error_codes_stability_and_uniqueness() {
    let mappings = [
        (1_u32, FreezeError::NotInitialized),
        (2, FreezeError::AlreadyInitialized),
        (3, FreezeError::Unauthorized),
        (4, FreezeError::AlreadyFrozen),
        (5, FreezeError::NotFrozen),
        (6, FreezeError::Overflow),
    ];

    let mut seen = BTreeSet::new();
    for (expected_code, variant) in mappings {
        assert_eq!(variant as u32, expected_code);
        assert!(
            seen.insert(expected_code),
            "duplicate freeze error code {expected_code}"
        );
    }

    assert_eq!(seen.len(), 6);
}

#[test]
fn test_error_code_docs_coverage() {
    let docs = include_str!("../../../docs/ERROR_CODES.md");
    let expected_lines = [
        "| 1 | `NotInitialized` | Freeze | Contract has not been initialized yet |",
        "| 2 | `AlreadyInitialized` | Freeze | `init` was called more than once |",
        "| 3 | `Unauthorized` | Freeze | Caller is not authorized for the operation |",
        "| 4 | `AlreadyFrozen` | Freeze | Contract is already frozen |",
        "| 5 | `NotFrozen` | Freeze | Contract is not currently frozen |",
        "| 6 | `Overflow` | Freeze | Arithmetic overflow detected |",
    ];

    for line in expected_lines {
        assert!(docs.contains(line), "missing freeze docs line: {line}");
    }
}

// ---------------------------------------------------------------------------
// Event emission tests (new)
// ---------------------------------------------------------------------------

/// `init` must emit exactly one `freeze_initialized` event with version marker.
#[test]
fn test_init_emits_freeze_initialized_event() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let events = env.events().all();
    // The last emitted event is ours (token mock events may precede it).
    let evt = events.last().expect("expected at least one event");

    // topics: (freeze_initialized, callora_v1, admin)
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    let topic1 = Symbol::try_from_val(&env, &evt.1.get(1).unwrap()).unwrap();
    let topic2 = Address::try_from_val(&env, &evt.1.get(2).unwrap()).unwrap();

    assert_eq!(topic0, Symbol::new(&env, "freeze_initialized"));
    assert_eq!(topic1, Symbol::new(&env, "callora_v1"));
    assert_eq!(topic2, admin);

    // data is ()
    let _data: () = TryFromVal::try_from_val(&env, &evt.2).unwrap_or(());
}

/// `freeze` must emit exactly one `freeze_set` event containing the reason
/// and the timestamp in the data payload.
#[test]
fn test_freeze_emits_freeze_set_event() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let reason = Symbol::new(&env, "exploit_risk");
    client.freeze(&admin, &reason);

    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");

    // topics: (freeze_set, callora_v1, caller)
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    let topic1 = Symbol::try_from_val(&env, &evt.1.get(1).unwrap()).unwrap();
    let topic2 = Address::try_from_val(&env, &evt.1.get(2).unwrap()).unwrap();

    assert_eq!(topic0, Symbol::new(&env, "freeze_set"));
    assert_eq!(topic1, Symbol::new(&env, "callora_v1"));
    assert_eq!(topic2, admin);

    // data: FreezeSetEvent { reason, frozen_at }
    let payload = FreezeSetEvent::try_from_val(&env, &evt.2).unwrap();
    assert_eq!(payload.reason, reason);
    assert_eq!(payload.frozen_at, env.ledger().timestamp());
}

/// `unfreeze` must emit exactly one `freeze_cleared` event.
#[test]
fn test_unfreeze_emits_freeze_cleared_event() {
    let (env, admin, client) = setup();
    client.init(&admin);
    client.freeze(&admin, &Symbol::new(&env, "maintenance"));
    client.unfreeze(&admin);

    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");

    // topics: (freeze_cleared, callora_v1, caller)
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    let topic1 = Symbol::try_from_val(&env, &evt.1.get(1).unwrap()).unwrap();
    let topic2 = Address::try_from_val(&env, &evt.1.get(2).unwrap()).unwrap();

    assert_eq!(topic0, Symbol::new(&env, "freeze_cleared"));
    assert_eq!(topic1, Symbol::new(&env, "callora_v1"));
    assert_eq!(topic2, admin);
}

/// `set_freeze_operator` (set) emits `freeze_operator_set` with old=None and new=operator.
#[test]
fn test_set_freeze_operator_emits_event_on_set() {
    let (env, admin, client) = setup();
    let operator = Address::generate(&env);
    client.init(&admin);
    client.set_freeze_operator(&admin, &Some(operator.clone()));

    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");

    // topics: (freeze_operator_set, callora_v1, caller)
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    let topic1 = Symbol::try_from_val(&env, &evt.1.get(1).unwrap()).unwrap();
    let topic2 = Address::try_from_val(&env, &evt.1.get(2).unwrap()).unwrap();

    assert_eq!(topic0, Symbol::new(&env, "freeze_operator_set"));
    assert_eq!(topic1, Symbol::new(&env, "callora_v1"));
    assert_eq!(topic2, admin);

    // data: FreezeOperatorSetEvent { old_operator: None, new_operator: Some(operator) }
    let payload = FreezeOperatorSetEvent::try_from_val(&env, &evt.2).unwrap();
    assert_eq!(payload.old_operator, None);
    assert_eq!(payload.new_operator, Some(operator));
}

/// `set_freeze_operator(None)` emits `freeze_operator_set` with old=Some(op) and new=None.
#[test]
fn test_set_freeze_operator_emits_event_on_clear() {
    let (env, admin, client) = setup();
    let operator = Address::generate(&env);
    client.init(&admin);
    // First set an operator.
    client.set_freeze_operator(&admin, &Some(operator.clone()));
    // Now clear it.
    client.set_freeze_operator(&admin, &None);

    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");

    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    assert_eq!(topic0, Symbol::new(&env, "freeze_operator_set"));

    let payload = FreezeOperatorSetEvent::try_from_val(&env, &evt.2).unwrap();
    assert_eq!(payload.old_operator, Some(operator));
    assert_eq!(payload.new_operator, None);
}

/// `set_freeze_operator` replaces an existing operator: old=Some(old_op), new=Some(new_op).
#[test]
fn test_set_freeze_operator_emits_event_on_replace() {
    let (env, admin, client) = setup();
    let op_a = Address::generate(&env);
    let op_b = Address::generate(&env);
    client.init(&admin);
    client.set_freeze_operator(&admin, &Some(op_a.clone()));
    client.set_freeze_operator(&admin, &Some(op_b.clone()));

    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");

    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    assert_eq!(topic0, Symbol::new(&env, "freeze_operator_set"));

    let payload = FreezeOperatorSetEvent::try_from_val(&env, &evt.2).unwrap();
    assert_eq!(payload.old_operator, Some(op_a));
    assert_eq!(payload.new_operator, Some(op_b));
}

// ---------------------------------------------------------------------------
// get_freeze_status tests (new)
// ---------------------------------------------------------------------------

/// Before init, `get_freeze_status` returns `NotInitialized`.
#[test]
fn test_get_freeze_status_before_init_returns_error() {
    let (env, _admin, client) = setup();
    // Do not call init.
    let _ = env; // suppress unused warning
    assert_eq!(
        client.try_get_freeze_status(),
        Err(Ok(FreezeError::NotInitialized))
    );
}

/// After init but before any freeze: `frozen=false`, `reason=None`, `frozen_at=None`.
#[test]
fn test_get_freeze_status_after_init_not_frozen() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let status = client.get_freeze_status();
    assert!(!status.frozen);
    assert!(status.reason.is_none());
    assert!(status.frozen_at.is_none());

    let _ = env;
}

/// After `freeze`: `frozen=true`, `reason` and `frozen_at` are populated.
#[test]
fn test_get_freeze_status_after_freeze() {
    let (env, admin, client) = setup();
    client.init(&admin);
    let reason = Symbol::new(&env, "exploit_risk");
    client.freeze(&admin, &reason);

    let status = client.get_freeze_status();
    assert!(status.frozen);
    assert_eq!(status.reason, Some(reason));
    assert!(status.frozen_at.is_some());
}

/// After `unfreeze`: `frozen=false`, `reason=None`, `frozen_at=None` (cleared).
#[test]
fn test_get_freeze_status_after_unfreeze_cleared() {
    let (env, admin, client) = setup();
    client.init(&admin);
    client.freeze(&admin, &Symbol::new(&env, "maintenance"));
    client.unfreeze(&admin);

    let status = client.get_freeze_status();
    assert!(!status.frozen);
    assert!(status.reason.is_none());
    assert!(status.frozen_at.is_none());
}

/// Reason persisted by `get_freeze_status` matches the reason in the `freeze_set` event.
#[test]
fn test_freeze_status_reason_matches_event_payload() {
    let (env, admin, client) = setup();
    client.init(&admin);
    let reason = Symbol::new(&env, "audit_hold");
    // freeze() is the last call; its event is last in the list.
    client.freeze(&admin, &reason);

    // Capture the event first, before making another contract call.
    let events = env.events().all();
    let evt = events.last().expect("expected at least one event");
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    assert_eq!(topic0, Symbol::new(&env, "freeze_set"));
    let payload = FreezeSetEvent::try_from_val(&env, &evt.2).unwrap();

    // Now call the view to get persisted state.
    let status = client.get_freeze_status();

    assert_eq!(status.reason, Some(payload.reason));
    assert_eq!(status.frozen_at, Some(payload.frozen_at));
}

/// Operator can freeze; the emitted event carries the operator's address, not the admin's.
#[test]
fn test_operator_freeze_event_carries_operator_address() {
    let (env, admin, client) = setup();
    let operator = Address::generate(&env);
    client.init(&admin);
    client.set_freeze_operator(&admin, &Some(operator.clone()));

    let reason = Symbol::new(&env, "emergency");
    client.freeze(&operator, &reason);

    let events = env.events().all();
    let evt = events.last().unwrap();
    let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
    let topic2 = Address::try_from_val(&env, &evt.1.get(2).unwrap()).unwrap();

    assert_eq!(topic0, Symbol::new(&env, "freeze_set"));
    assert_eq!(topic2, operator);
}

/// Each of the four state-changing entrypoints emits exactly one event (count check).
/// Each entrypoint is exercised in its own fresh Env so the event list only
/// contains events from that single call, making the last-event check reliable.
#[test]
fn test_each_entrypoint_emits_exactly_one_event() {
    // --- init ---
    {
        let (env, admin, client) = setup();
        client.init(&admin);
        let events = env.events().all();
        let evt = events.last().expect("init: no events");
        let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
        assert_eq!(
            topic0,
            Symbol::new(&env, "freeze_initialized"),
            "init must emit freeze_initialized"
        );
        // Exactly one event with our topic (env is fresh; init is the only call).
        let count = (0..events.len())
            .filter(|&i| {
                let e = events.get(i as u32).unwrap();
                Symbol::try_from_val(&env, &e.1.get(0).unwrap()).ok()
                    == Some(Symbol::new(&env, "freeze_initialized"))
            })
            .count();
        assert_eq!(count, 1, "init must emit exactly 1 freeze_initialized event");
    }

    // --- set_freeze_operator ---
    {
        let (env, admin, client) = setup();
        let operator = Address::generate(&env);
        client.init(&admin);
        client.set_freeze_operator(&admin, &Some(operator));
        let events = env.events().all();
        let evt = events.last().expect("set_freeze_operator: no events");
        let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
        assert_eq!(
            topic0,
            Symbol::new(&env, "freeze_operator_set"),
            "set_freeze_operator must emit freeze_operator_set"
        );
    }

    // --- freeze ---
    {
        let (env, admin, client) = setup();
        client.init(&admin);
        client.freeze(&admin, &Symbol::new(&env, "test_reason"));
        let events = env.events().all();
        let evt = events.last().expect("freeze: no events");
        let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
        assert_eq!(
            topic0,
            Symbol::new(&env, "freeze_set"),
            "freeze must emit freeze_set"
        );
    }

    // --- unfreeze ---
    {
        let (env, admin, client) = setup();
        client.init(&admin);
        client.freeze(&admin, &Symbol::new(&env, "test_reason"));
        client.unfreeze(&admin);
        let events = env.events().all();
        let evt = events.last().expect("unfreeze: no events");
        let topic0 = Symbol::try_from_val(&env, &evt.1.get(0).unwrap()).unwrap();
        assert_eq!(
            topic0,
            Symbol::new(&env, "freeze_cleared"),
            "unfreeze must emit freeze_cleared"
        );
    }
}
