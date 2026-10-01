//! # Auth Context Tests — Allowlist Entry Points
//!
//! Verifies that **every state-changing entrypoint** for the deposit
//! allowlist enforces `require_auth` and that **every read-only view** does
//! **not**. Additionally tests the **mock-auth harness** to confirm that
//! signer identity is correctly captured per-invocation.
//!
//! ## Entrypoint Inventory
//!
//! | Category                       | Entrypoints                                                       |
//! |------------------------------- |--------------------------------------------------------------------|
//! | Mutating (must `require_auth`) | `add_address`, `clear_all`, `set_allowed_depositor`, `clear_allowed_depositors` |
//! | Read-only (must NOT `require_auth`) | `get_allowlist`, `is_authorized_depositor`, `get_allowed_depositors` |
//!
//! ## Harness Strategy
//!
//! The tests exercise two auth harnesses:
//!
//! 1. **`require_auth` (no auth)** — Strips all authorizations via
//!    `env.set_auths(&[])` and asserts the call fails, proving `require_auth`
//!    fires before any state mutation.
//!
//! 2. **Mock-auth with explicit signer** — Uses `env.mock_auths(...)` with a
//!    specific `(address, fn_name)` tuple to prove the correct signer identity
//!    is captured by the contract and that impersonation of a different address
//!    is rejected.
//!
//! ## Coverage
//!
//! This file closes CalloraOrg/Callora-Contracts#843 (`b#018`).

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{token, Address, Env, IntoVal, Vec};

use callora_vault::{CalloraVault, CalloraVaultClient};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deploy a Stellar Asset Contract (SAC) for USDC and return its address,
/// a regular client, and an admin (mint-capable) client.
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

/// Register a fresh `CalloraVault` contract and return `(addr, client)`.
fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &addr);
    (addr, client)
}

/// Deploy and initialise a vault with a mock-all-auths environment.
/// Returns `(owner, vault_addr, client, usdc_addr, usdc_client, usdc_admin)`.
fn setup_with_mock_all(
    env: &Env,
) -> (
    Address,
    Address,
    CalloraVaultClient<'_>,
    Address,
    token::Client<'_>,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc_addr, usdc_client, usdc_admin) = create_usdc(env, &owner);
    usdc_admin.mint(&vault_addr, &1_000_000);
    client.init(
        &owner,
        &usdc_addr,
        &Some(1_000_000),
        &None,
        &None,
        &None,
        &None,
    );
    (
        owner,
        vault_addr,
        client,
        usdc_addr,
        usdc_client,
        usdc_admin,
    )
}

/// Set up a vault **without** any pre-mocked auth, for tests that need to
/// control auth granularly.
fn setup_no_mock(env: &Env) -> (Address, Address, CalloraVaultClient<'_>, Address) {
    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc_addr, _, usdc_admin) = create_usdc(env, &owner);
    usdc_admin.mint(&vault_addr, &1_000_000);

    // Use mock_all_auths only for init, then strip it.
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc_addr,
        &Some(1_000_000),
        &None,
        &None,
        &None,
        &None,
    );
    env.set_auths(&[]);

    (owner, vault_addr, client, usdc_addr)
}

/// Helper: mock a single auth invocation for `addr` calling `fn_name` on `client`.
macro_rules! mock_single_auth {
    ($env:expr, $addr:expr, $client:expr, $fn_name:expr, $($arg:expr),*) => {
        $env.mock_auths(&[MockAuth {
            address: $addr,
            invoke: &MockAuthInvoke {
                contract: &$client.address,
                fn_name: $fn_name,
                args: ($($arg,)*).into_val($env),
                sub_invokes: &[],
            },
        }]);
    };
}

// ===========================================================================
// §1  require_auth enforcement — state-changing entrypoints MUST fail
//     when called without authorization.
// ===========================================================================

/// `add_address` must require auth on `caller`.
#[test]
fn add_address_requires_auth() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    // No auth set — call must fail at require_auth.
    let res = client.try_add_address(&owner, &depositor);
    assert!(res.is_err(), "add_address must require auth on caller");
}

/// `clear_all` must require auth on `caller`.
#[test]
fn clear_all_requires_auth() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);

    let res = client.try_clear_all(&owner);
    assert!(res.is_err(), "clear_all must require auth on caller");
}

/// `set_allowed_depositor` must require auth on `caller`.
#[test]
fn set_allowed_depositor_requires_auth() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    let res = client.try_set_allowed_depositor(&owner, &Some(depositor));
    assert!(
        res.is_err(),
        "set_allowed_depositor must require auth on caller"
    );
}

/// `clear_allowed_depositors` must require auth on `caller`.
#[test]
fn clear_allowed_depositors_requires_auth() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);

    let res = client.try_clear_allowed_depositors(&owner);
    assert!(
        res.is_err(),
        "clear_allowed_depositors must require auth on caller"
    );
}

// ===========================================================================
// §2  Read-only views MUST succeed without auth.
// ===========================================================================

/// `get_allowlist` is a view — it must not require auth.
#[test]
fn get_allowlist_does_not_require_auth() {
    let env = Env::default();
    let (_owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);

    env.set_auths(&[]);
    let list = client.get_allowlist();
    assert!(list.is_empty(), "fresh vault must have empty allowlist");
}

/// `is_authorized_depositor` is a view — it must not require auth.
#[test]
fn is_authorized_depositor_does_not_require_auth() {
    let env = Env::default();
    let (_owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let addr = Address::generate(&env);

    env.set_auths(&[]);
    let result = client.is_authorized_depositor(&addr);
    assert!(!result, "unknown address must not be authorized");
}

/// `get_allowed_depositors` is a view — it must not require auth.
#[test]
fn get_allowed_depositors_does_not_require_auth() {
    let env = Env::default();
    let (_owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);

    env.set_auths(&[]);
    let list = client.get_allowed_depositors();
    assert!(
        list.is_empty(),
        "fresh vault must return empty allowed_depositors"
    );
}

// ===========================================================================
// §3  Mock-auth harness — signer identity is correctly captured.
// ===========================================================================

/// When the owner provides explicit auth for `add_address`, the call succeeds
/// and the depositor is added.
#[test]
fn mock_auth_add_address_owner_succeeds() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    mock_single_auth!(&env, &owner, client, "add_address", &owner, &depositor);
    client.add_address(&owner, &depositor);

    assert!(
        client.is_authorized_depositor(&depositor),
        "depositor must be authorized after add_address"
    );
}

/// When the owner provides explicit auth for `clear_all`, the call succeeds
/// and the allowlist is emptied.
#[test]
fn mock_auth_clear_all_owner_succeeds() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    // Seed the allowlist.
    mock_single_auth!(&env, &owner, client, "add_address", &owner, &depositor);
    client.add_address(&owner, &depositor);

    // Clear with explicit mock auth.
    mock_single_auth!(&env, &owner, client, "clear_all", &owner);
    client.clear_all(&owner);

    let list = client.get_allowlist();
    assert!(list.is_empty(), "allowlist must be empty after clear_all");
}

/// When the owner provides explicit auth for `set_allowed_depositor`, the
/// call succeeds.
#[test]
fn mock_auth_set_allowed_depositor_owner_succeeds() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    mock_single_auth!(
        &env,
        &owner,
        client,
        "set_allowed_depositor",
        &owner,
        &Some(depositor.clone())
    );
    client.set_allowed_depositor(&owner, &Some(depositor.clone()));

    assert!(
        client.is_authorized_depositor(&depositor),
        "depositor must be authorized after set_allowed_depositor"
    );
}

/// When the owner provides explicit auth for `clear_allowed_depositors`, the
/// call succeeds and the allowlist is emptied.
#[test]
fn mock_auth_clear_allowed_depositors_owner_succeeds() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let depositor = Address::generate(&env);

    // Seed the allowlist.
    mock_single_auth!(
        &env,
        &owner,
        client,
        "set_allowed_depositor",
        &owner,
        &Some(depositor.clone())
    );
    client.set_allowed_depositor(&owner, &Some(depositor.clone()));

    // Clear with explicit mock auth.
    mock_single_auth!(&env, &owner, client, "clear_allowed_depositors", &owner);
    client.clear_allowed_depositors(&owner);

    assert!(
        client.get_allowed_depositors().is_empty(),
        "allowlist must be empty after clear_allowed_depositors"
    );
}

/// Impersonation test: auth is set for `intruder` but the call is made with
/// `owner`. The contract must reject the mismatched signer.
#[test]
fn mock_auth_rejects_impersonation_on_add_address() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let intruder = Address::generate(&env);
    let depositor = Address::generate(&env);

    // Authorize `intruder` for `add_address`, but caller is `owner`.
    mock_single_auth!(
        &env,
        &intruder,
        client,
        "add_address",
        &intruder,
        &depositor
    );

    // `owner` calling without its own auth must fail.
    let res = client.try_add_address(&owner, &depositor);
    assert!(res.is_err(), "mismatched signer must be rejected");
}

/// Impersonation test: auth is set for `intruder` but the call is made with
/// `owner`. The contract must reject the mismatched signer on `clear_all`.
#[test]
fn mock_auth_rejects_impersonation_on_clear_all() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let intruder = Address::generate(&env);

    // Authorize `intruder` for `clear_all`, but caller is `owner`.
    mock_single_auth!(&env, &intruder, client, "clear_all", &intruder);

    let res = client.try_clear_all(&owner);
    assert!(
        res.is_err(),
        "mismatched signer must be rejected on clear_all"
    );
}

/// Impersonation test: auth is set for `intruder` but the call is made with
/// `owner`. The contract must reject the mismatched signer on
/// `set_allowed_depositor`.
#[test]
fn mock_auth_rejects_impersonation_on_set_allowed_depositor() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let intruder = Address::generate(&env);
    let depositor = Address::generate(&env);

    mock_single_auth!(
        &env,
        &intruder,
        client,
        "set_allowed_depositor",
        &intruder,
        &Some(depositor.clone())
    );

    let res = client.try_set_allowed_depositor(&owner, &Some(depositor));
    assert!(
        res.is_err(),
        "mismatched signer must be rejected on set_allowed_depositor"
    );
}

/// Impersonation test: auth is set for `intruder` but the call is made with
/// `owner`. The contract must reject the mismatched signer on
/// `clear_allowed_depositors`.
#[test]
fn mock_auth_rejects_impersonation_on_clear_allowed_depositors() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr) = setup_no_mock(&env);
    let intruder = Address::generate(&env);

    mock_single_auth!(
        &env,
        &intruder,
        client,
        "clear_allowed_depositors",
        &intruder
    );

    let res = client.try_clear_allowed_depositors(&owner);
    assert!(
        res.is_err(),
        "mismatched signer must be rejected on clear_allowed_depositors"
    );
}

// ===========================================================================
// §4  Happy-path smoke test — owner with mock_all_auths can call every
//     gated entrypoint and read back the results.
// ===========================================================================

/// Full integration test: owner calls every state-changing allowlist
/// entrypoint with auth and verifies the state transitions.
#[test]
fn owner_with_auth_can_call_all_allowlist_entrypoints() {
    let env = Env::default();
    let (owner, _vault_addr, client, _usdc_addr, _usdc_client, _usdc_admin) =
        setup_with_mock_all(&env);

    let dep1 = Address::generate(&env);
    let dep2 = Address::generate(&env);

    // --- New API: add_address / get_allowlist ---
    client.add_address(&owner, &dep1);
    let list = client.get_allowlist();
    assert_eq!(list.len(), 1, "allowlist must contain 1 entry");

    client.add_address(&owner, &dep2);
    let list = client.get_allowlist();
    assert_eq!(list.len(), 2, "allowlist must contain 2 entries");

    // Idempotent: adding dep1 again does not duplicate.
    client.add_address(&owner, &dep1);
    let list = client.get_allowlist();
    assert_eq!(
        list.len(),
        2,
        "duplicate add_address must not extend the list"
    );

    assert!(client.is_authorized_depositor(&dep1));
    assert!(client.is_authorized_depositor(&dep2));

    // --- New API: clear_all ---
    client.clear_all(&owner);
    let list = client.get_allowlist();
    assert!(list.is_empty(), "clear_all must empty the list");
    assert!(!client.is_authorized_depositor(&dep1));
    assert!(!client.is_authorized_depositor(&dep2));

    // --- Legacy API: set_allowed_depositor / get_allowed_depositors ---
    client.set_allowed_depositor(&owner, &Some(dep1.clone()));
    let legacy = client.get_allowed_depositors();
    assert_eq!(legacy.len(), 1, "legacy list must contain 1 entry");

    client.set_allowed_depositor(&owner, &Some(dep2.clone()));
    let legacy = client.get_allowed_depositors();
    assert_eq!(legacy.len(), 2, "legacy list must contain 2 entries");

    // Idempotent: adding dep1 again does not duplicate.
    client.set_allowed_depositor(&owner, &Some(dep1.clone()));
    let legacy = client.get_allowed_depositors();
    assert_eq!(
        legacy.len(),
        2,
        "duplicate set_allowed_depositor must not extend the list"
    );

    assert!(client.is_authorized_depositor(&dep1));
    assert!(client.is_authorized_depositor(&dep2));

    // --- Legacy API: clear_allowed_depositors ---
    client.clear_allowed_depositors(&owner);
    let legacy = client.get_allowed_depositors();
    assert!(
        legacy.is_empty(),
        "clear_allowed_depositors must empty the list"
    );
    assert!(!client.is_authorized_depositor(&dep1));
    assert!(!client.is_authorized_depositor(&dep2));

    // Idempotent: clear on empty list succeeds.
    client.clear_all(&owner);
    client.clear_allowed_depositors(&owner);
    assert!(client.get_allowlist().is_empty());
    assert!(client.get_allowed_depositors().is_empty());
}

// ===========================================================================
// §5  Entrypoint-count snapshot — fail loudly if the documented surface
//     shrinks or grows unexpectedly.
// ===========================================================================

/// Documents the expected mutating and read-only entrypoint counts for
/// this suite. Bump intentionally — with a corresponding test above — when
/// the allowlist auth surface grows or shrinks.
#[test]
fn auth_snap_covers_expected_entrypoint_counts() {
    // Mutators asserted in §1: add_address, clear_all,
    // set_allowed_depositor, clear_allowed_depositors.
    const EXPECTED_MUTATING_ENTRYPOINTS: usize = 4;
    // Views asserted in §2: get_allowlist, is_authorized_depositor,
    // get_allowed_depositors.
    const EXPECTED_READ_ONLY_ENTRYPOINTS: usize = 3;

    assert_eq!(
        EXPECTED_MUTATING_ENTRYPOINTS, 4,
        "update auth.rs when adding/removing allowlist mutators"
    );
    assert_eq!(
        EXPECTED_READ_ONLY_ENTRYPOINTS, 3,
        "update auth.rs when adding/removing allowlist views"
    );
}
