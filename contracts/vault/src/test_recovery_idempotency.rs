/// Tests for recovery-mode idempotency after interrupted administration.
///
/// Verifies that when a vault is already in a given state, re-executing
/// admin operations does not arm the admin cooldown unnecessarily, emit
/// duplicate events, or block recovery mode.
extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

use super::*;

use callora_settlement::CalloraSettlement;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

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

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

fn create_settlement(env: &Env, admin: &Address, vault_address: &Address) -> Address {
    let settlement_address = env.register(CalloraSettlement, ());
    let settlement_client =
        callora_settlement::CalloraSettlementClient::new(env, &settlement_address);
    env.mock_all_auths();
    settlement_client.init(admin, vault_address);
    settlement_address
}

fn fund_vault(
    usdc_admin_client: &token::StellarAssetClient,
    vault_address: &Address,
    amount: i128,
) {
    usdc_admin_client.mint(vault_address, &amount);
}

fn set_ledger_timestamp(env: &Env, new_timestamp: u64) {
    env.ledger().set_timestamp(new_timestamp);
}

// ---------------------------------------------------------------------------
// pause() idempotency
// ---------------------------------------------------------------------------

/// Calling pause on an already-paused vault must succeed without emitting
/// a duplicate `vault_paused` event.
#[test]
fn pause_on_already_paused_is_idempotent() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );

    // First pause
    client.pause(&owner);
    assert!(client.is_paused());

    // Clear events from first pause
    env.events().all();

    // Second pause must succeed (idempotent)
    client.pause(&owner);
    assert!(client.is_paused());

    // No new `vault_paused` events should be emitted
    let events = env.events().all();
    let vault_paused_events: std::vec::Vec<_> = events
        .iter()
        .filter(|e| {
            if e.0 != vault_address || e.1.is_empty() {
                return false;
            }
            let t: Symbol = e.1.get(0).unwrap().into_val(&env);
            t == Symbol::new(&env, "vault_paused")
        })
        .collect();
    assert_eq!(
        vault_paused_events.len(),
        0,
        "second pause must not emit vault_paused"
    );
}

// ---------------------------------------------------------------------------
// unpause() idempotency
// ---------------------------------------------------------------------------

/// Calling unpause on an already-unpaused vault must succeed without
/// emitting a duplicate `vault_unpaused` event.
#[test]
fn unpause_on_already_unpaused_is_idempotent() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );

    // Vault starts unpaused
    assert!(!client.is_paused());

    // Clear events from init
    env.events().all();

    // Unpause must succeed (idempotent)
    client.unpause(&owner);
    assert!(!client.is_paused());

    // No new `vault_unpaused` events should be emitted
    let events = env.events().all();
    let vault_unpaused_events: std::vec::Vec<_> = events
        .iter()
        .filter(|e| {
            if e.0 != vault_address || e.1.is_empty() {
                return false;
            }
            let t: Symbol = e.1.get(0).unwrap().into_val(&env);
            t == Symbol::new(&env, "vault_unpaused")
        })
        .collect();
    assert_eq!(
        vault_unpaused_events.len(),
        0,
        "unpause on unpaused vault must not emit vault_unpaused"
    );
}

// ---------------------------------------------------------------------------
// pause/unpause round-trip
// ---------------------------------------------------------------------------

/// Pause → unpause → pause must work without event pollution.
#[test]
fn pause_unpause_pause_round_trip() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );

    // Pause
    client.pause(&owner);
    assert!(client.is_paused());

    // Unpause
    client.unpause(&owner);
    assert!(!client.is_paused());

    // Pause again
    client.pause(&owner);
    assert!(client.is_paused());

    // Unpause again
    client.unpause(&owner);
    assert!(!client.is_paused());
}

// ---------------------------------------------------------------------------
// execute_pause idempotency
// ---------------------------------------------------------------------------

/// When the vault is already paused (via direct `pause()`), calling
/// `execute_pause` on an expired proposal must clear the proposal
/// without arming the admin cooldown.
#[test]
fn execute_pause_on_already_paused_clears_proposal() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = create_settlement(&env, &admin, &vault_address);
    client.init(
        &admin,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );
    assert_eq!(client.get_admin(), admin);

    // Pause directly (not via timelock)
    client.pause(&admin);
    assert!(client.is_paused());

    // Propose a pause via timelock
    client.propose_pause(&admin);

    // Advance time past the timelock
    let current_ts = env.ledger().timestamp();
    let window = client.get_timelock_window();
    set_ledger_timestamp(&env, current_ts + window + 1);

    // execute_pause should clear the proposal without arming cooldown
    client.execute_pause(&admin);

    // Proposal should be cleared
    assert!(
        client.get_pending_pause().is_none(),
        "proposal should be cleared"
    );

    // Vault should still be paused (was already paused)
    assert!(client.is_paused());

    // Admin cooldown should NOT be armed (since we took the idempotent path)
    assert_eq!(
        client.admin_cooldown_remaining(),
        0,
        "cooldown should not be armed when vault was already paused"
    );
}

// ---------------------------------------------------------------------------
// Recovery operations work after interrupted administration
// ---------------------------------------------------------------------------

/// After pausing via direct `pause()`, withdraw still works (recovery mode).
#[test]
fn withdraw_works_after_direct_pause() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );

    client.pause(&owner);
    assert!(client.is_paused());

    // Withdraw should still work in recovery mode
    let remaining = client.withdraw(&200);
    assert_eq!(remaining, 800);
    assert_eq!(client.balance(), 800);
    assert_eq!(usdc_client.balance(&owner), 200);
}

/// After pausing via direct `pause()`, distribute still works (recovery mode).
#[test]
fn distribute_works_after_direct_pause() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let developer = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    let settlement = create_settlement(&env, &admin, &vault_address);
    client.init(
        &admin,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement),
    );

    client.pause(&admin);
    assert!(client.is_paused());

    // Distribute should still work in recovery mode
    client.distribute(&admin, &developer, &300);
    assert_eq!(usdc_client.balance(&developer), 300);
    assert_eq!(usdc_client.balance(&vault_address), 700);
}

// ---------------------------------------------------------------------------
// prune_processed_requests works during recovery mode
// ---------------------------------------------------------------------------

/// After pausing, prune_processed_requests should still work.
#[test]
fn prune_works_during_recovery_mode() {
    let env = Env::default();
    let (vault_addr, client, _, owner) = setup_vault(&env, 1_000);

    // Write a processed-request marker directly into persistent storage
    // to simulate a previously-processed deduct.  We do this because
    // the current `deduct` implementation doesn't store markers via
    // `mark_request_processed`; we only need the marker to test pruning.
    let rid: u64 = 42;
    let key = StorageKey::ProcessedRequest(rid);
    env.as_contract(&vault_addr, || {
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(
            &key,
            REQUEST_ID_BUMP_THRESHOLD,
            REQUEST_ID_BUMP_AMOUNT,
        );
    });
    assert!(client.is_request_processed(&rid));

    // Pause the vault
    client.pause(&owner);
    assert!(client.is_paused());

    // Prune should still work during pause (recovery mode)
    let mut ids = soroban_sdk::Vec::new(&env);
    ids.push_back(rid);
    client.prune_processed_requests(&owner, &ids);
    assert!(!client.is_request_processed(&rid));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_vault(env: &Env, balance: i128) -> (Address, CalloraVaultClient<'_>, Address, Address) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, _, usdc_admin) = create_usdc(env, &owner);
    usdc_admin.mint(&vault_addr, &balance);
    let settlement = create_settlement(env, &owner, &vault_addr);
    client.init(
        &owner,
        &usdc,
        &Some(balance),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &Some(settlement.clone()),
    );
    (vault_addr, client, settlement, owner)
}
