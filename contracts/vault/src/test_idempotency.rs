//! Request-id idempotency tests for `deduct` and `batch_deduct`.
//!
//! Revived from `test_idempotency.rs.broken` and adapted to the vault's `u64`
//! request-id API, where `0` is the documented "no idempotency" sentinel.
//!
//! # Coverage
//! - A non-zero `request_id` is recorded on the first successful deduct and a
//!   replay is rejected with `DuplicateRequestId` leaving balance unchanged.
//! - Distinct ids succeed independently; `0` is never deduplicated.
//! - Failed deducts (insufficient balance, paused) do not mark their id.
//! - `batch_deduct` rejects duplicates against storage *and* within the batch,
//!   atomically (no balance change, no markers written).
//! - `is_request_processed` reflects processed state.
//! - `prune_processed_requests` clears a marker so its id can be reused.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, Error, InvokeError, Vec};

use super::*;

use callora_settlement::CalloraSettlement;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True if a `try_*` wrapper surfaced the given vault error code.
fn is_vault_err<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
    expected: u32,
) -> bool {
    match result {
        Err(Ok(e)) => e.into().get_code() == expected,
        _ => false,
    }
}

/// Register a vault funded with `balance` tracked/on-ledger USDC and a real
/// settlement contract, returning `(client, owner)`.
fn setup_vault(env: &Env, balance: i128) -> (CalloraVaultClient<'_>, Address) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);

    let ca = env.register_stellar_asset_contract_v2(owner.clone());
    let usdc_addr = ca.address();
    let usdc_admin = token::StellarAssetClient::new(env, &usdc_addr);

    let settlement_addr = env.register(CalloraSettlement, ());
    let settlement_client = callora_settlement::CalloraSettlementClient::new(env, &settlement_addr);
    settlement_client.init(&owner, &vault_addr);

    usdc_admin.mint(&vault_addr, &balance);
    client.init(
        &owner,
        &usdc_addr,
        &Some(balance),
        &Some(owner.clone()),
        &Some(1i128),
        &None::<Address>,
        &Some(10_000i128),
        &Some(settlement_addr.clone()),
    );

    (client, owner)
}

fn items(env: &Env, pairs: &[(i128, u64)]) -> Vec<(i128, u64)> {
    let mut v: Vec<(i128, u64)> = Vec::new(env);
    for (amount, request_id) in pairs.iter() {
        v.push_back((*amount, *request_id));
    }
    v
}

// ---------------------------------------------------------------------------
// Error code stability
// ---------------------------------------------------------------------------

#[test]
fn duplicate_request_id_error_code_is_29() {
    assert_eq!(VaultError::DuplicateRequestId as u32, 29);
}

// ---------------------------------------------------------------------------
// deduct — single-call idempotency
// ---------------------------------------------------------------------------

#[test]
fn deduct_duplicate_request_id_rejected() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    // First call succeeds.
    client.deduct(&owner, &100i128, &7u64);
    assert_eq!(client.balance(), 900);

    // Retry with the same id is rejected and leaves balance unchanged.
    let result = client.try_deduct(&owner, &100i128, &7u64);
    assert!(
        is_vault_err(result, VaultError::DuplicateRequestId as u32),
        "duplicate request_id must be rejected"
    );
    assert_eq!(client.balance(), 900);
}

#[test]
fn deduct_retry_with_different_amount_still_rejected() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    client.deduct(&owner, &100i128, &8u64);
    let result = client.try_deduct(&owner, &50i128, &8u64);
    assert!(
        is_vault_err(result, VaultError::DuplicateRequestId as u32),
        "retry with a different amount must still be rejected"
    );
    assert_eq!(client.balance(), 900);
}

#[test]
fn deduct_distinct_request_ids_both_succeed() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    client.deduct(&owner, &100i128, &101u64);
    assert_eq!(client.balance(), 900);
    client.deduct(&owner, &200i128, &102u64);
    assert_eq!(client.balance(), 700);

    assert!(client.is_request_processed(&101u64));
    assert!(client.is_request_processed(&102u64));
}

#[test]
fn deduct_zero_request_id_is_never_deduplicated() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    // `0` is the documented "no idempotency" sentinel and may be reused.
    client.deduct(&owner, &100i128, &0u64);
    client.deduct(&owner, &100i128, &0u64);
    client.deduct(&owner, &100i128, &0u64);

    assert_eq!(client.balance(), 700);
    assert!(!client.is_request_processed(&0u64));
}

#[test]
fn deduct_failed_due_to_insufficient_balance_does_not_mark_id() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 50);

    let result = client.try_deduct(&owner, &100i128, &11u64);
    assert!(
        is_vault_err(result, VaultError::InsufficientBalance as u32),
        "expected insufficient balance"
    );
    assert!(
        !client.is_request_processed(&11u64),
        "failed deduct must not mark the request id"
    );
    assert_eq!(client.balance(), 50);
}

#[test]
fn deduct_failed_due_to_paused_does_not_mark_id() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 500);

    client.pause(&owner);
    assert!(client.is_paused());

    let result = client.try_deduct(&owner, &100i128, &12u64);
    assert!(
        is_vault_err(result, VaultError::Paused as u32),
        "expected paused error"
    );
    assert!(
        !client.is_request_processed(&12u64),
        "paused deduct must not mark the request id"
    );
}

// ---------------------------------------------------------------------------
// is_request_processed view
// ---------------------------------------------------------------------------

#[test]
fn is_request_processed_false_before_deduct() {
    let env = Env::default();
    let (client, _owner) = setup_vault(&env, 500);

    assert!(!client.is_request_processed(&999u64));
}

#[test]
fn is_request_processed_true_after_successful_deduct() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 500);

    client.deduct(&owner, &50i128, &21u64);

    assert!(
        client.is_request_processed(&21u64),
        "is_request_processed must be true after a successful deduct"
    );
}

#[test]
fn is_request_processed_false_for_different_id() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 500);

    client.deduct(&owner, &50i128, &31u64);

    assert!(client.is_request_processed(&31u64));
    assert!(!client.is_request_processed(&32u64));
}

// ---------------------------------------------------------------------------
// batch_deduct — idempotency
// ---------------------------------------------------------------------------

#[test]
fn batch_deduct_duplicate_request_id_rejected_atomically() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    // Mark an id with a single deduct first.
    client.deduct(&owner, &100i128, &41u64);
    assert_eq!(client.balance(), 900);

    // A batch that reuses the marked id must be rejected atomically.
    let batch = items(&env, &[(50, 41), (50, 0)]);
    let result = client.try_batch_deduct(&owner, &batch);
    assert!(
        is_vault_err(result, VaultError::DuplicateRequestId as u32),
        "batch reusing a processed id must be rejected"
    );
    assert_eq!(client.balance(), 900, "balance must not change");
}

#[test]
fn batch_deduct_two_items_same_new_id_rejected() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    // Two items share a brand-new id: the second is a within-batch duplicate.
    let batch = items(&env, &[(100, 51), (100, 51)]);
    let result = client.try_batch_deduct(&owner, &batch);
    assert!(
        is_vault_err(result, VaultError::DuplicateRequestId as u32),
        "two items sharing the same new id must be rejected"
    );
    assert_eq!(client.balance(), 1_000);
    assert!(
        !client.is_request_processed(&51u64),
        "a rejected batch must not mark its request ids"
    );
}

#[test]
fn batch_deduct_repeated_id_variants_rejected_atomically() {
    for seed in 0..16u64 {
        let env = Env::default();
        let (client, owner) = setup_vault(&env, 10_000);

        let batch_size = 2 + (seed as usize % (MAX_BATCH_SIZE as usize - 1));
        let dup_a = seed as usize % batch_size;
        let dup_b = (dup_a + 1) % batch_size; // distinct by construction
        let dup_id = 5_000 + seed;

        let mut batch: Vec<(i128, u64)> = Vec::new(&env);
        for i in 0..batch_size {
            let rid = if i == dup_a || i == dup_b {
                dup_id
            } else {
                10_000 + seed * 100 + i as u64
            };
            batch.push_back((10i128, rid));
        }

        let result = client.try_batch_deduct(&owner, &batch);
        assert!(
            is_vault_err(result, VaultError::DuplicateRequestId as u32),
            "seed {seed}: repeated id must be rejected"
        );
        assert_eq!(client.balance(), 10_000, "seed {seed}: balance unchanged");
        assert!(
            !client.is_request_processed(&dup_id),
            "seed {seed}: rejected batch must not mark the id"
        );
    }
}

#[test]
fn batch_deduct_distinct_ids_all_succeed_and_marked() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    let batch = items(&env, &[(100, 61), (200, 62), (50, 63)]);
    client.batch_deduct(&owner, &batch);
    assert_eq!(client.balance(), 650);

    assert!(client.is_request_processed(&61u64));
    assert!(client.is_request_processed(&62u64));
    assert!(client.is_request_processed(&63u64));
}

#[test]
fn batch_deduct_zero_ids_not_marked() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    let batch = items(&env, &[(100, 0), (200, 0)]);
    client.batch_deduct(&owner, &batch);
    assert_eq!(client.balance(), 700);

    // Nothing should have been marked.
    assert!(!client.is_request_processed(&0u64));
    assert!(!client.is_request_processed(&999u64));
}

#[test]
fn batch_deduct_failed_insufficient_balance_does_not_mark_ids() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 100);

    // Cumulative 120 > 100 tracked balance → whole batch fails.
    let batch = items(&env, &[(60, 71), (60, 72)]);
    let result = client.try_batch_deduct(&owner, &batch);
    assert!(
        is_vault_err(result, VaultError::InsufficientBalance as u32),
        "expected insufficient balance"
    );
    assert!(!client.is_request_processed(&71u64));
    assert!(!client.is_request_processed(&72u64));
    assert_eq!(client.balance(), 100);
}

#[test]
fn batch_deduct_mixed_ids_marks_only_nonzero_ids() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    let batch = items(&env, &[(100, 81), (50, 0), (75, 82)]);
    client.batch_deduct(&owner, &batch);
    assert_eq!(client.balance(), 775);

    assert!(client.is_request_processed(&81u64));
    assert!(client.is_request_processed(&82u64));

    // Retrying either processed id must fail...
    assert!(is_vault_err(
        client.try_deduct(&owner, &10i128, &81u64),
        VaultError::DuplicateRequestId as u32
    ));
    assert!(is_vault_err(
        client.try_deduct(&owner, &10i128, &82u64),
        VaultError::DuplicateRequestId as u32
    ));

    // ...while the sentinel id still goes through.
    client.deduct(&owner, &10i128, &0u64);
    assert_eq!(client.balance(), 765);
}

// ---------------------------------------------------------------------------
// prune_processed_requests — retention / reuse
// ---------------------------------------------------------------------------

#[test]
fn prune_processed_request_allows_reuse_and_emits_event() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    client.deduct(&owner, &100i128, &91u64);
    client.deduct(&owner, &100i128, &92u64);
    assert!(client.is_request_processed(&91u64));

    let before = env.events().all().len();
    let ids = soroban_sdk::vec![&env, 91u64];
    client.prune_processed_requests(&owner, &ids);

    // Exactly the pruned marker emits an event.
    assert_eq!(env.events().all().len(), before + 1);
    assert!(!client.is_request_processed(&91u64));
    assert!(client.is_request_processed(&92u64));

    // The id can now be reused.
    client.deduct(&owner, &10i128, &91u64);
    assert_eq!(client.balance(), 790);
}

#[test]
fn prune_ignores_unknown_ids() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    let ids = soroban_sdk::vec![&env, 777u64];
    // Pruning an unknown id is a no-op, not an error.
    client.prune_processed_requests(&owner, &ids);
}

#[test]
fn prune_allowed_during_pause() {
    let env = Env::default();
    let (client, owner) = setup_vault(&env, 1_000);

    client.deduct(&owner, &100i128, &93u64);
    client.pause(&owner);
    assert!(client.is_paused());

    let ids = soroban_sdk::vec![&env, 93u64];
    client.prune_processed_requests(&owner, &ids);
    assert!(!client.is_request_processed(&93u64));
}
