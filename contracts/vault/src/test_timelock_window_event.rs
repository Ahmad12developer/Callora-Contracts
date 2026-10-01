//! # Tests for `tl_window_changed` event payload correctness (Issue #1112)
//!
//! Before the fix, `set_timelock_window` wrote the new window to storage and
//! then called `get_timelock_window` for the event's "old" value. Because the
//! write had already happened the read returned the new value, so the event
//! carried `(new, new)` instead of `(old, new)`.
//!
//! These tests assert the correct `(old, new)` ordering in both the
//! default-to-custom and custom-to-custom transition cases.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

use super::{timelock, CalloraVault, CalloraVaultClient};
use super::timelock::{DEFAULT_TIMELOCK_SECONDS, MAX_TIMELOCK_SECONDS, MIN_TIMELOCK_SECONDS};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (addr.clone(), token::StellarAssetClient::new(env, &addr))
}

/// Stand-up a vault with a separate admin role. Returns
/// `(owner, client, admin, vault_addr)`.
fn setup(env: &Env) -> (Address, CalloraVaultClient<'_>, Address, Address) {
    env.ledger().set_timestamp(1_700_000_000);
    let owner = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);
    let (usdc, _) = create_usdc(env, &owner);
    let admin = Address::generate(env);
    env.mock_all_auths();
    // min_deposit must be > 0; 1 is the minimum valid value.
    client.init(
        &owner,
        &usdc,
        &None,        // initial_balance
        &None,        // authorized_caller
        &Some(1i128), // min_deposit (must be > 0)
        &None,        // revenue_pool
        &None,        // max_deduct
        &None,        // settlement
    );
    // Rotate admin to a distinct address so admin != owner in auth tests.
    client.set_admin(&owner, &admin);
    client.accept_admin();
    (owner, client, admin, vault_addr)
}

// ---------------------------------------------------------------------------
// #1112 — event payload order
// ---------------------------------------------------------------------------

/// Default-to-custom: the first element of the data tuple must be the
/// DEFAULT window (172 800 s), not the newly-written value.
///
/// Regression for issue #1112: before the fix both elements were the new
/// value because `get_timelock_window` was called after `set_timelock_window`
/// had already persisted the change.
#[test]
fn tl_window_changed_default_to_custom_carries_old_value_first() {
    let env = Env::default();
    let (_, client, admin, vault_addr) = setup(&env);

    let new_window = MIN_TIMELOCK_SECONDS + 7_200; // 1 h + 2 h = 3 h
    client.set_timelock_window(&admin, &new_window);

    // Storage reflects the new value.
    assert_eq!(client.get_timelock_window(), new_window);

    let events = env.events().all();
    let last = events.last().expect("expected tl_window_changed event");
    assert_eq!(last.0, vault_addr, "event must originate from vault");

    let topic0: Symbol = last.1.get(0).unwrap().into_val(&env);
    assert_eq!(
        topic0,
        Symbol::new(&env, "tl_window_changed"),
        "topic 0 must be \"tl_window_changed\""
    );

    // data[0] == old (DEFAULT), data[1] == new
    let payload: (u64, u64) = last.2.into_val(&env);
    assert_eq!(
        payload.0,
        DEFAULT_TIMELOCK_SECONDS,
        "payload.0 must be the DEFAULT (old) window ({DEFAULT_TIMELOCK_SECONDS}), \
         not the new value ({new_window}); equal values mean the bug is still present"
    );
    assert_eq!(
        payload.1, new_window,
        "payload.1 must be the newly-set window"
    );
    assert_ne!(
        payload.0, payload.1,
        "old and new must differ — equal values indicate the bug is still present"
    );
}

/// Custom-to-custom: after changing from `first_window` to `second_window`,
/// the event must report `first_window` as the first element.
#[test]
fn tl_window_changed_custom_to_custom_carries_old_value_first() {
    let env = Env::default();
    let (_, client, admin, vault_addr) = setup(&env);

    // First change: default → first_window.
    let first_window = MIN_TIMELOCK_SECONDS + 3_600; // 2 h
    client.set_timelock_window(&admin, &first_window);
    assert_eq!(client.get_timelock_window(), first_window);

    // Second change: first_window → second_window.
    let second_window = MAX_TIMELOCK_SECONDS - 3_600; // ~30 d − 1 h
    client.set_timelock_window(&admin, &second_window);
    assert_eq!(client.get_timelock_window(), second_window);

    let events = env.events().all();
    let last = events.last().expect("expected second tl_window_changed event");
    assert_eq!(last.0, vault_addr);

    let topic0: Symbol = last.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "tl_window_changed"));

    // data[0] == first_window (the old value), data[1] == second_window (new)
    let payload: (u64, u64) = last.2.into_val(&env);
    assert_eq!(
        payload.0, first_window,
        "payload.0 must be the previously-set window ({first_window}), \
         not the new value ({second_window})"
    );
    assert_eq!(
        payload.1, second_window,
        "payload.1 must be the newly-set window"
    );
    assert_ne!(
        payload.0, payload.1,
        "old and new must differ — equal values indicate the bug is still present"
    );
}
