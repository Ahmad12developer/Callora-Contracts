extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

use super::*;

// ---------------------------------------------------------------------------
// Helpers (mirror the patterns in test.rs)
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let address = contract_address.address();
    let client = token::Client::new(env, &address);
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

/// Set up a fully initialised vault ready for deposits.
/// Returns `(vault_address, client, usdc_address, usdc_client, usdc_admin, owner)`.
fn setup(
    env: &Env,
) -> (
    Address,
    CalloraVaultClient<'_>,
    Address,
    token::Client<'_>,
    token::StellarAssetClient<'_>,
    Address,
) {
    let owner = Address::generate(env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(env, &owner);
    let (vault_address, client) = create_vault(env);
    env.mock_all_auths();
    client.init(&owner, &usdc, &Some(0), &Some(owner.clone()), &Some(1), &None, &None);
    (vault_address, client, usdc, usdc_client, usdc_admin, owner)
}

// ---------------------------------------------------------------------------
// get_reserve_cap — default
// ---------------------------------------------------------------------------

#[test]
fn get_reserve_cap_returns_max_when_not_set() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, _owner) = setup(&env);
    assert_eq!(client.get_reserve_cap(&usdc), i128::MAX);
}

// ---------------------------------------------------------------------------
// set_reserve_cap — validation
// ---------------------------------------------------------------------------

#[test]
fn set_reserve_cap_stores_value() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &1_000_000);
    assert_eq!(client.get_reserve_cap(&usdc), 1_000_000);
}

#[test]
fn set_reserve_cap_can_be_updated() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &500);
    client.set_reserve_cap(&owner, &usdc, &2_000);
    assert_eq!(client.get_reserve_cap(&usdc), 2_000);
}

#[test]
fn set_reserve_cap_to_max_restores_unlimited() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &100);
    client.set_reserve_cap(&owner, &usdc, &i128::MAX);
    assert_eq!(client.get_reserve_cap(&usdc), i128::MAX);
}

#[test]
fn set_reserve_cap_rejects_zero() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    let result = client.try_set_reserve_cap(&owner, &usdc, &0);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::AmountNotPositive);
}

#[test]
fn set_reserve_cap_rejects_negative() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    let result = client.try_set_reserve_cap(&owner, &usdc, &-1);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::AmountNotPositive);
}

#[test]
fn set_reserve_cap_requires_owner() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, _owner) = setup(&env);
    let non_owner = Address::generate(&env);
    env.mock_all_auths();
    let result = client.try_set_reserve_cap(&non_owner, &usdc, &1_000);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::Unauthorized);
}

#[test]
fn set_reserve_cap_is_per_token() {
    let env = Env::default();
    let (_vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    let other_token = Address::generate(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &500);
    // Other token still has no cap
    assert_eq!(client.get_reserve_cap(&other_token), i128::MAX);
}

// ---------------------------------------------------------------------------
// set_reserve_cap — events
// ---------------------------------------------------------------------------

#[test]
fn set_reserve_cap_emits_event() {
    let env = Env::default();
    let (vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &1_000);

    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            if e.0 != vault_address {
                return false;
            }
            if e.1.is_empty() {
                return false;
            }
            let s: Symbol = e.1.get(0).unwrap().into_val(&env);
            s == Symbol::new(&env, "reserve_cap_set")
        })
        .expect("expected reserve_cap_set event");

    let (prev, cap): (Option<i128>, i128) = ev.2.into_val(&env);
    assert_eq!(prev, None);
    assert_eq!(cap, 1_000);
}

#[test]
fn set_reserve_cap_event_includes_previous_value() {
    let env = Env::default();
    let (vault_address, client, usdc, _usdc_client, _usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &500);
    client.set_reserve_cap(&owner, &usdc, &999);

    let events = env.events().all();
    // Find the last reserve_cap_set event
    let ev = events
        .iter()
        .filter(|e| {
            if e.0 != vault_address {
                return false;
            }
            if e.1.is_empty() {
                return false;
            }
            let s: Symbol = e.1.get(0).unwrap().into_val(&env);
            s == Symbol::new(&env, "reserve_cap_set")
        })
        .last()
        .expect("expected reserve_cap_set event");

    let (prev, cap): (Option<i128>, i128) = ev.2.into_val(&env);
    assert_eq!(prev, Some(500));
    assert_eq!(cap, 999);
}

// ---------------------------------------------------------------------------
// deposit — cap enforcement
// ---------------------------------------------------------------------------

#[test]
fn deposit_succeeds_when_no_cap_set() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    usdc_admin.mint(&owner, &1_000_000);
    usdc_client.approve(&owner, &vault_address, &1_000_000, &99999);
    let balance = client.deposit(&owner, &1_000_000);
    assert_eq!(balance, 1_000_000);
}

#[test]
fn deposit_succeeds_when_below_cap() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &500);
    usdc_admin.mint(&owner, &300);
    usdc_client.approve(&owner, &vault_address, &300, &99999);
    let balance = client.deposit(&owner, &300);
    assert_eq!(balance, 300);
}

#[test]
fn deposit_succeeds_at_exact_cap() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &1_000);
    usdc_admin.mint(&owner, &1_000);
    usdc_client.approve(&owner, &vault_address, &1_000, &99999);
    let balance = client.deposit(&owner, &1_000);
    assert_eq!(balance, 1_000);
}

#[test]
fn deposit_fails_when_exceeds_cap() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &1_000);
    usdc_admin.mint(&owner, &1_001);
    usdc_client.approve(&owner, &vault_address, &1_001, &99999);
    let result = client.try_deposit(&owner, &1_001);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::ExceedsReserveCap);
}

#[test]
fn deposit_fails_when_cumulative_total_exceeds_cap() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &500);

    usdc_admin.mint(&owner, &800);
    usdc_client.approve(&owner, &vault_address, &800, &99999);

    // First deposit of 400 — succeeds
    let b1 = client.deposit(&owner, &400);
    assert_eq!(b1, 400);

    // Second deposit of 200 — succeeds (total = 600 > cap? No, 400+200=600 > 500 — fails!)
    let result = client.try_deposit(&owner, &200);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::ExceedsReserveCap);
}

#[test]
fn deposit_succeeds_again_after_cap_raised() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &300);

    usdc_admin.mint(&owner, &600);
    usdc_client.approve(&owner, &vault_address, &600, &99999);

    client.deposit(&owner, &300);

    // Would fail with cap=300
    let result = client.try_deposit(&owner, &1);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::ExceedsReserveCap);

    // Raise the cap
    client.set_reserve_cap(&owner, &usdc, &600);

    // Now deposit of 300 more succeeds
    let b = client.deposit(&owner, &300);
    assert_eq!(b, 600);
}

#[test]
fn deposit_at_one_above_cap_fails() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    client.set_reserve_cap(&owner, &usdc, &99);
    usdc_admin.mint(&owner, &100);
    usdc_client.approve(&owner, &vault_address, &100, &99999);
    let result = client.try_deposit(&owner, &100);
    assert_eq!(result.unwrap_err().unwrap(), VaultError::ExceedsReserveCap);
}

#[test]
fn reserve_cap_does_not_affect_withdrawal() {
    let env = Env::default();
    let (vault_address, client, _usdc, usdc_client, usdc_admin, owner) = setup(&env);
    env.mock_all_auths();
    usdc_admin.mint(&owner, &500);
    usdc_client.approve(&owner, &vault_address, &500, &99999);
    client.deposit(&owner, &500);

    // Set a tight cap (lower than current balance — should not block withdrawal)
    client.set_reserve_cap(&owner, &usdc, &100);
    let new_bal = client.withdraw(&200);
    assert_eq!(new_bal, 300);
}
