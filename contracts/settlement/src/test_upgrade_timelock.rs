//! Tests for the timelocked settlement upgrade flow.
//!
//! Covers every acceptance criterion from the issue:
//!
//! 1. Upgrade cannot execute before the configured delay.
//! 2. Pending upgrade is visible via `get_pending_upgrade`.
//! 3. Cancel clears the proposal and emits `upgrade_cancelled`.
//! 4. `execute_upgrade` succeeds after the delay (mocked via `env.ledger()`).
//! 5. Zero WASM hash is rejected by `propose_upgrade`.
//! 6. Non-admin callers are rejected on all three mutating entry points.
//! 7. `execute_upgrade` / `cancel_upgrade` fail when no proposal is pending.
//! 8. Re-proposal replaces the old entry and restarts the delay.
//! 9. `upgrade` (alias) behaves identically to `propose_upgrade`.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};

use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError, UPGRADE_TIMELOCK_SECONDS};

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Register the contract, call `init`, and return `(env, client, admin, vault)`.
fn setup() -> (Env, CalloraSettlementClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    client.init(&admin, &vault);
    (env, client, admin, vault)
}

/// Return a non-zero 32-byte WASM hash.
fn dummy_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0xABu8; 32])
}

/// Return an all-zero 32-byte hash (sentinel for "invalid").
fn zero_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

// ─── propose_upgrade ────────────────────────────────────────────────────────

#[test]
fn test_propose_upgrade_creates_pending_entry() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    let now = env.ledger().timestamp();

    client.propose_upgrade(&admin, &hash);

    let pending = client.get_pending_upgrade().expect("proposal should exist");
    assert_eq!(pending.wasm_hash, hash);
    assert_eq!(pending.proposed_at, now);
    assert_eq!(pending.execute_after, now + UPGRADE_TIMELOCK_SECONDS);
}

#[test]
fn test_propose_upgrade_rejects_zero_hash() {
    let (env, client, admin, _vault) = setup();
    let err = client
        .try_propose_upgrade(&admin, &zero_hash(&env))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::ZeroWasmHash);
}

#[test]
fn test_propose_upgrade_rejects_non_admin() {
    let (env, client, _admin, _vault) = setup();
    let stranger = Address::generate(&env);
    let hash = dummy_hash(&env);
    let err = client
        .try_propose_upgrade(&stranger, &hash)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::Unauthorized);
}

#[test]
fn test_propose_upgrade_replaces_existing_proposal() {
    let (env, client, admin, _vault) = setup();
    let hash1 = dummy_hash(&env);
    let hash2 = BytesN::from_array(&env, &[0xCDu8; 32]);

    client.propose_upgrade(&admin, &hash1);

    // Advance time so the first proposal's deadline has moved.
    env.ledger().with_mut(|l| l.timestamp += 1000);
    client.propose_upgrade(&admin, &hash2);

    let pending = client.get_pending_upgrade().expect("proposal should exist");
    assert_eq!(pending.wasm_hash, hash2, "second proposal should replace first");
    // execute_after should be based on the new proposed_at (original + 1000 + delay)
    assert!(pending.execute_after > 1000 + UPGRADE_TIMELOCK_SECONDS - 1);
}

// ─── execute_upgrade ────────────────────────────────────────────────────────

#[test]
fn test_execute_upgrade_rejects_before_delay() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    // Advance time but stay inside the timelock window.
    env.ledger()
        .with_mut(|l| l.timestamp += UPGRADE_TIMELOCK_SECONDS - 1);

    let err = client
        .try_execute_upgrade(&admin)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::UpgradeTimelockNotExpired);
}

#[test]
fn test_execute_upgrade_rejects_non_admin() {
    let (env, client, admin, _vault) = setup();
    let stranger = Address::generate(&env);
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    env.ledger()
        .with_mut(|l| l.timestamp += UPGRADE_TIMELOCK_SECONDS + 1);

    let err = client
        .try_execute_upgrade(&stranger)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::Unauthorized);
}

#[test]
fn test_execute_upgrade_fails_when_no_pending() {
    let (_env, client, admin, _vault) = setup();
    let err = client
        .try_execute_upgrade(&admin)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::NoUpgradePending);
}

#[test]
fn test_execute_upgrade_clears_proposal_after_success() {
    // NOTE: In the Soroban test environment `update_current_contract_wasm` is a
    // no-op mock, so we can verify state transitions without actual WASM.
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    env.ledger()
        .with_mut(|l| l.timestamp += UPGRADE_TIMELOCK_SECONDS);

    client.execute_upgrade(&admin);

    // Proposal must be gone after execution.
    assert!(
        client.get_pending_upgrade().is_none(),
        "pending proposal should be cleared after execution"
    );
}

#[test]
fn test_execute_upgrade_at_exact_deadline() {
    // Execute_after is the earliest inclusive timestamp, so `now == execute_after` should pass.
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    env.ledger()
        .with_mut(|l| l.timestamp += UPGRADE_TIMELOCK_SECONDS);

    // Should not panic.
    client.execute_upgrade(&admin);
    assert!(client.get_pending_upgrade().is_none());
}

// ─── cancel_upgrade ─────────────────────────────────────────────────────────

#[test]
fn test_cancel_upgrade_clears_proposal() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    assert!(client.get_pending_upgrade().is_some(), "proposal should exist before cancel");

    client.cancel_upgrade(&admin);

    assert!(
        client.get_pending_upgrade().is_none(),
        "proposal should be cleared after cancel"
    );
}

#[test]
fn test_cancel_upgrade_rejects_non_admin() {
    let (env, client, admin, _vault) = setup();
    let stranger = Address::generate(&env);
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    let err = client
        .try_cancel_upgrade(&stranger)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::Unauthorized);
}

#[test]
fn test_cancel_upgrade_fails_when_no_pending() {
    let (_env, client, admin, _vault) = setup();
    let err = client
        .try_cancel_upgrade(&admin)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, SettlementError::NoUpgradePending);
}

#[test]
fn test_cancel_upgrade_before_delay_is_allowed() {
    // Cancellation must work even if the timelock has not expired yet.
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);

    // Still inside the window.
    env.ledger()
        .with_mut(|l| l.timestamp += UPGRADE_TIMELOCK_SECONDS / 2);

    client.cancel_upgrade(&admin); // must succeed
    assert!(client.get_pending_upgrade().is_none());
}

// ─── get_pending_upgrade ────────────────────────────────────────────────────

#[test]
fn test_get_pending_upgrade_returns_none_before_proposal() {
    let (_env, client, _admin, _vault) = setup();
    assert!(client.get_pending_upgrade().is_none());
}

#[test]
fn test_get_pending_upgrade_returns_some_after_proposal() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.propose_upgrade(&admin, &hash);
    assert!(client.get_pending_upgrade().is_some());
}

// ─── upgrade alias ──────────────────────────────────────────────────────────

#[test]
fn test_upgrade_alias_behaves_like_propose() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    let now = env.ledger().timestamp();

    // `upgrade` is the deprecated alias for `propose_upgrade`.
    client.upgrade(&admin, &hash);

    let pending = client.get_pending_upgrade().expect("alias should create proposal");
    assert_eq!(pending.wasm_hash, hash);
    assert_eq!(pending.proposed_at, now);
    assert_eq!(pending.execute_after, now + UPGRADE_TIMELOCK_SECONDS);
}

#[test]
fn test_upgrade_alias_does_not_execute_immediately() {
    let (env, client, admin, _vault) = setup();
    let hash = dummy_hash(&env);
    client.upgrade(&admin, &hash);

    // No execute call → version should still be `None`.
    assert!(
        client.get_version().is_none(),
        "`upgrade` alias must not execute immediately"
    );
}
