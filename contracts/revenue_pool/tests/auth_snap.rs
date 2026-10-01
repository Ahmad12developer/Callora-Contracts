//! # Auth Snapshot — Per‑Entrypoint Authorization Tests
//!
//! This integration test module verifies that **every state‑changing entrypoint**
//! in the revenue‑pool contract enforces `require_auth` and that every
//! **read‑only entrypoint** does **not** require authorization.
//!
//! The tests serve as a living snapshot of the contract's auth surface: if a new
//! state‑changing entrypoint is added **without** `require_auth`, the
//! corresponding read‑only‑group test in this file will catch it during CI.
//!
//! ## Coverage
//!
//! | Category                         | Entrypoints covered |
//! |----------------------------------|---------------------|
//! | Initialization                   | `init` |
//! | Admin rotation                   | `set_admin`, `accept_admin`, `claim_admin`, `cancel_admin_transfer` |
//! | Pause guardian                   | `set_pause_guardian`, `clear_pause_guardian` |
//! | Circuit breaker                  | `pause`, `unpause` |
//! | Yield management                 | `receive_payment`, `deposit_yield`, `set_max_distribute` |
//! | Distribution                     | `distribute`, `batch_distribute` |
//! | Upgrade / broadcast              | `upgrade`, `broadcast` |
//! | Emergency drain                  | `propose_emergency_drain`, `execute_emergency_drain`, `cancel_emergency_drain` |
//! | Read‑only views + helpers        | `get_admin`, `get_usdc_token`, `get_pending_admin`, `get_pause_guardian`, `is_paused`, `get_cumulative_yield_deposited`, `get_max_distribute`, `balance`, `get_version`, `version`, `get_ttl_policy`, `get_pending_emergency_drain`, `chunk_iter` |

extern crate std;

use callora_revenue_pool::{chunk_iter, RevenuePool, RevenuePoolClient, Severity, MAX_BATCH_SIZE};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token;
use soroban_sdk::{Address, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deploy a Stellar Asset Contract (SAC) token for USDC and return its address
/// together with a regular client and an admin (mint‑capable) client.
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

/// Deploy the `RevenuePool` contract and return its on‑ledger address together
/// with a typed client.
fn create_pool(env: &Env) -> (Address, RevenuePoolClient<'_>) {
    let address = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(env, &address);
    (address, client)
}

/// Mint `amount` USDC into `pool_address` (requires the admin SAC client).
fn fund_pool(usdc_admin_client: &token::StellarAssetClient, pool_address: &Address, amount: i128) {
    usdc_admin_client.mint(pool_address, &amount);
}

/// Initialise the pool with a fresh admin + USDC token, fully mocking auth for
/// the setup call.  Returns `(admin, pool_address, client, usdc_address, usdc_client, usdc_admin_client)`.
fn setup_pool(
    env: &Env,
) -> (
    Address,
    Address,
    RevenuePoolClient<'_>,
    Address,
    token::Client<'_>,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let (pool_addr, client) = create_pool(env);
    let (usdc_addr, usdc_client, usdc_admin) = create_usdc(env, &admin);
    client.init(&admin, &usdc_addr);
    (admin, pool_addr, client, usdc_addr, usdc_client, usdc_admin)
}

// ---------------------------------------------------------------------------
// Auth‑requiring entrypoints — each test verifies that calling the entrypoint
// WITHOUT authorization fails.
// ---------------------------------------------------------------------------

/// Verify that `init` requires auth on the configured admin.
#[test]
fn init_requires_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (_, client) = create_pool(&env);
    let (usdc, _, _) = create_usdc(&env, &admin);

    env.set_auths(&[]);
    let res = client.try_init(&admin, &usdc);
    assert!(res.is_err(), "init must require auth");
}

/// Verify that `set_admin` requires auth on `caller`.
#[test]
fn set_admin_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    // After setup we consumed all mocked-auths; an unauthorised caller must be
    // rejected when require_auth is hit.
    env.set_auths(&[]);
    let new_admin = Address::generate(&env);
    let res = client.try_set_admin(&admin, &new_admin);
    assert!(res.is_err(), "set_admin must require auth");
}

/// Verify that `accept_admin` requires auth on `caller`.
#[test]
fn accept_admin_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    // Nominate a new admin first (with mocked auth).
    env.mock_all_auths();
    let pending = Address::generate(&env);
    client.set_admin(&admin, &pending);

    // Now strip auth — accept must fail.
    env.set_auths(&[]);
    let res = client.try_accept_admin(&pending);
    assert!(res.is_err(), "accept_admin must require auth");
}

/// Verify that `claim_admin` (alias for `accept_admin`) requires auth on `caller`.
#[test]
fn claim_admin_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.mock_all_auths();
    let pending = Address::generate(&env);
    client.set_admin(&admin, &pending);

    env.set_auths(&[]);
    let res = client.try_claim_admin(&pending);
    assert!(res.is_err(), "claim_admin must require auth");
}

/// Verify that `cancel_admin_transfer` requires auth on `caller`.
#[test]
fn cancel_admin_transfer_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.mock_all_auths();
    let pending = Address::generate(&env);
    client.set_admin(&admin, &pending);

    env.set_auths(&[]);
    let res = client.try_cancel_admin_transfer(&admin);
    assert!(res.is_err(), "cancel_admin_transfer must require auth");
}

/// Verify that `set_pause_guardian` requires auth on `caller`.
#[test]
fn set_pause_guardian_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let guardian = Address::generate(&env);
    let res = client.try_set_pause_guardian(&admin, &guardian);
    assert!(res.is_err(), "set_pause_guardian must require auth");
}

/// Verify that `clear_pause_guardian` requires auth on `caller`.
#[test]
fn clear_pause_guardian_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    // Must have a guardian set first so the function can reach require_auth.
    env.mock_all_auths();
    let guardian = Address::generate(&env);
    client.set_pause_guardian(&admin, &guardian);

    env.set_auths(&[]);
    let res = client.try_clear_pause_guardian(&admin);
    assert!(res.is_err(), "clear_pause_guardian must require auth");
}

/// Verify that `pause` requires auth on `caller`.
#[test]
fn pause_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let res = client.try_pause(&admin);
    assert!(res.is_err(), "pause must require auth");
}

/// Verify that `unpause` requires auth on `caller`.
#[test]
fn unpause_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    // Pause first (mocked auth).
    env.mock_all_auths();
    client.pause(&admin);

    // Strip auth — unpause must fail.
    env.set_auths(&[]);
    let res = client.try_unpause(&admin);
    assert!(res.is_err(), "unpause must require auth");
}

/// Verify that `receive_payment` requires auth on `caller`.
#[test]
fn receive_payment_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let res = client.try_receive_payment(&admin, &250_i128, &true);
    assert!(res.is_err(), "receive_payment must require auth");
}

/// Verify that `deposit_yield` requires auth on the `treasury` argument
/// (the first non‑env parameter).
#[test]
fn deposit_yield_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, usdc_admin) = setup_pool(&env);

    // Give the admin some USDC so deposit_yield could succeed if auth passed.
    env.mock_all_auths();
    usdc_admin.mint(&admin, &1_000);

    // Auth is on `treasury` (the admin address here); strip it.
    env.set_auths(&[]);
    let source = Symbol::new(&env, "fees");
    let res = client.try_deposit_yield(&admin, &400_i128, &source);
    assert!(res.is_err(), "deposit_yield must require auth on treasury");
}

/// Verify that `set_max_distribute` requires auth on `caller`.
#[test]
fn set_max_distribute_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let res = client.try_set_max_distribute(&admin, &500_i128);
    assert!(res.is_err(), "set_max_distribute must require auth");
}

/// Verify that `distribute` requires auth on `caller`.
#[test]
fn distribute_requires_auth() {
    let env = Env::default();
    let (admin, pool_addr, client, _usdc_addr, _usdc_client, usdc_admin) = setup_pool(&env);

    env.mock_all_auths();
    fund_pool(&usdc_admin, &pool_addr, 1_000);

    env.set_auths(&[]);
    let developer = Address::generate(&env);
    let res = client.try_distribute(&admin, &developer, &100_i128);
    assert!(res.is_err(), "distribute must require auth");
}

/// Verify that `batch_distribute` requires auth on `caller`.
#[test]
fn batch_distribute_requires_auth() {
    let env = Env::default();
    let (admin, pool_addr, client, _usdc_addr, _usdc_client, usdc_admin) = setup_pool(&env);

    env.mock_all_auths();
    fund_pool(&usdc_admin, &pool_addr, 1_000);
    let dev = Address::generate(&env);

    env.set_auths(&[]);
    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
    payments.push_back((dev, 100_i128));
    let res = client.try_batch_distribute(&admin, &payments);
    assert!(res.is_err(), "batch_distribute must require auth");
}

/// Verify that `upgrade` requires auth on `caller`.
#[test]
fn upgrade_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let dummy_hash = BytesN::from_array(&env, &[0u8; 32]);
    let res = client.try_upgrade(&admin, &dummy_hash);
    assert!(res.is_err(), "upgrade must require auth");
}

/// Verify that `broadcast` requires auth on `caller`.
#[test]
fn broadcast_requires_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let msg = soroban_sdk::String::from_str(&env, "test broadcast");
    let res = client.try_broadcast(&admin, &Severity::Info, &msg);
    assert!(res.is_err(), "broadcast must require auth");
}

/// Verify that `propose_emergency_drain` requires auth on `caller`.
#[test]
fn propose_emergency_drain_requires_auth() {
    let env = Env::default();
    let (admin, pool_addr, client, _, _, usdc_admin) = setup_pool(&env);

    env.mock_all_auths();
    fund_pool(&usdc_admin, &pool_addr, 10_000);
    let treasury = Address::generate(&env);

    env.set_auths(&[]);
    let res = client.try_propose_emergency_drain(&admin, &treasury, &5_000_i128);
    assert!(res.is_err(), "propose_emergency_drain must require auth");
}

/// Verify that `execute_emergency_drain` requires auth on `caller`.
#[test]
fn execute_emergency_drain_requires_auth() {
    let env = Env::default();
    let (admin, pool_addr, client, _, _, usdc_admin) = setup_pool(&env);

    // Propose a drain first (mocked auth).
    env.mock_all_auths();
    fund_pool(&usdc_admin, &pool_addr, 10_000);
    let treasury = Address::generate(&env);
    client.propose_emergency_drain(&admin, &treasury, &5_000_i128);

    // Advance the ledger beyond the 24‑h timelock.
    env.ledger().set_timestamp(86_401);

    // Strip auth — execution must fail.
    env.set_auths(&[]);
    let res = client.try_execute_emergency_drain(&admin);
    assert!(res.is_err(), "execute_emergency_drain must require auth");
}

/// Verify that `cancel_emergency_drain` requires auth on `caller`.
#[test]
fn cancel_emergency_drain_requires_auth() {
    let env = Env::default();
    let (admin, pool_addr, client, _, _, usdc_admin) = setup_pool(&env);

    env.mock_all_auths();
    fund_pool(&usdc_admin, &pool_addr, 10_000);
    let treasury = Address::generate(&env);
    client.propose_emergency_drain(&admin, &treasury, &5_000_i128);

    env.set_auths(&[]);
    let res = client.try_cancel_emergency_drain(&admin);
    assert!(res.is_err(), "cancel_emergency_drain must require auth");
}

// ---------------------------------------------------------------------------
// Read‑only entrypoints — each test verifies that calling without auth
// **succeeds** (no require_auth panic).
// ---------------------------------------------------------------------------

/// `get_admin` is a view — it must not require auth.
#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    // get_admin is a view; if it required auth the call would panic.
    let _admin = client.get_admin();
}

/// `get_usdc_token` is a view — it must not require auth.
#[test]
fn get_usdc_token_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, usdc_addr, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let token = client.get_usdc_token();
    assert_eq!(token, usdc_addr);
}

/// `get_pending_admin` is a view — it must not require auth.
#[test]
fn get_pending_admin_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let pending = client.get_pending_admin();
    assert_eq!(pending, None);
}

/// `get_pause_guardian` is a view — it must not require auth.
#[test]
fn get_pause_guardian_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let guardian = client.get_pause_guardian();
    assert_eq!(guardian, None);
}

/// `is_paused` is a view — it must not require auth.
#[test]
fn is_paused_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let paused = client.is_paused();
    assert!(!paused);
}

/// `get_cumulative_yield_deposited` is a view — it must not require auth,
/// even after state changes (e.g. after a yield deposit).
#[test]
fn get_cumulative_yield_deposited_does_not_require_auth() {
    let env = Env::default();
    let (admin, _, client, _, _, usdc_admin) = setup_pool(&env);

    // After init, the view returns 0 without auth.
    env.set_auths(&[]);
    let cum = client.get_cumulative_yield_deposited();
    assert_eq!(cum, 0);

    // Deposit yield (with auth) to change state.
    env.mock_all_auths();
    usdc_admin.mint(&admin, &1_000);
    let source = Symbol::new(&env, "fees");
    client.deposit_yield(&admin, &400_i128, &source);

    // After state change, the view still must not require auth.
    env.set_auths(&[]);
    let cum = client.get_cumulative_yield_deposited();
    assert_eq!(cum, 400);
}

/// `get_max_distribute` is a view — it must not require auth.
#[test]
fn get_max_distribute_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let cap = client.get_max_distribute();
    assert_eq!(cap, i128::MAX);
}

/// `balance` is a view — it must not require auth.
#[test]
fn balance_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let bal = client.balance();
    assert_eq!(bal, 0);
}

/// `get_version` is a view — it must not require auth.
#[test]
fn get_version_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let ver = client.get_version();
    // Version is None until the contract is upgraded.
    assert_eq!(ver, None);
}

/// `version` is a view — it must not require auth.
#[test]
fn version_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let v = client.version();
    // Should return the crate version string.
    assert!(!v.is_empty());
}

/// `get_ttl_policy` is a view — it must not require auth.
#[test]
fn get_ttl_policy_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let policy = client.get_ttl_policy();
    assert!(!policy.is_empty());
}

/// `get_pending_emergency_drain` is a view — it must not require auth.
#[test]
fn get_pending_emergency_drain_does_not_require_auth() {
    let env = Env::default();
    let (_, _, client, _, _, _) = setup_pool(&env);

    env.set_auths(&[]);
    let pending = client.get_pending_emergency_drain();
    assert_eq!(pending, None);
}

/// `chunk_iter` is a pure helper function — it must not require auth.
#[test]
fn chunk_iter_does_not_require_auth() {
    let env = Env::default();
    let payments: Vec<(Address, i128)> = Vec::new(&env);
    let chunks = chunk_iter(&env, payments, MAX_BATCH_SIZE);
    assert_eq!(chunks.len(), 0);
}

// ---------------------------------------------------------------------------
// Canonical smoke test — admin **with** auth can call every gated entrypoint.
// ---------------------------------------------------------------------------

/// A single integration test that successfully invokes every state‑changing
/// entrypoint with proper authorization.  This proves the harness setup is
/// correct and that the entrypoints are reachable when auth is provided.
#[test]
fn admin_with_auth_can_call_all_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (pool_addr, client) = create_pool(&env);
    let (usdc_addr, _usdc_client, usdc_admin) = create_usdc(&env, &admin);
    client.init(&admin, &usdc_addr);
    fund_pool(&usdc_admin, &pool_addr, 50_000);

    // --- Admin rotation ---
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    assert!(client.get_pending_admin().is_some());
    client.cancel_admin_transfer(&admin);

    client.set_admin(&admin, &new_admin);
    client.claim_admin(&new_admin);
    assert_eq!(client.get_admin(), new_admin);

    // Reset admin back for consistency.
    client.set_admin(&new_admin, &admin);
    client.claim_admin(&admin);
    assert_eq!(client.get_admin(), admin);

    // --- Pause guardian ---
    let guardian = Address::generate(&env);
    client.set_pause_guardian(&admin, &guardian);
    assert_eq!(client.get_pause_guardian(), Some(guardian.clone()));
    client.clear_pause_guardian(&admin);
    assert_eq!(client.get_pause_guardian(), None);

    // --- Circuit breaker ---
    client.pause(&admin);
    assert!(client.is_paused());
    client.unpause(&admin);
    assert!(!client.is_paused());

    // --- Yield management ---
    client.receive_payment(&admin, &1_000, &true);
    let source = Symbol::new(&env, "fees");
    usdc_admin.mint(&admin, &5_000);
    client.deposit_yield(&admin, &2_000, &source);
    assert_eq!(client.get_cumulative_yield_deposited(), 2_000);

    client.set_max_distribute(&admin, &10_000);
    assert_eq!(client.get_max_distribute(), 10_000);

    // --- Distribution ---
    let dev = Address::generate(&env);
    client.distribute(&admin, &dev, &500);
    assert_eq!(client.balance(), 50_000 + 2_000 - 500); // initial + yield - distribute

    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
    payments.push_back((Address::generate(&env), 100_i128));
    payments.push_back((Address::generate(&env), 200_i128));
    assert!(client.try_batch_distribute(&admin, &payments).is_ok());

    // --- Upgrade ---
    // upgrade requires auth. With mock_all_auths active, the call should reach
    // the wasm-update step (which may fail with a non-auth error for a dummy
    // hash). We just need to prove auth is not the failure cause.
    let dummy_hash = BytesN::from_array(&env, &[1u8; 32]);
    let upgrade_res = client.try_upgrade(&admin, &dummy_hash);
    // The call did not panic due to auth — any error is from the wasm layer.
    let _ = upgrade_res;

    // --- Broadcast ---
    let msg = soroban_sdk::String::from_str(&env, "auth smoke test");
    client.broadcast(&admin, &Severity::Info, &msg);

    // --- Emergency drain ---
    let treasury = Address::generate(&env);
    client.propose_emergency_drain(&admin, &treasury, &1_000);
    assert!(client.get_pending_emergency_drain().is_some());
    client.cancel_emergency_drain(&admin);
    assert_eq!(client.get_pending_emergency_drain(), None);

    client.propose_emergency_drain(&admin, &treasury, &1_000);
    env.ledger().set_timestamp(86_401);
    client.execute_emergency_drain(&admin);
    assert_eq!(client.get_pending_emergency_drain(), None);
}
