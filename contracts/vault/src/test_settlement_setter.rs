/// Tests for `CalloraVault::set_settlement` (issue #1111).
///
/// Covers:
/// - Rejection of `settlement == vault` (new error: `SettlementCannotBeVault`)
/// - Rejection of `settlement == usdc_token` (new error: `SettlementCannotBeToken`)
/// - Unauthorized caller is rejected
/// - Valid call succeeds, persists the address, and emits exactly one
///   `set_settlement` event carrying `(old, new)` in the data payload.
extern crate std;
use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(env: &'a Env, admin: &'a Address) -> (Address, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (addr.clone(), token::StellarAssetClient::new(env, &addr))
}

/// Returns (vault_addr, vault_client, usdc_addr, owner, initial_settlement).
fn setup(env: &Env) -> (Address, CalloraVaultClient, Address, Address, Address) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);
    let (usdc, _) = create_usdc(env, &owner);
    let initial_settlement = Address::generate(env);
    client.init(
        &owner,
        &usdc,
        &Some(0i128),
        &Some(owner.clone()),
        &Some(1i128),
        &None::<Address>,
        &Some(10_000_000_000i128),
        &Some(initial_settlement.clone()),
    );
    (vault_addr, client, usdc, owner, initial_settlement)
}

/// Helper: find all events whose first topic equals `topic_str`.
fn events_with_topic(
    env: &Env,
    topic_str: &str,
) -> std::vec::Vec<(soroban_sdk::Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val)> {
    let needle = Symbol::new(env, topic_str);
    env.events()
        .all()
        .iter()
        .filter(|(_, topics, _)| {
            if topics.is_empty() {
                return false;
            }
            let t0: Symbol = topics.get(0).unwrap().into_val(env);
            t0 == needle
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn set_settlement_vault_address_rejected() {
    let env = Env::default();
    let (vault_addr, client, _, owner, _) = setup(&env);
    let result = client.try_set_settlement(&owner, &vault_addr);
    assert_eq!(result, Err(Ok(VaultError::SettlementCannotBeVault)));
}

#[test]
fn set_settlement_usdc_token_rejected() {
    let env = Env::default();
    let (_, client, usdc, owner, _) = setup(&env);
    let result = client.try_set_settlement(&owner, &usdc);
    assert_eq!(result, Err(Ok(VaultError::SettlementCannotBeToken)));
}

#[test]
fn set_settlement_unauthorized_caller_rejected() {
    let env = Env::default();
    let (_, client, _, _, _) = setup(&env);
    let not_owner = Address::generate(&env);
    let new_settlement = Address::generate(&env);
    let result = client.try_set_settlement(&not_owner, &new_settlement);
    assert_eq!(result, Err(Ok(VaultError::Unauthorized)));
}

// ---------------------------------------------------------------------------
// Happy-path: persistence
// ---------------------------------------------------------------------------

#[test]
fn set_settlement_valid_address_persists() {
    let env = Env::default();
    let (_, client, _, owner, _) = setup(&env);
    let new_settlement = Address::generate(&env);
    client.set_settlement(&owner, &new_settlement);
    assert_eq!(client.get_settlement(), new_settlement);
}

// ---------------------------------------------------------------------------
// Happy-path: event emission
// ---------------------------------------------------------------------------

#[test]
fn set_settlement_emits_exactly_one_event() {
    let env = Env::default();
    let (_, client, _, owner, _) = setup(&env);
    // Drain init events.
    env.events().all();

    let new_settlement = Address::generate(&env);
    client.set_settlement(&owner, &new_settlement);

    let matched = events_with_topic(&env, "set_settlement");
    assert_eq!(
        matched.len(),
        1,
        "expected exactly one set_settlement event, got {}",
        matched.len()
    );
}

#[test]
fn set_settlement_event_carries_old_and_new_address() {
    let env = Env::default();
    let (_, client, _, owner, initial_settlement) = setup(&env);

    let new_settlement = Address::generate(&env);
    client.set_settlement(&owner, &new_settlement);

    let matched = events_with_topic(&env, "set_settlement");
    assert_eq!(matched.len(), 1, "set_settlement event not found");

    let (_, _, data) = matched.into_iter().next().unwrap();
    // data payload is (Option<Address>, Address)
    let (old_val, new_val): (Option<Address>, Address) = data.into_val(&env);
    assert_eq!(old_val, Some(initial_settlement), "old address mismatch");
    assert_eq!(new_val, new_settlement, "new address mismatch");
}

#[test]
fn set_settlement_event_topic_shape() {
    // Topics must be: (set_settlement, callora_v1, caller).
    let env = Env::default();
    let (_, client, _, owner, _) = setup(&env);

    let new_settlement = Address::generate(&env);
    client.set_settlement(&owner, &new_settlement);

    let matched = events_with_topic(&env, "set_settlement");
    assert_eq!(matched.len(), 1);
    let (_, topics, _) = matched.into_iter().next().unwrap();

    assert_eq!(topics.len(), 3, "expected 3 topics: (action, version, caller)");

    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    let t2: Address = topics.get(2).unwrap().into_val(&env);

    assert_eq!(t0, Symbol::new(&env, "set_settlement"));
    assert_eq!(t1, Symbol::new(&env, "callora_v1"));
    assert_eq!(t2, owner);
}

#[test]
fn set_settlement_first_call_old_value_is_some_of_initial() {
    // When a settlement was set during init, old value in the event should be
    // Some(initial_settlement), not None.
    let env = Env::default();
    let (_, client, _, owner, initial_settlement) = setup(&env);

    let new_settlement = Address::generate(&env);
    client.set_settlement(&owner, &new_settlement);

    let matched = events_with_topic(&env, "set_settlement");
    assert_eq!(matched.len(), 1);
    let (_, _, data) = matched.into_iter().next().unwrap();
    let (old_val, _): (Option<Address>, Address) = data.into_val(&env);
    assert_eq!(old_val, Some(initial_settlement));
}
