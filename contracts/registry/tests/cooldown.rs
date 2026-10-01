//! Per-developer cooldown tests for `callora-registry`.
//!
//! Acceptance criteria verified here:
//!
//! 1. **Same-developer rejection** – A second registration by the same
//!    developer within the cooldown window returns `AdminCooldownActive`.
//!
//! 2. **Different-developer concurrency** – Two distinct developers can each
//!    register within the same cooldown hour without blocking each other.
//!
//! 3. **Cooldown expiry** – The same developer may register again after the
//!    cooldown window has elapsed.
//!
//! 4. **Storage-key isolation** – The `DeveloperCooldown` key for developer A
//!    is independent of the key for developer B: writing one does not affect
//!    the other, and reading back each key returns the correct timestamp.
//!
//! 5. **Authorization / security boundaries** – Cooldown enforcement does not
//!    weaken existing auth checks; an unauthenticated caller and a non-admin
//!    caller are both still rejected before cooldown is consulted.
//!
//! 6. **Gated path parity** – `register_offering_with_gate` enforces the same
//!    per-developer cooldown as `register_offering`.
//!
//! 7. **No partial state on cooldown rejection** – A rejected registration
//!    leaves `registered_count` and `is_offering_registered` unchanged.

#![cfg(test)]

extern crate std;

use callora_registry::{admin, CalloraRegistry, CalloraRegistryClient, RegistryError};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{contract, contractimpl, Address, Env, String};

// ---------------------------------------------------------------------------
// Minimal mock catalog (accepts every call, no side-effects)
// ---------------------------------------------------------------------------

pub mod ok_catalog {
    use super::*;

    #[contract]
    pub struct OkCatalog;

    #[contractimpl]
    impl OkCatalog {
        pub fn put_offering(
            _env: Env,
            _registry: Address,
            _offering_id: String,
            _metadata: String,
        ) {
        }
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn setup(env: &Env) -> (Address, CalloraRegistryClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let catalog = env.register(ok_catalog::OkCatalog, ());
    let registry_id = env.register(CalloraRegistry, ());
    let client = CalloraRegistryClient::new(env, &registry_id);
    client.init(&admin, &catalog);
    (admin, client)
}

fn oid(env: &Env, tag: &str) -> String {
    String::from_str(env, &std::format!("offering-{tag}"))
}

fn meta(env: &Env) -> String {
    String::from_str(env, "ipfs://QmCooldownTest")
}

/// Advance the ledger clock exactly `seconds` into the future.
fn advance(env: &Env, seconds: u64) {
    let current = env.ledger().get().timestamp;
    env.ledger().set(LedgerInfo {
        timestamp: current + seconds,
        ..env.ledger().get()
    });
}

// ---------------------------------------------------------------------------
// 1. Same-developer rejection within the cooldown window
// ---------------------------------------------------------------------------

/// A developer who just registered cannot register again within the same
/// cooldown window; the second attempt returns `AdminCooldownActive`.
#[test]
fn cooldown_same_developer_rejected_within_window() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    // First registration succeeds.
    client.register_offering(&admin, &dev, &oid(&env, "first"), &meta(&env));
    assert_eq!(client.registered_count(), 1);

    // Advance by less than one full cooldown period (one second short).
    advance(&env, admin::COOLDOWN_SECONDS - 1);

    // Second registration by the same developer must be rejected.
    let result = client.try_register_offering(&admin, &dev, &oid(&env, "second"), &meta(&env));
    assert!(
        matches!(result, Err(Ok(RegistryError::AdminCooldownActive))),
        "expected AdminCooldownActive for same developer within window, got {result:?}"
    );
    // State is unchanged.
    assert_eq!(client.registered_count(), 1);
    assert!(!client.is_offering_registered(&oid(&env, "second")));
}

/// Rejection at t = last + COOLDOWN_SECONDS - 1 (boundary: one second before
/// expiry). The cooldown check is strict: `elapsed < COOLDOWN_SECONDS`.
#[test]
fn cooldown_boundary_one_second_before_expiry_rejected() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    client.register_offering(&admin, &dev, &oid(&env, "base"), &meta(&env));

    // Move to exactly one second before the window closes.
    advance(&env, admin::COOLDOWN_SECONDS - 1);

    let result = client.try_register_offering(&admin, &dev, &oid(&env, "boundary"), &meta(&env));
    assert!(
        matches!(result, Err(Ok(RegistryError::AdminCooldownActive))),
        "one second before expiry must still be blocked, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Different developers can register concurrently within the same hour
// ---------------------------------------------------------------------------

/// Developer B must not be blocked by developer A's cooldown, and vice-versa.
/// Both can register within the same one-hour window.
#[test]
fn cooldown_different_developers_register_concurrently() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    // Developer A registers first.
    client.register_offering(&admin, &dev_a, &oid(&env, "a-first"), &meta(&env));
    assert_eq!(client.registered_count(), 1);

    // Developer B registers at the same ledger time (< 1 h after dev A).
    // This must succeed even though dev A's cooldown is active.
    client.register_offering(&admin, &dev_b, &oid(&env, "b-first"), &meta(&env));
    assert_eq!(client.registered_count(), 2);

    assert!(client.is_offering_registered(&oid(&env, "a-first")));
    assert!(client.is_offering_registered(&oid(&env, "b-first")));
}

/// After both developers register, each of them is independently blocked by
/// their own cooldown but can still see the other's offering.
#[test]
fn cooldown_each_developer_blocked_independently() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    client.register_offering(&admin, &dev_a, &oid(&env, "a"), &meta(&env));
    client.register_offering(&admin, &dev_b, &oid(&env, "b"), &meta(&env));

    // Advance but stay within the cooldown window.
    advance(&env, admin::COOLDOWN_SECONDS / 2);

    // Both developers are blocked for their own second offering.
    let r_a = client.try_register_offering(&admin, &dev_a, &oid(&env, "a2"), &meta(&env));
    let r_b = client.try_register_offering(&admin, &dev_b, &oid(&env, "b2"), &meta(&env));

    assert!(
        matches!(r_a, Err(Ok(RegistryError::AdminCooldownActive))),
        "dev_a must be blocked, got {r_a:?}"
    );
    assert!(
        matches!(r_b, Err(Ok(RegistryError::AdminCooldownActive))),
        "dev_b must be blocked, got {r_b:?}"
    );
    // Count stayed at 2.
    assert_eq!(client.registered_count(), 2);
}

// ---------------------------------------------------------------------------
// 3. Cooldown expiry allows re-registration by the same developer
// ---------------------------------------------------------------------------

/// Exactly at t = last + COOLDOWN_SECONDS the cooldown has expired and the
/// same developer may register again.
#[test]
fn cooldown_expires_exactly_at_window_boundary() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    client.register_offering(&admin, &dev, &oid(&env, "first"), &meta(&env));

    // Advance by exactly COOLDOWN_SECONDS (elapsed == COOLDOWN_SECONDS, not <).
    advance(&env, admin::COOLDOWN_SECONDS);

    // Must succeed.
    client.register_offering(&admin, &dev, &oid(&env, "second"), &meta(&env));
    assert_eq!(client.registered_count(), 2);
    assert!(client.is_offering_registered(&oid(&env, "second")));
}

/// A developer may register again well after the cooldown window has passed.
#[test]
fn cooldown_expired_same_developer_can_register_again() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    client.register_offering(&admin, &dev, &oid(&env, "first"), &meta(&env));

    // Advance past the full cooldown window.
    advance(&env, admin::COOLDOWN_SECONDS + 1);

    client.register_offering(&admin, &dev, &oid(&env, "second"), &meta(&env));
    assert_eq!(client.registered_count(), 2);
}

// ---------------------------------------------------------------------------
// 4. Storage-key isolation between developers
// ---------------------------------------------------------------------------

/// `last_developer_action` returns `None` for a developer who has never
/// registered, even after another developer has registered.
#[test]
fn cooldown_storage_key_absent_for_fresh_developer() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let registry_id = client.address.clone();
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    // Dev A registers.
    client.register_offering(&admin, &dev_a, &oid(&env, "a"), &meta(&env));

    // Dev B has no cooldown record yet.
    env.as_contract(&registry_id, || {
        let ts = admin::last_developer_action(&env, &dev_b);
        assert!(
            ts.is_none(),
            "dev_b must have no cooldown record before their first registration"
        );
    });
}

/// After each developer registers, `last_developer_action` for each returns
/// their own timestamp and is not overwritten by the other.
#[test]
fn cooldown_storage_keys_are_independent_per_developer() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let registry_id = client.address.clone();
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    let ts_a_before: u64 = env.ledger().timestamp();
    client.register_offering(&admin, &dev_a, &oid(&env, "a"), &meta(&env));
    let ts_a_after: u64 = env.ledger().timestamp();

    // Advance time before dev B registers.
    advance(&env, 10);
    let ts_b_before: u64 = env.ledger().timestamp();
    client.register_offering(&admin, &dev_b, &oid(&env, "b"), &meta(&env));
    let ts_b_after: u64 = env.ledger().timestamp();

    env.as_contract(&registry_id, || {
        let ts_a = admin::last_developer_action(&env, &dev_a)
            .expect("dev_a must have a cooldown record");
        let ts_b = admin::last_developer_action(&env, &dev_b)
            .expect("dev_b must have a cooldown record");

        // Each timestamp falls within the ledger range for that registration.
        assert!(
            ts_a >= ts_a_before && ts_a <= ts_a_after,
            "dev_a timestamp out of range: {ts_a}"
        );
        assert!(
            ts_b >= ts_b_before && ts_b <= ts_b_after,
            "dev_b timestamp out of range: {ts_b}"
        );

        // The two timestamps are different (dev B registered later).
        assert_ne!(ts_a, ts_b, "cooldown timestamps must be independent");
    });
}

/// Writing the cooldown for developer A does not create or modify the
/// `DeveloperCooldown` storage entry for developer B.
#[test]
fn cooldown_storage_key_write_does_not_cross_contaminate() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let registry_id = client.address.clone();
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    // Only dev A registers.
    client.register_offering(&admin, &dev_a, &oid(&env, "a-only"), &meta(&env));

    env.as_contract(&registry_id, || {
        // Dev A has a record.
        assert!(
            admin::last_developer_action(&env, &dev_a).is_some(),
            "dev_a must have a cooldown record after registering"
        );
        // Dev B's key must remain absent — no cross-contamination.
        assert!(
            admin::last_developer_action(&env, &dev_b).is_none(),
            "dev_b must NOT have a cooldown record after dev_a registers"
        );
    });
}

// ---------------------------------------------------------------------------
// 5. Authorization / security boundaries are preserved
// ---------------------------------------------------------------------------

/// An unauthenticated call is rejected before cooldown is checked.
/// (Tests that cooldown enforcement does not bypass `require_auth`.)
#[test]
fn cooldown_unauth_caller_rejected_before_cooldown_check() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    // No auth mocked — must fail with an auth error, not a cooldown error.
    env.set_auths(&[]);
    let result = client.try_register_offering(&admin, &dev, &oid(&env, "unauth"), &meta(&env));
    assert!(
        result.is_err(),
        "unauthenticated call must be rejected regardless of cooldown state"
    );
}

/// A non-admin authenticated caller is rejected with `Unauthorized`, not with
/// a cooldown error. The order admin-check → cooldown-check is preserved.
#[test]
fn cooldown_non_admin_rejected_before_cooldown_check() {
    let env = Env::default();
    let (_admin, client) = setup(&env);
    let non_admin = Address::generate(&env);
    let dev = Address::generate(&env);

    env.mock_all_auths();
    // non_admin is authenticated but is not the registry admin.
    let result =
        client.try_register_offering(&non_admin, &dev, &oid(&env, "nonauth"), &meta(&env));
    assert!(
        matches!(result, Err(Ok(RegistryError::Unauthorized))),
        "non-admin must get Unauthorized, not a cooldown error, got {result:?}"
    );
}

/// Cooldown rejection does not persist the offering or update the developer's
/// cooldown timestamp — the failed attempt consumes no cooldown credit.
#[test]
fn cooldown_rejection_does_not_update_timestamp() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let registry_id = client.address.clone();
    let dev = Address::generate(&env);

    // First registration sets the initial timestamp.
    client.register_offering(&admin, &dev, &oid(&env, "first"), &meta(&env));

    // Capture the timestamp set by the first registration into a Cell so the
    // closure can write it without requiring a mutable borrow of the outer
    // scope (as_contract does not support returning values in this repo).
    let ts_cell = std::cell::Cell::new(0u64);
    env.as_contract(&registry_id, || {
        ts_cell.set(
            admin::last_developer_action(&env, &dev)
                .expect("must exist after first registration"),
        );
    });
    let ts_after_first = ts_cell.get();

    // Advance (but not past the cooldown window).
    advance(&env, admin::COOLDOWN_SECONDS / 2);

    // Rejected attempt.
    let _ = client.try_register_offering(&admin, &dev, &oid(&env, "rejected"), &meta(&env));

    // Timestamp must not have changed after a rejected attempt.
    env.as_contract(&registry_id, || {
        let ts_after_rejection = admin::last_developer_action(&env, &dev)
            .expect("must still exist after rejection");
        assert_eq!(
            ts_after_first, ts_after_rejection,
            "a failed registration must not update the cooldown timestamp"
        );
    });

    assert_eq!(client.registered_count(), 1);
}

// ---------------------------------------------------------------------------
// 6. register_offering_with_gate enforces the same per-developer cooldown
// ---------------------------------------------------------------------------

/// The gated variant also enforces the per-developer cooldown: same developer,
/// sufficient balance, but within the cooldown window → `AdminCooldownActive`.
#[test]
fn cooldown_with_gate_same_developer_rejected_within_window() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    // Mint enough tokens for the developer.
    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner);
    let token_addr = sac.address();
    let token_admin = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
    token_admin.mint(&dev, &10_000);

    // First gated registration succeeds.
    client.register_offering_with_gate(
        &admin,
        &dev,
        &token_addr,
        &100i128,
        &oid(&env, "gate-first"),
        &meta(&env),
    );
    assert_eq!(client.registered_count(), 1);

    // Advance less than one full cooldown period.
    advance(&env, admin::COOLDOWN_SECONDS - 1);

    // Second attempt within the window must be blocked.
    let result = client.try_register_offering_with_gate(
        &admin,
        &dev,
        &token_addr,
        &100i128,
        &oid(&env, "gate-second"),
        &meta(&env),
    );
    assert!(
        matches!(result, Err(Ok(RegistryError::AdminCooldownActive))),
        "gated path: expected AdminCooldownActive within window, got {result:?}"
    );
    assert_eq!(client.registered_count(), 1);
    assert!(!client.is_offering_registered(&oid(&env, "gate-second")));
}

/// Two different developers can use `register_offering_with_gate`
/// concurrently (both within the same hour).
#[test]
fn cooldown_with_gate_different_developers_concurrent() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);

    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner);
    let token_addr = sac.address();
    let token_admin = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
    token_admin.mint(&dev_a, &10_000);
    token_admin.mint(&dev_b, &10_000);

    client.register_offering_with_gate(
        &admin,
        &dev_a,
        &token_addr,
        &100i128,
        &oid(&env, "gate-a"),
        &meta(&env),
    );
    client.register_offering_with_gate(
        &admin,
        &dev_b,
        &token_addr,
        &100i128,
        &oid(&env, "gate-b"),
        &meta(&env),
    );

    assert_eq!(client.registered_count(), 2);
    assert!(client.is_offering_registered(&oid(&env, "gate-a")));
    assert!(client.is_offering_registered(&oid(&env, "gate-b")));
}

// ---------------------------------------------------------------------------
// 7. No partial state on cooldown rejection
// ---------------------------------------------------------------------------

/// After a cooldown rejection the count is unchanged, the offering is not
/// persisted, and the existing cooldown timestamp is not updated.
#[test]
fn cooldown_rejection_leaves_no_partial_state() {
    let env = Env::default();
    let (admin, client) = setup(&env);
    let dev = Address::generate(&env);

    client.register_offering(&admin, &dev, &oid(&env, "ok"), &meta(&env));
    assert_eq!(client.registered_count(), 1);
    assert!(client.is_offering_registered(&oid(&env, "ok")));

    // Attempt a second registration while blocked.
    advance(&env, admin::COOLDOWN_SECONDS / 2);
    let bad_oid = oid(&env, "blocked");
    let result = client.try_register_offering(&admin, &dev, &bad_oid, &meta(&env));
    assert!(matches!(result, Err(Ok(RegistryError::AdminCooldownActive))));

    // Registry state is exactly as before the rejected attempt.
    assert_eq!(client.registered_count(), 1);
    assert!(!client.is_offering_registered(&bad_oid));
    assert!(client.is_offering_registered(&oid(&env, "ok")));
}
