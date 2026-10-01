use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

use super::*;

fn setup(env: &Env) -> (Address, Address, CalloraVaultClient<'_>) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let usdc = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let vault = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault);
    client.init(
        &owner,
        &usdc,
        &Some(0i128),
        &None,
        &Some(1i128),
        &None,
        &Some(1i128),
        &None,
    );
    env.events().all();
    (owner, vault, client)
}

fn assert_versioned_event(env: &Env, vault: &Address, name: &str) {
    let events = env.events().all();
    let (_, topics, _) = events
        .iter()
        .rev()
        .find(|(address, _, _)| address == vault)
        .expect("vault event");
    let topic: Symbol = topics.get(0).unwrap().into_val(env);
    let version: Symbol = topics.get(1).unwrap().into_val(env);
    assert_eq!(topic, Symbol::new(env, name));
    assert_eq!(version, Symbol::new(env, "callora_v1"));
}

#[test]
fn cancel_admin_transfer_removes_nominee_and_emits_versioned_event() {
    let env = Env::default();
    let (owner, vault, client) = setup(&env);
    let nominee = Address::generate(&env);

    client.set_admin(&owner, &nominee);
    client.cancel_admin_transfer(&owner);

    assert_versioned_event(&env, &vault, "admin_cancelled");
    assert_eq!(
        client.try_accept_admin(),
        Err(Ok(VaultError::NoAdminTransferPending))
    );
}

#[test]
fn cancel_ownership_transfer_removes_nominee_and_emits_versioned_event() {
    let env = Env::default();
    let (owner, vault, client) = setup(&env);
    let nominee = Address::generate(&env);

    client.transfer_ownership(&owner, &nominee);
    client.cancel_ownership_transfer(&owner);

    assert_versioned_event(&env, &vault, "ownership_cancelled");
    assert_eq!(
        client.try_accept_ownership(),
        Err(Ok(VaultError::NoOwnershipTransferPending))
    );
}

#[test]
fn cancelling_without_pending_transfer_returns_typed_errors() {
    let env = Env::default();
    let (owner, _, client) = setup(&env);

    assert_eq!(
        client.try_cancel_admin_transfer(&owner),
        Err(Ok(VaultError::NoAdminTransferPending))
    );
    assert_eq!(
        client.try_cancel_ownership_transfer(&owner),
        Err(Ok(VaultError::NoOwnershipTransferPending))
    );
    assert_eq!(
        client.try_accept_ownership(),
        Err(Ok(VaultError::NoOwnershipTransferPending))
    );
}

#[test]
fn cancellation_requires_the_matching_role() {
    let env = Env::default();
    let (owner, _, client) = setup(&env);
    let attacker = Address::generate(&env);
    let nominee = Address::generate(&env);

    client.set_admin(&owner, &nominee);
    assert_eq!(
        client.try_cancel_admin_transfer(&attacker),
        Err(Ok(VaultError::Unauthorized))
    );

    client.transfer_ownership(&owner, &nominee);
    assert_eq!(
        client.try_cancel_ownership_transfer(&attacker),
        Err(Ok(VaultError::Unauthorized))
    );
}
