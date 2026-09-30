#![cfg(test)]

extern crate std;

use callora_distribute::{CalloraDistribute, CalloraDistributeClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};

fn create_contract(env: &Env) -> CalloraDistributeClient<'_> {
    let contract_id = env.register(CalloraDistribute, ());
    CalloraDistributeClient::new(env, &contract_id)
}

/// Create a contract and initialise it with a fresh admin and a real USDC
/// token address.  Returns `(admin, usdc_address, client)`.
fn setup(env: &Env) -> (Address, Address, CalloraDistributeClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let client = create_contract(env);
    client.init(&admin, &usdc);
    (admin, usdc, client)
}

// ---------------------------------------------------------------------------
// Mutating entrypoints — must require auth
// ---------------------------------------------------------------------------

#[test]
fn init_requires_no_prior_initialization() {
    // init itself does not call require_auth on the admin in this contract
    // (the admin is set during init).  Verify double-init is rejected.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let client = create_contract(&env);
    client.init(&admin, &usdc);

    // Second call must fail with AlreadyInitialized.
    let res = client.try_init(&admin, &usdc);
    assert!(res.is_err(), "double init must be rejected");
}

#[test]
fn set_admin_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let new_admin = Address::generate(&env);
    let res = client.try_set_admin(&admin, &new_admin);
    assert!(res.is_err(), "set_admin must require auth");
}

#[test]
fn accept_admin_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.mock_all_auths();
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_accept_admin(&new_admin);
    assert!(res.is_err(), "accept_admin must require auth");
}

#[test]
fn cancel_admin_transfer_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.mock_all_auths();
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_cancel_admin_transfer(&admin);
    assert!(res.is_err(), "cancel_admin_transfer must require auth");
}

#[test]
fn pause_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_pause(&admin);
    assert!(res.is_err(), "pause must require auth");
}

#[test]
fn unpause_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.mock_all_auths();
    client.pause(&admin);

    env.set_auths(&[]);
    let res = client.try_unpause(&admin);
    assert!(res.is_err(), "unpause must require auth");
}

#[test]
fn set_max_distribute_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_set_max_distribute(&admin, &500_i128);
    assert!(res.is_err(), "set_max_distribute must require auth");
}

#[test]
fn distribute_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let recipient = Address::generate(&env);
    let res = client.try_distribute(&admin, &recipient, &100_i128);
    assert!(res.is_err(), "distribute must require auth");
}

#[test]
fn upgrade_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let hash = BytesN::from_array(&env, &[0u8; 32]);
    let res = client.try_upgrade(&admin, &hash);
    assert!(res.is_err(), "upgrade must require auth");
}

#[test]
fn broadcast_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let msg = soroban_sdk::String::from_str(&env, "test");
    let res = client.try_broadcast(&admin, &callora_distribute::Severity::Info, &msg);
    assert!(res.is_err(), "broadcast must require auth");
}

// ---------------------------------------------------------------------------
// View entrypoints — must NOT require auth
// ---------------------------------------------------------------------------

#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn get_usdc_token_does_not_require_auth() {
    let env = Env::default();
    let (_admin, usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_usdc_token(), usdc);
}

#[test]
fn is_paused_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert!(!client.is_paused());
}

#[test]
fn get_paused_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert!(!client.get_paused());
}

#[test]
fn get_pending_admin_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_pending_admin(), None);
}

#[test]
fn get_max_distribute_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let max = client.get_max_distribute();
    assert!(max > 0);
}

#[test]
fn get_max_batch_size_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert!(client.get_max_batch_size() > 0);
}

#[test]
fn get_version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_version(), None);
}

#[test]
fn version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    let v = client.version();
    assert!(!v.is_empty());
}

#[test]
fn balance_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.balance(), 0);
}

// ---------------------------------------------------------------------------
// Pause state invariant — single StorageKey::Paused key
// ---------------------------------------------------------------------------

/// After `pause`, `is_paused()` and `get_paused()` must both return `true`.
/// This confirms both aliases read from the same `StorageKey::Paused` slot.
#[test]
fn pause_and_unpause_single_key_invariant() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);

    env.mock_all_auths();

    // Initially unpaused via both view aliases.
    assert!(!client.is_paused());
    assert!(!client.get_paused());

    client.pause(&admin);
    assert!(client.is_paused(), "is_paused must return true after pause");
    assert!(client.get_paused(), "get_paused must return true after pause");

    client.unpause(&admin);
    assert!(!client.is_paused(), "is_paused must return false after unpause");
    assert!(!client.get_paused(), "get_paused must return false after unpause");
}

// ---------------------------------------------------------------------------
// Happy-path integration — admin with auth can call all entrypoints
// ---------------------------------------------------------------------------

#[test]
fn admin_with_auth_can_call_mutating_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let client = create_contract(&env);
    client.init(&admin, &usdc);

    assert_eq!(client.get_admin(), admin);

    // Two-step admin rotation.
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);

    // Pause / unpause round-trip.
    assert!(!client.is_paused());
    client.pause(&new_admin);
    assert!(client.is_paused());
    client.unpause(&new_admin);
    assert!(!client.is_paused());

    // Distribution cap.
    client.set_max_distribute(&new_admin, &1_000_i128);
    assert_eq!(client.get_max_distribute(), 1_000_i128);
}
