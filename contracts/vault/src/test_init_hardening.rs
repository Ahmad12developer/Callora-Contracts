extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (
        addr.clone(),
        token::Client::new(env, &addr),
        token::StellarAssetClient::new(env, &addr),
    )
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &addr);
    (addr, client)
}

// ---------------------------------------------------------------------------
// Re-init guard
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "vault already initialized")]
fn reinit_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    // second call must panic
    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
}

#[test]
fn reinit_via_try_returns_err() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    let result = client.try_init(&owner, &usdc, &None, &None, &None, &None, &None);
    assert!(result.is_err(), "second init must return Err");
}

// ---------------------------------------------------------------------------
// usdc_token validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "usdc_token cannot be vault address")]
fn init_usdc_token_is_vault_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);

    // pass the vault's own address as usdc_token
    client.init(
        &owner,
        &vault_addr,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
}

// ---------------------------------------------------------------------------
// min_deposit validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "min_deposit must be positive")]
fn init_min_deposit_zero_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(&owner, &usdc, &None, &None, &Some(0), &None, &None);
}

#[test]
#[should_panic(expected = "min_deposit must be positive")]
fn init_min_deposit_negative_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(&owner, &usdc, &None, &None, &Some(-1), &None, &None);
}

#[test]
fn init_min_deposit_one_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    let meta = client.init(&owner, &usdc, &None, &None, &Some(1), &None, &None);
    assert_eq!(meta.min_deposit, 1);
}

// ---------------------------------------------------------------------------
// max_deduct validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "max_deduct must be positive")]
fn init_max_deduct_zero_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        0,
        &soroban_sdk::Address::generate(&env),
    );
}

#[test]
#[should_panic(expected = "min_deposit cannot exceed max_deduct")]
fn init_min_deposit_exceeds_max_deduct_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    // min=100, max=50 → invalid
    client.init(&owner, &usdc, &None, &None, &Some(100), &None, &Some(50));
}

#[test]
fn init_min_equals_max_deduct_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    let meta = client.init(&owner, &usdc, &None, &None, &Some(50), &None, &Some(50));
    assert_eq!(meta.min_deposit, 50);
    assert_eq!(client.get_max_deduct(), 50);
}

// ---------------------------------------------------------------------------
// revenue_pool validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "revenue_pool cannot be vault address")]
fn init_revenue_pool_is_vault_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(&owner, &usdc, &None, &None, &None, &Some(vault_addr), &None);
}

#[test]
fn init_with_valid_revenue_pool_stores_it() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let pool = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &None,
        &Some(pool.clone()),
        &None,
    );
    assert_eq!(client.get_revenue_pool(), Some(pool));
}

#[test]
fn init_without_revenue_pool_stores_none() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    assert_eq!(client.get_revenue_pool(), None);
}

// ---------------------------------------------------------------------------
// authorized_caller validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "authorized_caller cannot be vault address")]
fn init_authorized_caller_is_vault_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(&owner, &usdc, &None, &Some(vault_addr), &None, &None, &None);
}

// ---------------------------------------------------------------------------
// initial_balance validation
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "initial_balance exceeds on-ledger USDC balance")]
fn init_initial_balance_exceeds_onchain_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    // fund vault with 50 but claim 100
    usdc_admin.mint(&vault_addr, &50);
    client.init(&owner, &usdc, &Some(100), &None, &None, &None, &None);
}

#[test]
fn init_initial_balance_zero_no_onchain_check() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    // zero balance — no on-chain check needed
    let meta = client.init(&owner, &usdc, &Some(0), &None, &None, &None, &None);
    assert_eq!(meta.balance, 0);
}

#[test]
fn init_initial_balance_exact_onchain_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    usdc_admin.mint(&vault_addr, &200);
    let meta = client.init(&owner, &usdc, &Some(200), &None, &None, &None, &None);
    assert_eq!(meta.balance, 200);
}

// ---------------------------------------------------------------------------
// Post-init state correctness
// ---------------------------------------------------------------------------

#[test]
fn init_sets_admin_to_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    assert_eq!(client.get_admin(), owner);
}

#[test]
fn init_emits_event_with_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (vault_addr, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    usdc_admin.mint(&vault_addr, &500);
    client.init(&owner, &usdc, &Some(500), &None, &None, &None, &None);

    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::IntoVal;
    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            e.0 == vault_addr && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "init")
            }
        })
        .expect("init event not found");

    let data: i128 = ev.2.into_val(&env);
    assert_eq!(data, 500);
}

#[test]
fn init_default_min_deposit_is_one() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    let meta = client.init(
        &owner,
        &usdc,
        &0,
        &owner,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    assert_eq!(meta.min_deposit, DEFAULT_MIN_DEPOSIT);
}

#[test]
fn init_none_initial_balance_allows_deposit_and_deduct() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    let settlement = Address::generate(&env);

    client.init(
        &owner,
        &usdc,
        &None,
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(1000),
        &Some(settlement),
    );

    usdc_admin.mint(&owner, &100);

    // Deposit 50
    client.deposit(&owner, &50);
    assert_eq!(client.balance(), 50);

    // Deduct 20
    client.deduct(&owner, &20, &123);
    assert_eq!(client.balance(), 30);
}
