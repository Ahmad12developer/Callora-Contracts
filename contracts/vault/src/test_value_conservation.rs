//! Value-conservation regression tests for the vault's natural-settlement path.
//!
//! These tests pin the behaviour required by
//! [Issue #1048 — "Conserve value across cancel, stop, and natural settlement"]:
//!
//! 1. **Authorization & lifecycle preconditions are checked before any value
//!    or state mutation** (`Unauthorized`, `Paused` leave tracked balance,
//!    on-ledger USDC, and events unchanged).
//! 2. **Successful execution changes each relevant state exactly once and
//!    rolls back atomically on failure** — a failed batch (insufficient
//!    balance) mutates nothing, and a successful batch conserves value
//!    (`Δ tracked balance == Σ items == Δ settlement USDC`).
//! 3. **Arithmetic, boundaries, and batch limits are safe for extreme inputs**
//!    (`BatchEmpty`, `BatchTooLarge`, `AmountNotPositive`, `BelowMinDeposit`,
//!    `ExceedsMaxDeduct`, overflow-safe total accumulation).
//! 4. **Retries, unauthorized callers, boundaries, concurrency, and failed
//!    transactions** are exercised.
//!
//! The vault's settlement interop is a no-op stub in unit tests, but the USDC
//! transfer to the settlement address is real, so value conservation is
//! verified directly against the on-ledger token balances.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{token, Address, Env, Error, InvokeError, Vec};

use super::*;
use callora_settlement::CalloraSettlement;

/// True if the `try_*` wrapper returned a specific vault error code.
///
/// The Soroban `try_*` client returns `Result<Result<V, CE>, Result<E,
/// InvokeError>>`; a contract-level `Err(VaultError)` surfaces as the outer
/// `Err` wrapping the inner `Ok(e)` (the established convention across this
/// workspace's tests).
fn is_vault_err<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
    expected: u32,
) -> bool {
    match result {
        Err(Ok(e)) => e.into().get_code() == expected,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    let token_client = token::Client::new(env, &addr);
    let admin_client = token::StellarAssetClient::new(env, &addr);
    (addr.clone(), token_client, admin_client)
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

/// Register and initialize the real settlement contract so it can hold USDC.
fn create_settlement(env: &Env, admin: &Address, vault_address: &Address) -> Address {
    let settlement_address = env.register(CalloraSettlement, ());
    let settlement_client =
        callora_settlement::CalloraSettlementClient::new(env, &settlement_address);
    env.mock_all_auths();
    settlement_client.init(admin, vault_address);
    settlement_address
}

/// Snapshot of every value-bearing sink we need to assert conservation across.
struct ConservationSnapshot {
    vault_tracked: i128,
    vault_usdc: i128,
    settlement_usdc: i128,
}

fn snapshot(
    usdc: &token::Client<'_>,
    vault_addr: &Address,
    client: &CalloraVaultClient<'_>,
    settlement_addr: &Address,
) -> ConservationSnapshot {
    ConservationSnapshot {
        vault_tracked: client.balance(),
        vault_usdc: usdc.balance(vault_addr),
        settlement_usdc: usdc.balance(settlement_addr),
    }
}

/// Initialize a funded vault with a configured settlement address.
///
/// Returns `(client, owner, usdc_client, usdc_admin_client, settlement_addr)`.
#[allow(clippy::type_complexity)]
fn setup_funded_vault(
    env: &Env,
    tracked: i128,
    on_ledger: i128,
    max_deduct: i128,
) -> (
    CalloraVaultClient<'_>,
    Address,
    token::Client<'_>,
    token::StellarAssetClient<'_>,
    Address,
) {
    let (client, owner, usdc, usdc_admin, settlement) =
        setup_funded_vault_with_min(env, tracked, on_ledger, max_deduct, 1i128);
    (client, owner, usdc, usdc_admin, settlement)
}

/// Like [`setup_funded_vault`] but with a custom minimum deposit threshold.
fn setup_funded_vault_with_min(
    env: &Env,
    tracked: i128,
    on_ledger: i128,
    max_deduct: i128,
    min_deposit: i128,
) -> (
    CalloraVaultClient<'_>,
    Address,
    token::Client<'_>,
    token::StellarAssetClient<'_>,
    Address,
) {
    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(env, &owner);
    let settlement = create_settlement(env, &owner, &vault_addr);

    env.mock_all_auths();
    // Pre-fund both the tracked balance (deducted down) and the on-ledger USDC
    // (actually transferred out).
    client.init(
        &owner,
        &usdc,
        &Some(tracked),
        &Some(owner.clone()),
        &Some(min_deposit),
        &None::<Address>,
        &Some(max_deduct),
        &Some(settlement.clone()),
    );
    usdc_admin.mint(&vault_addr, &on_ledger);

    (client, owner, usdc_client, usdc_admin, settlement)
}

fn setup_vault_with_optional_settlement(
    env: &Env,
    settlement: Option<Address>,
) -> (CalloraVaultClient<'_>, Address, token::Client<'_>) {
    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(1_000i128),
        &Some(owner.clone()),
        &Some(1i128),
        &None::<Address>,
        &Some(1_000i128),
        &settlement,
    );
    usdc_admin.mint(&vault_addr, &1_000i128);

    (client, owner, usdc_client)
}

fn items_from(env: &Env, amounts: &[i128]) -> Vec<(i128, u64)> {
    let mut v: Vec<(i128, u64)> = Vec::new(env);
    for (i, a) in amounts.iter().enumerate() {
        v.push_back((*a, i as u64));
    }
    v
}

// ---------------------------------------------------------------------------
// 3. Boundaries / batch limits / arithmetic safety
// ---------------------------------------------------------------------------

#[test]
fn batch_deduct_empty_is_rejected_and_mutates_nothing() {
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 1_000, 1_000, 1_000);
    env.mock_all_auths();
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    let items: Vec<(i128, u64)> = Vec::new(&env);
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::BatchEmpty as u32),
        "empty batch must be BatchEmpty"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn batch_deduct_too_large_is_rejected() {
    let env = Env::default();
    let (client, owner, usdc, _, settlement) =
        setup_funded_vault(&env, i128::MAX, i128::MAX, i128::MAX);
    env.mock_all_auths();
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    // MAX_BATCH_SIZE + 1 items, each 1 stroop.
    let mut items: Vec<(i128, u64)> = Vec::new(&env);
    for i in 0..(MAX_BATCH_SIZE + 1) {
        items.push_back((1i128, i as u64));
    }
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::BatchTooLarge as u32),
        "oversized batch must be BatchTooLarge"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn batch_deduct_max_size_is_accepted_and_conserves_value() {
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 100_000, 100_000, 10_000);
    env.mock_all_auths();
    let tracked_before = client.balance();
    let vault_usdc_before = usdc.balance(&client.address);
    let settlement_usdc_before = usdc.balance(&settlement);

    let mut items: Vec<(i128, u64)> = Vec::new(&env);
    for i in 0..MAX_BATCH_SIZE {
        items.push_back((50i128, i as u64));
    }
    let total: i128 = 50 * MAX_BATCH_SIZE as i128;
    assert!(client.try_batch_deduct(&owner, &items).is_ok());

    // Δ tracked == total and Δ settlement USDC == total → value conserved.
    assert_eq!(client.balance(), tracked_before - total);
    assert_eq!(usdc.balance(&client.address), vault_usdc_before - total);
    assert_eq!(usdc.balance(&settlement), settlement_usdc_before + total);
}

#[test]
fn batch_deduct_item_below_min_is_rejected() {
    let env = Env::default();
    // min_deposit = 5, max_deduct = 1000; item of 4 is below the threshold.
    let (client, owner, _usdc, _, _settlement) =
        setup_funded_vault_with_min(&env, 1_000, 1_000, 1000, 5i128);
    env.mock_all_auths();
    let items = items_from(&env, &[4]);
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::BelowMinDeposit as u32),
        "item below min must be BelowMinDeposit"
    );
}

#[test]
fn batch_deduct_item_above_max_is_rejected() {
    let env = Env::default();
    let (client, owner, _usdc, _, _settlement) = setup_funded_vault(&env, 1_000, 1_000, 100);
    env.mock_all_auths();
    let items = items_from(&env, &[101]);
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::ExceedsMaxDeduct as u32),
        "item above max must be ExceedsMaxDeduct"
    );
}

#[test]
fn batch_deduct_non_positive_item_is_rejected() {
    let env = Env::default();
    let (client, owner, _usdc, _, _settlement) = setup_funded_vault(&env, 1_000, 1_000, 1_000);
    env.mock_all_auths();

    for amount in [0i128, -1i128] {
        let items = items_from(&env, &[amount]);
        let res = client.try_batch_deduct(&owner, &items);
        assert!(
            is_vault_err(res, VaultError::AmountNotPositive as u32),
            "amount {amount} must be AmountNotPositive"
        );
    }
}

#[test]
fn batch_deduct_tiny_then_huge_within_cap_succeeds() {
    // Extreme-but-valid boundary: a tiny item plus a huge item both inside the
    // per-item cap must succeed and conserve value.
    let env = Env::default();
    // max_deduct = i128::MAX avoids per-item caps interfering.
    let (client, owner, usdc, _, settlement) =
        setup_funded_vault(&env, i128::MAX, i128::MAX, i128::MAX);
    env.mock_all_auths();

    let items = items_from(&env, &[1i128, i128::MAX - 1]);
    assert!(client.try_batch_deduct(&owner, &items).is_ok());
    assert_eq!(client.balance(), 0);
    assert_eq!(usdc.balance(&settlement), i128::MAX);
}

#[test]
fn batch_deduct_total_aggregation_is_checked() {
    // Two items whose per-item cap allows them but whose summed total exceeds
    // the tracked balance must fail with an overflow/insufficient error before
    // transferring anything. Here each item is below max_deduct but together
    // they exceed the balance → InsufficientBalance, no mutation.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 100, 100, 1_000);
    env.mock_all_auths();
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    let items = items_from(&env, &[70, 70]); // total 140 > 100
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::InsufficientBalance as u32),
        "over-balance total must be InsufficientBalance"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn deduct_without_settlement_returns_error_before_mutation() {
    let env = Env::default();
    let (client, owner, usdc) = setup_vault_with_optional_settlement(&env, None);
    env.mock_all_auths();

    let result = client.try_deduct(&owner, &100i128, &1u64);

    assert!(is_vault_err(result, VaultError::SettlementNotSet as u32));
    assert_eq!(client.balance(), 1_000);
    assert_eq!(usdc.balance(&client.address), 1_000);
    // A failed top-level invocation rolls the event buffer back; no vault
    // event may survive it.
    assert!(env.events().all().is_empty());
}

#[test]
fn batch_deduct_without_settlement_returns_error_before_mutation() {
    let env = Env::default();
    let (client, owner, usdc) = setup_vault_with_optional_settlement(&env, None);
    env.mock_all_auths();
    let items = items_from(&env, &[100]);

    let result = client.try_batch_deduct(&owner, &items);

    assert!(is_vault_err(result, VaultError::SettlementNotSet as u32));
    assert_eq!(client.balance(), 1_000);
    assert_eq!(usdc.balance(&client.address), 1_000);
    // A failed top-level invocation rolls the event buffer back; no vault
    // event may survive it.
    assert!(env.events().all().is_empty());
}

#[test]
fn deduct_and_batch_deduct_without_usdc_return_not_initialized() {
    let env = Env::default();
    let settlement = Address::generate(&env);
    let (client, owner, usdc) = setup_vault_with_optional_settlement(&env, Some(settlement));
    env.as_contract(&client.address, || {
        env.storage().instance().remove(&DataKey::UsdcToken);
    });
    env.mock_all_auths();
    let event_count = env.events().all().len();

    let deduct_result = client.try_deduct(&owner, &100i128, &1u64);
    assert!(is_vault_err(
        deduct_result,
        VaultError::NotInitialized as u32
    ));

    let items = items_from(&env, &[100]);
    let batch_result = client.try_batch_deduct(&owner, &items);
    assert!(is_vault_err(
        batch_result,
        VaultError::NotInitialized as u32
    ));
    assert_eq!(client.balance(), 1_000);
    assert_eq!(usdc.balance(&client.address), 1_000);
    assert_eq!(env.events().all().len(), event_count);
}

// ---------------------------------------------------------------------------
// 1 + 2. Authorization / lifecycle before mutation, atomic failure, retry
// ---------------------------------------------------------------------------

#[test]
fn batch_deduct_unauthorized_caller_is_rejected_before_mutation() {
    let env = Env::default();
    // auth caller = owner; an attacker must be rejected.
    let attacker = Address::generate(&env);
    let (client, _owner, usdc, _, settlement) = setup_funded_vault(&env, 1_000, 1_000, 1_000);
    env.mock_all_auths();
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    let items = items_from(&env, &[100]);
    let res = client.try_batch_deduct(&attacker, &items);
    assert!(
        is_vault_err(res, VaultError::Unauthorized as u32),
        "unauthorized caller must be rejected"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn batch_deduct_while_paused_is_rejected_before_mutation() {
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 1_000, 1_000, 1_000);
    env.mock_all_auths();
    // Lifecycle "stop": pause the vault (a value-stop circuit breaker).
    assert!(client.try_pause(&owner).is_ok());

    let before = snapshot(&usdc, &client.address, &client, &settlement);
    let items = items_from(&env, &[100]);
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::Paused as u32),
        "paused vault must reject batch_deduct"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn batch_deduct_single_deduct_route_conserves_value_end_to_end() {
    // The natural-settlement flow: funds leave the vault and land in the
    // settlement contract. Tracked balance and on-ledger balances move in lockstep.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 1_000, 1_000, 1_000);
    env.mock_all_auths();

    let tracked_before = client.balance();
    let vault_usdc_before = usdc.balance(&client.address);
    let settlement_usdc_before = usdc.balance(&settlement);

    assert!(client.try_deduct(&owner, &100i128, &1u64).is_ok());
    let tracked_after = client.balance();
    let vault_usdc_after = usdc.balance(&client.address);
    let settlement_usdc_after = usdc.balance(&settlement);

    // Every deducted unit is exactly accounted for.
    assert_eq!(tracked_after, tracked_before - 100);
    assert_eq!(vault_usdc_after, vault_usdc_before - 100);
    assert_eq!(settlement_usdc_after, settlement_usdc_before + 100);
}

// ---------------------------------------------------------------------------
// 4. Concurrency / failed transaction determinism
// ---------------------------------------------------------------------------

#[test]
fn concurrent_sources_cannot_double_spend_deficit() {
    // Two independent "callers" both attempt to deduct more than the single
    // tracked balance. Only the first success is possible; the second must
    // fail and conserve value. This simulates racing callers against a single
    // conserved pool.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 100, 100, 1_000);
    env.mock_all_auths();

    // First caller takes the whole 100.
    assert!(client
        .try_batch_deduct(&owner, &items_from(&env, &[100]))
        .is_ok());
    assert_eq!(client.balance(), 0);

    // Second caller now has nothing left → atomic failure, nothing changes.
    let settlement_before = usdc.balance(&settlement);
    let res = client.try_batch_deduct(&owner, &items_from(&env, &[1]));
    assert!(
        is_vault_err(res, VaultError::InsufficientBalance as u32),
        "over-draft after full spend must fail"
    );
    assert_eq!(client.balance(), 0);
    assert_eq!(usdc.balance(&settlement), settlement_before);
}

#[test]
fn failed_batch_deduct_leaves_every_event_sink_unchanged() {
    // A batch with a mix of a valid and an out-of-range item fails as a whole;
    // no partial settlement, no partial deduction, no events.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 1_000, 1_000, 100);
    env.mock_all_auths();
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    // Second item (200) exceeds the per-item cap → entire batch rejects.
    let items = items_from(&env, &[50, 200]);
    let res = client.try_batch_deduct(&owner, &items);
    assert!(
        is_vault_err(res, VaultError::ExceedsMaxDeduct as u32),
        "mixed out-of-range batch must fail as a whole"
    );

    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
    assert_eq!(before.settlement_usdc, after.settlement_usdc);
}

#[test]
fn retry_of_failed_batch_does_not_corrupt_value() {
    // A caller may retry a rejected batch with corrected inputs; neither the
    // rejected attempt nor the retry drifts bookkeeping.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 500, 500, 100);
    env.mock_all_auths();

    // Failed attempt: includes an over-cap item.
    assert!(client
        .try_batch_deduct(&owner, &items_from(&env, &[100, 100, 500]))
        .is_err());

    // Corrected retry succeeds.
    assert!(client
        .try_batch_deduct(&owner, &items_from(&env, &[100, 100, 100]))
        .is_ok());
    assert_eq!(client.balance(), 200);
    assert_eq!(usdc.balance(&settlement), 300);
}

// ---------------------------------------------------------------------------
// 4. "Cancel" lifecycle: cancelling a pending sweep conserves value
// ---------------------------------------------------------------------------

#[test]
fn cancel_sweep_before_execution_conserves_value() {
    // Sweep is the vault's cold-storage/surplus "settlement"; it is timelocked
    // and cancellable. Cancelling a pending sweep must not move any value.
    let env = Env::default();
    let (client, owner, usdc, _, settlement) = setup_funded_vault(&env, 100, 140, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    assert!(client
        .try_propose_sweep(&owner, &recipient, &40i128)
        .is_ok());
    let before = snapshot(&usdc, &client.address, &client, &settlement);

    // Cancel before the timelock elapses.
    assert!(client.try_cancel_sweep(&owner).is_ok());

    // Executing now must fail (proposal cleared) and nothing was transferred.
    assert!(is_vault_err(
        client.try_execute_sweep(&owner),
        VaultError::ProposalNotFound as u32
    ));
    let after = snapshot(&usdc, &client.address, &client, &settlement);
    assert_eq!(before.vault_tracked, after.vault_tracked);
    assert_eq!(before.vault_usdc, after.vault_usdc);
}

#[test]
fn execute_sweep_conserves_value_to_recipient() {
    let env = Env::default();
    let (client, owner, usdc, _, _settlement) = setup_funded_vault(&env, 100, 140, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    assert!(client
        .try_propose_sweep(&owner, &recipient, &40i128)
        .is_ok());
    let vault = client.address.clone();

    // Advance ledger past the default timelock so the sweep may execute.
    // The default timelock window is 172800s; jump well past it.
    env.ledger().set_timestamp(1_000_000);
    assert!(client.try_execute_sweep(&owner).is_ok()); // 40 moved to recipient, tracked balance untouched by sweep (sweep recovers
                                                       // on-ledger surplus, it does not touch `DataKey::Balance`).
    assert_eq!(usdc.balance(&recipient), 40);
    assert_eq!(usdc.balance(&vault), 100);
    assert_eq!(client.balance(), 100);
}

#[test]
fn propose_sweep_fails_when_amount_exceeds_current_surplus() {
    let env = Env::default();
    // Tracked = 100, on-ledger = 140 -> surplus = 40.
    let (client, owner, _usdc, _, _settlement) = setup_funded_vault(&env, 100, 140, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    // Amount 41 exceeds surplus of 40 -> must fail with InsufficientBalance.
    let res = client.try_propose_sweep(&owner, &recipient, &41i128);
    assert!(is_vault_err(res, VaultError::InsufficientBalance as u32));
    assert!(client.get_pending_sweep().is_none());
}

#[test]
fn execute_sweep_fails_when_surplus_drops_below_amount_after_proposal() {
    let env = Env::default();
    // Tracked = 100, on-ledger = 140 -> surplus = 40.
    let (client, owner, usdc, _, _settlement) = setup_funded_vault(&env, 100, 140, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    assert!(client
        .try_propose_sweep(&owner, &recipient, &40i128)
        .is_ok());

    // Reduce surplus before execution by distributing 10 surplus USDC to another address
    let other = Address::generate(&env);
    client.distribute(&owner, &other, &10i128);
    // Now on-ledger = 130, tracked = 100 -> surplus = 30 < 40.

    env.ledger().set_timestamp(1_000_000);
    let res = client.try_execute_sweep(&owner);
    assert!(is_vault_err(res, VaultError::InsufficientBalance as u32));

    // Proposal must NOT be consumed, tracked balance untouched, surplus untouched
    assert!(client.get_pending_sweep().is_some());
    assert_eq!(client.balance(), 100);
    assert_eq!(usdc.balance(&client.address), 130);
}

#[test]
fn successful_sweep_leaves_balance_untouched() {
    let env = Env::default();
    // Tracked = 500, on-ledger = 800 -> surplus = 300.
    let (client, owner, usdc, _, _settlement) = setup_funded_vault(&env, 500, 800, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    assert!(client
        .try_propose_sweep(&owner, &recipient, &200i128)
        .is_ok());

    env.ledger().set_timestamp(1_000_000);
    assert!(client.try_execute_sweep(&owner).is_ok());

    // Tracked balance must remain exactly 500 (untouched).
    assert_eq!(client.balance(), 500);
    // On-ledger USDC balance drops by 200 (800 -> 600).
    assert_eq!(usdc.balance(&client.address), 600);
    assert_eq!(usdc.balance(&recipient), 200);
}

#[test]
fn withdrawal_between_proposal_and_execution_specifically() {
    let env = Env::default();
    // Tracked = 100, on-ledger = 140 -> surplus = 40.
    let (client, owner, usdc, _, _settlement) = setup_funded_vault(&env, 100, 140, 1_000);
    env.mock_all_auths();

    let recipient = Address::generate(&env);
    // Propose sweeping the maximum surplus (40).
    assert!(client
        .try_propose_sweep(&owner, &recipient, &40i128)
        .is_ok());

    // Intervening operation reduces available surplus (e.g. distribute 15 surplus USDC).
    let drain_addr = Address::generate(&env);
    client.distribute(&owner, &drain_addr, &15i128);
    // Surplus is now 125 - 100 = 25.

    // Advance time past timelock window.
    env.ledger().set_timestamp(1_000_000);

    // Execution must fail without consuming proposal or modifying tracked balance.
    let res = client.try_execute_sweep(&owner);
    assert!(is_vault_err(res, VaultError::InsufficientBalance as u32));
    assert!(client.get_pending_sweep().is_some());
    assert_eq!(client.balance(), 100);
    assert_eq!(usdc.balance(&client.address), 125);
}
