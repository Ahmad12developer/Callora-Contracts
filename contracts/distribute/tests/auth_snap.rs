//! # Auth Snapshot — Per-Entrypoint Authorization Tests
//!
//! Verifies that every **state-changing entrypoint** in `callora-distribute`
//! enforces `require_auth` and that every **read-only entrypoint** does
//! **not** require authorization.
//!
//! ## Coverage
//!
//! | Category              | Entrypoints covered                                               |
//! |-----------------------|-------------------------------------------------------------------|
//! | Initialization        | `init`                                                            |
//! | Admin rotation        | `set_admin`, `accept_admin`, `claim_admin`, `cancel_admin_transfer` |
//! | Circuit-breaker       | `pause`, `unpause`                                                |
//! | Distribution cap      | `set_max_distribute`                                              |
//! | Distribution          | `distribute`, `batch_distribute`                                  |
//! | Upgrade               | `upgrade`                                                         |
//! | Read-only views       | `get_admin`, `get_usdc_token`, `get_pending_admin`,               |
//! |                       | `get_paused`, `get_max_distribute`, `get_max_batch_size`,         |
//! |                       | `balance`, `get_version`, `version`                               |

#![cfg(test)]

extern crate std;

use callora_distribute::{CalloraDistribute, CalloraDistributeClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env};
use callora_distribute::{Distribute, DistributeClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token;
use soroban_sdk::{Address, BytesN, Env, Vec};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deploy a Stellar Asset Contract for USDC; returns (address, admin_client).
fn create_usdc(env: &Env, admin: Address) -> (Address, token::StellarAssetClient<'_>) {
    let contract = env.register_stellar_asset_contract_v2(admin);
    let addr = contract.address();
    let sac = token::StellarAssetClient::new(env, &addr);
    (addr, sac)
}

/// Deploy the Distribute contract and return (contract_address, client).
fn create_contract(env: &Env) -> (Address, DistributeClient<'_>) {
    let addr = env.register(Distribute, ());
    let client = DistributeClient::new(env, &addr);
    (addr, client)
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
/// Full setup: deploy contract + USDC, call `init` under `mock_all_auths`.
/// Returns `(contract_addr, admin, usdc_addr, client, usdc_sac)`.
fn setup(
    env: &Env,
) -> (
    Address,
    Address,
    Address,
    DistributeClient<'_>,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let (usdc_addr, usdc_sac) = create_usdc(env, admin.clone());
    let (contract_addr, client) = create_contract(env);
    client.init(&admin, &usdc_addr);
    (contract_addr, admin, usdc_addr, client, usdc_sac)
}

// ---------------------------------------------------------------------------
// init — must require admin signature
// ---------------------------------------------------------------------------

/// `init` without any auth must fail (front-running protection).
#[test]
fn init_requires_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let (usdc_addr, _sac) = create_usdc(&env, admin.clone());
    let (_addr, client) = create_contract(&env);

    // No auths installed — must be rejected.
    env.set_auths(&[]);
    let result = client.try_init(&admin, &usdc_addr);
    assert!(
        result.is_err(),
        "init must fail when admin has not signed"
    );
}

/// `init` with the admin's signature must succeed and write state.
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
fn init_succeeds_with_admin_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (usdc_addr, _sac) = create_usdc(&env, admin.clone());
    let (_addr, client) = create_contract(&env);

    client.init(&admin, &usdc_addr);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_usdc_token(), usdc_addr);
}

/// A second `init` call must always fail, even under full auth.
#[test]
fn init_second_call_fails_with_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (usdc_addr, _sac) = create_usdc(&env, admin.clone());
    let (_addr, client) = create_contract(&env);

    client.init(&admin, &usdc_addr);

    // Second call — must be rejected regardless of auth.
    let result = client.try_init(&admin, &usdc_addr);
    assert!(
        result.is_err(),
        "second init must fail with AlreadyInitialized"
    );
}

// ---------------------------------------------------------------------------
// Admin rotation
// ---------------------------------------------------------------------------

#[test]
fn set_admin_requires_auth() {
    let env = Env::default();
    let (_, admin, _, client, _) = setup(&env);

    let new_admin = Address::generate(&env);
    env.set_auths(&[]);
    let result = client.try_set_admin(&admin, &new_admin);
    assert!(result.is_err(), "set_admin must require auth");
}

#[test]
fn accept_admin_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);
    let (_, admin, _, client, _) = setup(&env);

    let new_admin = Address::generate(&env);
    // Nominate — with full auths still active from setup's mock_all_auths.
    env.mock_all_auths();
    client.set_admin(&admin, &new_admin);

    // Now strip auths: accept_admin must fail.
    env.set_auths(&[]);
    let result = client.try_accept_admin(&new_admin);
    assert!(result.is_err(), "accept_admin must require auth");
}

#[test]
fn claim_admin_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);
    let (_, admin, _, client, _) = setup(&env);

    let new_admin = Address::generate(&env);
    env.mock_all_auths();
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_accept_admin(&new_admin);
    assert!(res.is_err(), "accept_admin must require auth");
    let result = client.try_claim_admin(&new_admin);
    assert!(result.is_err(), "claim_admin must require auth");
}

#[test]
fn cancel_admin_transfer_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);
    let (_, admin, _, client, _) = setup(&env);

    let new_admin = Address::generate(&env);
    env.mock_all_auths();
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let result = client.try_cancel_admin_transfer(&admin);
    assert!(result.is_err(), "cancel_admin_transfer must require auth");
}

// ---------------------------------------------------------------------------
// Circuit-breaker
// ---------------------------------------------------------------------------

#[test]
fn pause_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);
    let (_, admin, _, client, _) = setup(&env);

    env.set_auths(&[]);
    let result = client.try_pause(&admin);
    assert!(result.is_err(), "pause must require auth");
}

#[test]
fn unpause_requires_auth() {
    let env = Env::default();
    let (admin, _usdc, client) = setup(&env);
    let (_, admin, _, client, _) = setup(&env);

    env.mock_all_auths();
    client.pause(&admin);

    env.set_auths(&[]);
    let result = client.try_unpause(&admin);
    assert!(result.is_err(), "unpause must require auth");
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
// ---------------------------------------------------------------------------
// Distribution cap
// ---------------------------------------------------------------------------

#[test]
fn set_max_distribute_requires_auth() {
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
    let (_, admin, _, client, _) = setup(&env);

    env.set_auths(&[]);
    let result = client.try_set_max_distribute(&admin, &1_000);
    assert!(result.is_err(), "set_max_distribute must require auth");
}

// ---------------------------------------------------------------------------
// distribute / batch_distribute
// ---------------------------------------------------------------------------

#[test]
fn distribute_requires_auth() {
    let env = Env::default();
    let (contract_addr, admin, _usdc_addr, client, usdc_sac) = setup(&env);

    // Fund the contract so that auth is the only thing blocking.
    env.mock_all_auths();
    usdc_sac.mint(&contract_addr, &1_000);

    let recipient = Address::generate(&env);
    env.set_auths(&[]);
    assert_eq!(client.get_usdc_token(), usdc);
    let result = client.try_distribute(&admin, &recipient, &100);
    assert!(result.is_err(), "distribute must require auth");
}

#[test]
fn batch_distribute_requires_auth() {
    let env = Env::default();
    let (_admin, _usdc, client) = setup(&env);
    let (contract_addr, admin, _usdc_addr, client, usdc_sac) = setup(&env);

    env.mock_all_auths();
    usdc_sac.mint(&contract_addr, &1_000);

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
    let recipient = Address::generate(&env);
    let mut payments = Vec::new(&env);
    payments.push_back((recipient, 100_i128));

    env.set_auths(&[]);
    let result = client.try_batch_distribute(&admin, &payments);
    assert!(result.is_err(), "batch_distribute must require auth");
}

// ---------------------------------------------------------------------------
// Upgrade
// ---------------------------------------------------------------------------

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
fn upgrade_requires_auth() {
    let env = Env::default();
    let (_, admin, _, client, _) = setup(&env);

    let dummy_hash = BytesN::from_array(&env, &[0u8; 32]);
    env.set_auths(&[]);
    let result = client.try_upgrade(&admin, &dummy_hash);
    assert!(result.is_err(), "upgrade must require auth");
}

// ---------------------------------------------------------------------------
// Read-only views — must NOT require auth
// ---------------------------------------------------------------------------

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
fn read_only_views_do_not_require_auth() {
    let env = Env::default();
    let (contract_addr, admin, usdc_addr, client, usdc_sac) = setup(&env);

    // Fund so that `balance` doesn't panic from missing USDC state.
    env.mock_all_auths();
    usdc_sac.mint(&contract_addr, &500);

    // Strip all auths — every view below must still succeed.
    env.set_auths(&[]);

    assert_eq!(client.get_admin(), admin, "get_admin");
    assert_eq!(client.get_usdc_token(), usdc_addr, "get_usdc_token");
    assert_eq!(client.get_pending_admin(), None, "get_pending_admin");
    assert!(!client.get_paused(), "get_paused");
    assert_eq!(
        client.get_max_distribute(),
        i128::MAX,
        "get_max_distribute default"
    );
    let _ = client.get_max_batch_size();
    assert_eq!(client.balance(), 500, "balance");
    assert_eq!(client.get_version(), None, "get_version — never upgraded");
    let _ = client.version();
}
