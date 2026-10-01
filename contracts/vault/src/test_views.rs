extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (addr.clone(), token::StellarAssetClient::new(env, &addr))
}

/// Register a vault without initializing it.
fn register(env: &Env) -> CalloraVaultClient<'_> {
    let vault_addr = env.register(CalloraVault, ());
    CalloraVaultClient::new(env, &vault_addr)
}

/// Initialize a vault with the given optional max_deduct, settlement and
/// revenue pool. Returns `(owner, client, usdc)`.
fn setup_with(
    env: &Env,
    initial_balance: i128,
    max_deduct: Option<i128>,
    settlement: Option<Address>,
    revenue_pool: Option<Address>,
) -> (Address, CalloraVaultClient<'_>, Address) {
    let owner = Address::generate(env);
    let client = register(env);
    let (usdc, _) = create_usdc(env, &owner);
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(initial_balance),
        &None,
        &Some(1),
        &revenue_pool,
        &max_deduct,
        &settlement,
    );
    (owner, client, usdc)
}

/// Default vault: zero balance, large max_deduct, no settlement, no pool.
fn setup(env: &Env) -> (Address, CalloraVaultClient<'_>, Address) {
    setup_with(env, 0, Some(10_000_000_000), None, None)
}

// ---------------------------------------------------------------------------
// get_owner
// (replaces the removed get_meta tests: the vault no longer exposes get_meta)
// ---------------------------------------------------------------------------

#[test]
fn get_owner_before_init_panics() {
    let env = Env::default();
    let client = register(&env);
    assert!(client.try_get_owner().is_err());
}

#[test]
fn get_owner_returns_owner_after_init() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    assert_eq!(client.get_owner(), owner);
}

// ---------------------------------------------------------------------------
// balance
// ---------------------------------------------------------------------------

/// Behaviour change: `balance()` now returns 0 on an uninitialized vault
/// instead of panicking.
#[test]
fn balance_before_init_returns_zero() {
    let env = Env::default();
    let client = register(&env);
    assert_eq!(client.balance(), 0);
}

#[test]
fn balance_returns_zero_after_init() {
    let env = Env::default();
    let (_, client, _) = setup(&env);
    assert_eq!(client.balance(), 0);
}

#[test]
fn balance_returns_initial_balance_after_init() {
    let env = Env::default();
    let (_, client, _) = setup_with(&env, 500, Some(10_000), None, None);
    assert_eq!(client.balance(), 500);
}

// ---------------------------------------------------------------------------
// get_admin
// ---------------------------------------------------------------------------

#[test]
fn get_admin_before_init_panics() {
    let env = Env::default();
    let client = register(&env);
    assert!(client.try_get_admin().is_err());
}

#[test]
fn get_admin_returns_owner_after_init() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    assert_eq!(client.get_admin(), owner);
}

// ---------------------------------------------------------------------------
// get_usdc_token
// ---------------------------------------------------------------------------

#[test]
fn get_usdc_token_before_init_panics() {
    let env = Env::default();
    let client = register(&env);
    assert!(client.try_get_usdc_token().is_err());
}

#[test]
fn get_usdc_token_returns_address_after_init() {
    let env = Env::default();
    let (_, client, usdc) = setup(&env);
    assert_eq!(client.get_usdc_token(), usdc);
}

// ---------------------------------------------------------------------------
// get_max_deduct / set_max_deduct
// ---------------------------------------------------------------------------

/// `DEFAULT_MAX_DEDUCT` no longer exists: when `init` gets `None`, the cap
/// is `i128::MAX`.
#[test]
fn get_max_deduct_returns_default_when_not_set() {
    let env = Env::default();
    let (_, client, _) = setup_with(&env, 0, None, None, None);
    assert_eq!(client.get_max_deduct(), i128::MAX);
}

#[test]
fn get_max_deduct_returns_configured_value() {
    let env = Env::default();
    let (_, client, _) = setup_with(&env, 0, Some(500), None, None);
    assert_eq!(client.get_max_deduct(), 500);
}

#[test]
fn set_max_deduct_updates_max_deduct_key_and_getter() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    assert_eq!(client.get_max_deduct(), 10_000_000_000);

    client.set_max_deduct(&owner, &250);
    assert_eq!(client.get_max_deduct(), 250);

    client.set_max_deduct(&owner, &900);
    assert_eq!(client.get_max_deduct(), 900);
}

#[test]
fn set_max_deduct_rejects_non_positive_values() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    let zero = client.try_set_max_deduct(&owner, &0);
    assert!(matches!(zero, Err(Ok(VaultError::MaxDeductNotPositive))));
    let negative = client.try_set_max_deduct(&owner, &-1);
    assert!(matches!(negative, Err(Ok(VaultError::MaxDeductNotPositive))));
    // The stored value is unchanged after the rejected calls.
    assert_eq!(client.get_max_deduct(), 10_000_000_000);
}

#[test]
fn set_max_deduct_rejects_non_owner() {
    let env = Env::default();
    let (_, client, _) = setup(&env);
    let stranger = Address::generate(&env);
    let res = client.try_set_max_deduct(&stranger, &100);
    assert!(matches!(res, Err(Ok(VaultError::Unauthorized))));
    assert_eq!(client.get_max_deduct(), 10_000_000_000);
}

// ---------------------------------------------------------------------------
// get_settlement / set_settlement
// ---------------------------------------------------------------------------

#[test]
fn get_settlement_before_set_panics() {
    let env = Env::default();
    let (_, client, _) = setup(&env); // init with settlement = None
    assert!(client.try_get_settlement().is_err());
}

#[test]
fn get_settlement_returns_address_configured_at_init() {
    let env = Env::default();
    let settlement = Address::generate(&env);
    let (_, client, _) = setup_with(&env, 0, Some(10_000), Some(settlement.clone()), None);
    assert_eq!(client.get_settlement(), settlement);
}

#[test]
fn get_settlement_returns_address_after_set() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    let settlement = Address::generate(&env);

    client.set_settlement(&owner, &settlement);
    assert_eq!(client.get_settlement(), settlement);
}

// ---------------------------------------------------------------------------
// get_revenue_pool
// (propose_revenue_pool / accept_revenue_pool no longer exist; the pool is
// now set at init)
// ---------------------------------------------------------------------------

#[test]
fn get_revenue_pool_returns_none_when_not_set() {
    let env = Env::default();
    let (_, client, _) = setup(&env);
    assert!(client.get_revenue_pool().is_none());
}

#[test]
fn get_revenue_pool_returns_some_when_set_at_init() {
    let env = Env::default();
    let pool = Address::generate(&env);
    let (_, client, _) = setup_with(&env, 0, Some(10_000), None, Some(pool.clone()));
    assert_eq!(client.get_revenue_pool(), Some(pool));
}

// ---------------------------------------------------------------------------
// is_paused
// ---------------------------------------------------------------------------

#[test]
fn is_paused_returns_false_before_init() {
    let env = Env::default();
    let client = register(&env);
    // must not panic and must return false
    assert!(!client.is_paused());
}

#[test]
fn is_paused_reflects_pause_unpause() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    assert!(!client.is_paused());
    client.pause(&owner);
    assert!(client.is_paused());
    client.unpause(&owner);
    assert!(!client.is_paused());
}

// ---------------------------------------------------------------------------
// is_authorized_depositor
// ---------------------------------------------------------------------------

/// The doc comment on `is_authorized_depositor` says the view reflects only
/// the explicit allowlist; the owner may deposit regardless but is not listed.
#[test]
fn is_authorized_depositor_owner_is_not_on_allowlist() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    assert!(!client.is_authorized_depositor(&owner));
}

#[test]
fn is_authorized_depositor_unknown_address_false() {
    let env = Env::default();
    let (_, client, _) = setup(&env);
    let stranger = Address::generate(&env);
    assert!(!client.is_authorized_depositor(&stranger));
}

#[test]
fn is_authorized_depositor_added_address_true() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    let depositor = Address::generate(&env);
    client.add_address(&owner, &depositor);
    assert!(client.is_authorized_depositor(&depositor));
}

// ---------------------------------------------------------------------------
// get_allowlist (was get_allowed_depositors)
// ---------------------------------------------------------------------------

#[test]
fn get_allowlist_empty_before_any_added() {
    let env = Env::default();
    let (_, client, _) = setup(&env);
    assert_eq!(client.get_allowlist().len(), 0);
}

#[test]
fn get_allowlist_reflects_additions() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    let d1 = Address::generate(&env);
    let d2 = Address::generate(&env);
    client.add_address(&owner, &d1);
    client.add_address(&owner, &d2);
    let list = client.get_allowlist();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&d1));
    assert!(list.contains(&d2));
}

#[test]
fn get_allowlist_add_is_idempotent() {
    let env = Env::default();
    let (owner, client, _) = setup(&env);
    let d1 = Address::generate(&env);
    client.add_address(&owner, &d1);
    client.add_address(&owner, &d1);
    assert_eq!(client.get_allowlist().len(), 1);
}