//! Tests for `remove_address` on the vault deposit allowlist (Issue #1109).
//!
//! Covers:
//! - Owner removes one address; remaining addresses can still deposit.
//! - Removed address gets `CallerNotInAllowlist` on its next deposit attempt.
//! - Non-owner call to `remove_address` returns `Unauthorized` and changes nothing.
//! - `allowlist_remove` event is emitted with the correct topics (caller, depositor).
//! - Removing an address that is not in the list succeeds and emits no event.
//! - Remove then re-add works correctly.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

use super::*;

// ---------------------------------------------------------------------------
// Helpers — mirror the style used in test_event_schema.rs
// ---------------------------------------------------------------------------

fn create_usdc<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let address = contract_address.address();
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, admin_client)
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

/// Sets up a vault with `initial_balance = Some(0)` and `min_deposit = 1`.
/// Returns `(owner, vault_address, client, usdc_addr, usdc_admin)`.
fn setup(
    env: &Env,
) -> (
    Address,
    Address,
    CalloraVaultClient<'_>,
    Address,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths_allowing_non_root_auth();
    let owner = Address::generate(env);
    let (usdc, usdc_admin) = create_usdc(env, &owner);
    let (vault_address, client) = create_vault(env);
    let settlement = Address::generate(env);
    client.init(
        &owner,
        &usdc,
        &Some(0i128),
        &None,
        &Some(1i128),
        &None,
        &None,
        &Some(settlement),
    );
    // Clear init event so per-test assertions start from a clean slate.
    env.events().all();
    (owner, vault_address, client, usdc, usdc_admin)
}

// ---------------------------------------------------------------------------
// Test: owner removes one address; remaining addresses can still deposit
// ---------------------------------------------------------------------------

#[test]
fn remove_address_keeps_other_entries_and_they_can_deposit() {
    let env = Env::default();
    let (owner, _vault, client, usdc, usdc_admin) = setup(&env);

    let depositor_a = Address::generate(&env);
    let depositor_b = Address::generate(&env);
    let depositor_c = Address::generate(&env);

    // Add all three to the allowlist.
    client.add_address(&owner, &depositor_a);
    client.add_address(&owner, &depositor_b);
    client.add_address(&owner, &depositor_c);
    env.events().all();

    // Remove depositor_b.
    client.remove_address(&owner, &depositor_b);

    // Verify in-contract list no longer contains depositor_b.
    let list = client.get_allowlist();
    assert!(!list.contains(&depositor_b), "depositor_b must be absent");
    assert!(list.contains(&depositor_a), "depositor_a must remain");
    assert!(list.contains(&depositor_c), "depositor_c must remain");

    // depositor_a and depositor_c can still deposit.
    usdc_admin.mint(&depositor_a, &100);
    usdc_admin.mint(&depositor_c, &100);
    client.deposit(&depositor_a, &50);
    client.deposit(&depositor_c, &50);
}

// ---------------------------------------------------------------------------
// Test: removed address gets CallerNotInAllowlist on next deposit
// ---------------------------------------------------------------------------

#[test]
fn removed_address_cannot_deposit() {
    let env = Env::default();
    let (owner, _vault, client, usdc, usdc_admin) = setup(&env);

    let depositor = Address::generate(&env);
    client.add_address(&owner, &depositor);
    env.events().all();

    // Mint so the depositor has funds; the rejection should be auth/allowlist,
    // not a token-balance failure.
    usdc_admin.mint(&depositor, &1_000);

    client.remove_address(&owner, &depositor);

    // Attempting to deposit must return CallerNotInAllowlist.
    let result = client.try_deposit(&depositor, &100);
    assert_eq!(
        result,
        Err(Ok(VaultError::CallerNotInAllowlist)),
        "removed depositor must be rejected with CallerNotInAllowlist"
    );
}

// ---------------------------------------------------------------------------
// Test: non-owner call returns Unauthorized and changes nothing
// ---------------------------------------------------------------------------

#[test]
fn non_owner_remove_address_returns_unauthorized() {
    let env = Env::default();
    let (owner, _vault, client, _usdc, _usdc_admin) = setup(&env);

    let depositor = Address::generate(&env);
    let non_owner = Address::generate(&env);

    client.add_address(&owner, &depositor);
    env.events().all();

    let result = client.try_remove_address(&non_owner, &depositor);
    assert_eq!(
        result,
        Err(Ok(VaultError::Unauthorized)),
        "non-owner must get Unauthorized"
    );

    // Allowlist must be unchanged.
    let list = client.get_allowlist();
    assert!(
        list.contains(&depositor),
        "depositor must still be in the list after failed non-owner remove"
    );

    // No events from the failed call.
    let evts = env.events().all();
    assert!(evts.is_empty(), "failed remove must emit no events");
}

// ---------------------------------------------------------------------------
// Test: allowlist_remove event has correct topics (caller and depositor)
// ---------------------------------------------------------------------------

#[test]
fn remove_address_emits_allowlist_remove_event_with_correct_topics() {
    let env = Env::default();
    let (owner, vault, client, _usdc, _usdc_admin) = setup(&env);

    let depositor = Address::generate(&env);
    client.add_address(&owner, &depositor);
    env.events().all();

    client.remove_address(&owner, &depositor);

    // Collect only events emitted by the vault contract.
    let vault_evts: std::vec::Vec<_> = env
        .events()
        .all()
        .iter()
        .filter(|(addr, _, _)| addr == &vault)
        .collect();

    assert_eq!(vault_evts.len(), 1, "exactly one vault event must be emitted");

    let (_, topics, data) = &vault_evts[0];

    // topic[0] = "allowlist_remove"
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "allowlist_remove"), "topic[0]");

    // topic[1] = "callora_v1" (version marker)
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");

    // topic[2] = caller (owner)
    let t2: Address = topics.get(2).unwrap().into_val(&env);
    assert_eq!(t2, owner, "topic[2] must be the caller (owner)");

    // topic[3] = depositor
    let t3: Address = topics.get(3).unwrap().into_val(&env);
    assert_eq!(t3, depositor, "topic[3] must be the depositor");

    // data = ()
    let _: () = data.clone().into_val(&env);
}

// ---------------------------------------------------------------------------
// Test: removing an address not in the list succeeds and emits no event
// ---------------------------------------------------------------------------

#[test]
fn remove_address_not_in_list_is_idempotent_and_emits_no_event() {
    let env = Env::default();
    let (owner, vault, client, _usdc, _usdc_admin) = setup(&env);

    let stranger = Address::generate(&env);

    // stranger was never added.
    let result = client.try_remove_address(&owner, &stranger);
    assert!(result.is_ok(), "removing absent address must return Ok");

    // No vault events must be emitted.
    let vault_evts: std::vec::Vec<_> = env
        .events()
        .all()
        .iter()
        .filter(|(addr, _, _)| addr == &vault)
        .collect();
    assert!(
        vault_evts.is_empty(),
        "no event must be emitted for absent-address remove"
    );

    // List is still empty / unchanged.
    assert!(client.get_allowlist().is_empty());
}

// ---------------------------------------------------------------------------
// Test: remove then re-add works correctly
// ---------------------------------------------------------------------------

#[test]
fn remove_then_re_add_works() {
    let env = Env::default();
    let (owner, _vault, client, usdc, usdc_admin) = setup(&env);

    let depositor = Address::generate(&env);

    // Add, remove, re-add.
    client.add_address(&owner, &depositor);
    client.remove_address(&owner, &depositor);
    client.add_address(&owner, &depositor);

    // Must be present in the list exactly once.
    let list = client.get_allowlist();
    let count = list.iter().filter(|a| a == depositor).count();
    assert_eq!(count, 1, "address must appear exactly once after re-add");

    // Must be able to deposit again.
    usdc_admin.mint(&depositor, &500);
    let result = client.try_deposit(&depositor, &200);
    assert!(result.is_ok(), "re-added address must be able to deposit");
}
