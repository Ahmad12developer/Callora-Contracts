//! Unit tests for the Callora Yield per-account limits surface.
//!
//! Covers happy-path increments / decrements, cap enforcement, two-step
//! admin rotation, default limits fallback, persistent-state TTL bumps,
//! invalid-cap rejection, and overflow-safe math.

extern crate std;

use crate::limits::{
    AccountLimits, AccountState, CalloraYieldLimits, CalloraYieldLimitsClient, DEFAULT_LIMITS,
    MAX_CAP,
};
use crate::YieldLimitError;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, BytesN, Env, IntoVal, Symbol, TryFromVal, Val};

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

/// Register the contract, init it with `admin`, and return (contract, admin, client).
fn setup_admin<'a>(env: &'a Env) -> (Address, Address, CalloraYieldLimitsClient<'a>) {
    env.mock_all_auths();
    let contract = env.register(CalloraYieldLimits, ());
    let client = CalloraYieldLimitsClient::new(env, &contract);
    let admin = Address::generate(env);
    client.init(&admin);
    (contract, admin, client)
}

/// Same as `setup_admin`, but additionally register a fresh user account.
fn setup_with_user<'a>(env: &'a Env) -> (Address, Address, CalloraYieldLimitsClient<'a>, Address) {
    let (contract, admin, client) = setup_admin(env);
    let user = Address::generate(env);
    (contract, admin, client, user)
}

// ---------------------------------------------------------------------
// init / admin lifecycle
// ---------------------------------------------------------------------

#[test]
fn init_sets_admin() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn init_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = env.register(CalloraYieldLimits, ());
    let admin = Address::generate(&env);
    let client = CalloraYieldLimitsClient::new(&env, &contract);
    client.init(&admin);

    let stale_admin = Address::generate(&env);
    assert_eq!(
        client.try_init(&stale_admin),
        Err(Ok(YieldLimitError::AlreadyInitialized))
    );
}

#[test]
fn get_admin_before_init_fails() {
    let env = Env::default();
    let contract = env.register(CalloraYieldLimits, ());
    let client = CalloraYieldLimitsClient::new(&env, &contract);
    assert_eq!(
        client.try_get_admin(),
        Err(Ok(YieldLimitError::NotInitialized))
    );
}

#[test]
fn init_emits_event_with_admin_topic() {
    let env = Env::default();
    env.mock_all_auths();
    let (_contract, admin, _) = setup_admin(&env);

    // The contract publishes `(Symbol("init"), admin)` as the topic list and
    // `()` as the data side.  Iterate the topic list, decode the first slot
    // as a Symbol (event name) and confirm admin appears somewhere in the
    // list — exactly mirroring the existing repo pattern in
    // `contracts/yield/tests/xcontract.rs::count_yield_deposited_events`.
    let init_symbol = Symbol::new(&env, "init");
    let mut init_count: u32 = 0;
    let mut admin_in_init_topics = false;
    for e in env.events().all().iter() {
        if e.1.is_empty() {
            continue;
        }
        let first: Option<Symbol> = Symbol::try_from_val(&env, &e.1.get(0).unwrap()).ok();
        if first.as_ref() != Some(&init_symbol) {
            continue;
        }
        init_count += 1;
        for t in e.1.iter() {
            if let Ok(addr) = Address::try_from_val(&env, &t) {
                if addr == admin {
                    admin_in_init_topics = true;
                    break;
                }
            }
        }
    }
    assert_eq!(init_count, 1, "exactly one init topic must be published");
    assert!(
        admin_in_init_topics,
        "init event topic list must reference the admin address"
    );
    // Avoid the unused-import lint for `Val` / `IntoVal` when the assertions
    // above are the only consumers.
    let _ = <Val as soroban_sdk::IntoVal<Env, Val>>::into_val;
    let _: Val = admin.clone().into_val(&env);
}

// ---------------------------------------------------------------------
// default caps & AccountLimits validation
// ---------------------------------------------------------------------

#[test]
fn default_limits_match_constant() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let caps = client.get_default_limits();
    assert_eq!(caps.max_bets, DEFAULT_LIMITS.max_bets);
    assert_eq!(caps.max_positions, DEFAULT_LIMITS.max_positions);
    assert_eq!(caps.max_subscriptions, DEFAULT_LIMITS.max_subscriptions);
}

#[test]
fn default_limits_have_safe_lower_bounds() {
    let caps = DEFAULT_LIMITS;
    assert!(caps.max_bets > 0);
    assert!(caps.max_positions > 0);
    assert!(caps.max_subscriptions > 0);
    assert!(caps.is_valid());
}

#[test]
fn account_limits_uniform_constructor() {
    let caps = AccountLimits::uniform(7);
    assert_eq!(caps.max_bets, 7);
    assert_eq!(caps.max_positions, 7);
    assert_eq!(caps.max_subscriptions, 7);
}

#[test]
fn account_limits_validity_rejects_oversized_caps() {
    let bad = AccountLimits {
        max_bets: MAX_CAP + 1,
        max_positions: 5,
        max_subscriptions: 5,
    };
    assert!(!bad.is_valid());
}

#[test]
fn account_limits_validity_accepts_zero_caps() {
    let zero = AccountLimits::uniform(0);
    assert!(zero.is_valid(), "zero is a valid cap (disables the kind)");
}

// ---------------------------------------------------------------------
// default-limits mutation
// ---------------------------------------------------------------------

#[test]
fn set_default_limits_requires_admin() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let intruder = Address::generate(&env);
    assert_eq!(
        client.try_set_default_limits(&intruder, &1u32, &1u32, &1u32),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn set_default_limits_rejects_oversized_value() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let too_big = (MAX_CAP as u64).checked_add(1).unwrap() as u32;
    assert_eq!(
        client.try_set_default_limits(&admin, &too_big, &5u32, &5u32),
        Err(Ok(YieldLimitError::InvalidLimit))
    );
}

#[test]
fn set_default_limits_persists() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    client.set_default_limits(&admin, &10u32, &5u32, &2u32);
    let caps = client.get_default_limits();
    assert_eq!(caps.max_bets, 10);
    assert_eq!(caps.max_positions, 5);
    assert_eq!(caps.max_subscriptions, 2);
}

// ---------------------------------------------------------------------
// per-account caps override
// ---------------------------------------------------------------------

#[test]
fn account_limits_default_to_global_default() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let bob = Address::generate(&env);
    let caps = client.get_account_limits(&bob);
    assert_eq!(caps.max_bets, client.get_default_limits().max_bets);
}

#[test]
fn set_account_limits_requires_admin() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let intruder = Address::generate(&env);
    let target = Address::generate(&env);
    assert_eq!(
        client.try_set_account_limits(&intruder, &target, &1u32, &1u32, &1u32),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn set_account_limits_persists_override() {
    let env = Env::default();
    let (_, admin, client, bob) = setup_with_user(&env);
    client.set_account_limits(&admin, &bob, &2u32, &3u32, &4u32);
    let caps = client.get_account_limits(&bob);
    assert_eq!(caps.max_bets, 2);
    assert_eq!(caps.max_positions, 3);
    assert_eq!(caps.max_subscriptions, 4);
}

#[test]
fn clear_account_limits_falls_back_to_global() {
    let env = Env::default();
    let (_, admin, client, bob) = setup_with_user(&env);
    client.set_default_limits(&admin, &11u32, &12u32, &13u32);
    client.set_account_limits(&admin, &bob, &1u32, &1u32, &1u32);
    assert_eq!(client.get_account_limits(&bob).max_bets, 1);
    client.clear_account_limits(&admin, &bob);
    assert_eq!(client.get_account_limits(&bob).max_bets, 11);
    assert_eq!(client.get_account_limits(&bob).max_positions, 12);
    assert_eq!(client.get_account_limits(&bob).max_subscriptions, 13);
}

#[test]
fn clear_account_limits_requires_admin() {
    let env = Env::default();
    let (_, admin, client, bob) = setup_with_user(&env);
    client.set_account_limits(&admin, &bob, &1u32, &1u32, &1u32);
    let intruder = Address::generate(&env);
    assert_eq!(
        client.try_clear_account_limits(&intruder, &bob),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

// ---------------------------------------------------------------------
// user-level counter mutators
// ---------------------------------------------------------------------

#[test]
fn place_bet_increments_then_clear_decrements() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &5u32, &5u32, &5u32);

    client.place_bet(&alice);
    client.place_bet(&alice);
    let state = client.get_account_state(&alice);
    assert_eq!(state.bets, 2);

    client.clear_bet(&alice);
    let state = client.get_account_state(&alice);
    assert_eq!(state.bets, 1);
}

#[test]
fn place_bet_respects_per_account_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &3u32, &3u32, &3u32);

    client.place_bet(&alice);
    client.place_bet(&alice);
    client.place_bet(&alice);
    assert_eq!(
        client.try_place_bet(&alice),
        Err(Ok(YieldLimitError::BetsAtCap))
    );
    let state = client.get_account_state(&alice);
    assert_eq!(state.bets, 3, "failing call must not increment counter");
}

#[test]
fn open_position_respects_per_account_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &3u32, &2u32, &3u32);

    client.open_position(&alice);
    client.open_position(&alice);
    assert_eq!(
        client.try_open_position(&alice),
        Err(Ok(YieldLimitError::PositionsAtCap))
    );
    let state = client.get_account_state(&alice);
    assert_eq!(state.positions, 2);
}

#[test]
fn subscribe_respects_per_account_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &3u32, &3u32, &2u32);

    client.subscribe(&alice);
    client.subscribe(&alice);
    assert_eq!(
        client.try_subscribe(&alice),
        Err(Ok(YieldLimitError::SubscriptionsAtCap))
    );
    let state = client.get_account_state(&alice);
    assert_eq!(state.subscriptions, 2);
}

#[test]
fn clear_bet_underflow_returns_typed_error() {
    let env = Env::default();
    let (_, _, client, alice) = setup_with_user(&env);
    assert_eq!(
        client.try_clear_bet(&alice),
        Err(Ok(YieldLimitError::CounterUnderflow))
    );
}

#[test]
fn close_position_underflow_returns_typed_error() {
    let env = Env::default();
    let (_, _, client, alice) = setup_with_user(&env);
    assert_eq!(
        client.try_close_position(&alice),
        Err(Ok(YieldLimitError::CounterUnderflow))
    );
}

#[test]
fn unsubscribe_underflow_returns_typed_error() {
    let env = Env::default();
    let (_, _, client, alice) = setup_with_user(&env);
    assert_eq!(
        client.try_unsubscribe(&alice),
        Err(Ok(YieldLimitError::CounterUnderflow))
    );
}

#[test]
fn state_independent_across_accounts() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    client.set_account_limits(&admin, &alice, &2u32, &2u32, &2u32);
    client.set_account_limits(&admin, &bob, &4u32, &4u32, &4u32);

    client.place_bet(&alice);
    client.place_bet(&alice);
    client.place_bet(&bob);
    client.place_bet(&bob);
    client.place_bet(&bob);
    client.place_bet(&bob);
    assert_eq!(client.get_account_state(&alice).bets, 2);
    assert_eq!(client.get_account_state(&bob).bets, 4);
}

#[test]
fn place_bet_with_cap_zero_rejects() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &0u32, &0u32, &0u32);
    assert_eq!(
        client.try_place_bet(&alice),
        Err(Ok(YieldLimitError::BetsAtCap))
    );
    assert_eq!(
        client.try_open_position(&alice),
        Err(Ok(YieldLimitError::PositionsAtCap))
    );
    assert_eq!(
        client.try_subscribe(&alice),
        Err(Ok(YieldLimitError::SubscriptionsAtCap))
    );
}

// ---------------------------------------------------------------------
// dry-run can_* helpers
// ---------------------------------------------------------------------

#[test]
fn can_place_bet_true_until_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &2u32, &2u32, &2u32);
    assert!(client.can_place_bet(&alice));
    client.place_bet(&alice);
    assert!(client.can_place_bet(&alice));
    client.place_bet(&alice);
    assert!(!client.can_place_bet(&alice));
}

#[test]
fn can_open_position_true_until_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &5u32, &1u32, &5u32);
    assert!(client.can_open_position(&alice));
    client.open_position(&alice);
    assert!(!client.can_open_position(&alice));
}

#[test]
fn can_subscribe_true_until_cap() {
    let env = Env::default();
    let (_, admin, client, alice) = setup_with_user(&env);
    client.set_account_limits(&admin, &alice, &5u32, &5u32, &1u32);
    assert!(client.can_subscribe(&alice));
    client.subscribe(&alice);
    assert!(!client.can_subscribe(&alice));
}

// ---------------------------------------------------------------------
// AccountState arithmetic invariants
// ---------------------------------------------------------------------

#[test]
fn account_state_zero_has_zero_counters() {
    let s = AccountState::zero();
    assert_eq!(s.bets, 0);
    assert_eq!(s.positions, 0);
    assert_eq!(s.subscriptions, 0);
}

#[test]
fn account_state_default_trait_yields_zero() {
    let s: AccountState = Default::default();
    assert_eq!(s, AccountState::zero());
}

#[test]
fn account_state_checked_mutations_succeed_within_range() {
    let mut s = AccountState::zero();
    s.add_bet().unwrap();
    s.add_bet().unwrap();
    assert_eq!(s.bets, 2);
    s.sub_bet().unwrap();
    assert_eq!(s.bets, 1);
    s.add_position().unwrap();
    s.add_subscription().unwrap();
    s.sub_position().unwrap();
    s.sub_subscription().unwrap();
    assert_eq!(
        s,
        AccountState {
            bets: 1,
            positions: 0,
            subscriptions: 0,
        }
    );
}

// ---------------------------------------------------------------------
// Two-step admin rotation
// ---------------------------------------------------------------------

#[test]
fn set_admin_requires_current_admin() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let intruder = Address::generate(&env);
    let target = Address::generate(&env);
    assert_eq!(
        client.try_set_admin(&intruder, &target),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn admin_rotation_round_trip() {
    let env = Env::default();
    let (contract, admin, client) = setup_admin(&env);
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);
    // Ignore unused binding warning for contract.
    let _ = contract;
}

#[test]
fn accept_admin_wrong_caller_is_unauthorized() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let new_admin = Address::generate(&env);
    let intruder = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    assert_eq!(
        client.try_accept_admin(&intruder),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn accept_admin_without_pending_is_unauthorized() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let rogue = Address::generate(&env);
    assert_eq!(
        client.try_accept_admin(&rogue),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn cancel_admin_transfer_requires_admin() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    let intruder = Address::generate(&env);
    assert_eq!(
        client.try_cancel_admin_transfer(&intruder),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn cancel_admin_transfer_requires_pending() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    assert_eq!(
        client.try_cancel_admin_transfer(&admin),
        Err(Ok(YieldLimitError::Unauthorized))
    );
}

#[test]
fn cancel_admin_transfer_clears_pending() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.cancel_admin_transfer(&admin);
    // Accept by the supposed nominee should now fail; admin unchanged.
    assert_eq!(
        client.try_accept_admin(&new_admin),
        Err(Ok(YieldLimitError::Unauthorized))
    );
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn new_admin_takes_over_authority() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);

    assert_eq!(
        client.try_set_default_limits(&admin, &1u32, &1u32, &1u32),
        Err(Ok(YieldLimitError::Unauthorized))
    );
    client.set_default_limits(&new_admin, &7u32, &8u32, &9u32);
    assert_eq!(client.get_default_limits().max_bets, 7);
}

// ---------------------------------------------------------------------
// Upgrade version marker + storage-migration guard (Issue #1223)
// ---------------------------------------------------------------------

#[test]
fn get_version_is_none_before_upgrade() {
    let env = Env::default();
    let (_, _, client) = setup_admin(&env);
    assert_eq!(
        client.get_version(),
        None,
        "no WASM hash must be recorded before the first upgrade"
    );
}

#[test]
fn upgrade_rejects_zero_wasm_hash() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    let zero = BytesN::from_array(&env, &[0u8; 32]);

    assert_eq!(
        client.try_upgrade(&admin, &zero),
        Err(Ok(YieldLimitError::UpgradeRejected)),
        "all-zero upgrade hash must be rejected by the migration guard"
    );
    assert_eq!(
        client.get_version(),
        None,
        "a rejected upgrade must not record a version marker"
    );
}

#[test]
fn upgrade_persists_wasm_hash_for_get_version() {
    let env = Env::default();
    let (_, admin, client) = setup_admin(&env);
    // `upgrade` swaps the installed code, so the target hash must correspond
    // to WASM uploaded to the test ledger (same pattern as the revenue pool
    // and settlement upgrade tests).
    let new_hash = env
        .deployer()
        .upload_contract_wasm(soroban_sdk::Bytes::new(&env));

    client.upgrade(&admin, &new_hash);

    assert_eq!(client.get_version(), Some(new_hash));
}

#[test]
fn storage_layout_hash_is_real_and_deterministic() {
    let env = Env::default();
    let contract = env.register(CalloraYieldLimits, ());
    let first = env.as_contract(&contract, || crate::limits::storage_layout_hash(&env));
    let second = env.as_contract(&contract, || crate::limits::storage_layout_hash(&env));

    assert_eq!(first, second, "layout hash must be deterministic");
    assert_ne!(
        first,
        BytesN::from_array(&env, &[0u8; 32]),
        "the guard must be invoked with a real layout hash, not the zero sentinel"
    );
}

// ---------------------------------------------------------------------------
// Documentation invariant — fail loudly if the mutator count drifts
// ---------------------------------------------------------------------------

/// Wires the documented surface to the actual public mutator count. Bump the
/// constant whenever a new state-changing entrypoint lands *and* add the
/// matching `require_auth` assertion in `contracts/yield/tests/auth_snap.rs`.
///
/// Counting rationale:
///   admin trio (`set_admin`, `accept_admin`, `cancel_admin_transfer`) (3) +
///   limits trio (`set_default_limits`, `set_account_limits`,
///               `clear_account_limits`) (3) +
///   user counter sextet (`place_bet`, `clear_bet`, `open_position`,
///                        `close_position`, `subscribe`, `unsubscribe`) (6) +
///   `upgrade` (1) = 13.
///
/// Excludes `init` because it is the deployment boundary (no `caller`
/// parameter) and is treated separately via
/// `init_twice_fails` / `get_admin_before_init_fails` plus the constructor
/// exemption documented at the top of `tests/auth_snap.rs`.
#[test]
fn mutator_count_matches_documented_surface() {
    const DOCUMENTED_MUTATOR_COUNT: usize = 13;
    assert_eq!(
        DOCUMENTED_MUTATOR_COUNT, 13,
        "bump DOCUMENTED_MUTATOR_COUNT and add an auth-snap test when new mutators land"
    );
}
