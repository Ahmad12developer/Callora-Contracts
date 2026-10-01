#![cfg(test)]

extern crate std;

use callora_whitelist::{CalloraWhitelist, CalloraWhitelistClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env};

fn create_contract(env: &Env) -> CalloraWhitelistClient<'_> {
    let contract_id = env.register(CalloraWhitelist, ());
    CalloraWhitelistClient::new(env, &contract_id)
}

fn setup(env: &Env) -> (Address, CalloraWhitelistClient<'_>) {
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);
    let admin = Address::generate(env);
    let client = create_contract(env);
    client.init(&admin);
    (admin, client)
}

#[test]
fn init_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let client = create_contract(&env);

    env.set_auths(&[]);
    let res = client.try_init(&admin);
    assert!(res.is_err(), "init must require auth");
}

#[test]
fn set_admin_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let new_admin = Address::generate(&env);
    let res = client.try_set_admin(&admin, &new_admin);
    assert!(res.is_err(), "set_admin must require auth");
}

#[test]
fn accept_admin_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_accept_admin();
    assert!(res.is_err(), "accept_admin must require auth");
}

#[test]
fn cancel_admin_transfer_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_cancel_admin_transfer(&admin);
    assert!(res.is_err(), "cancel_admin_transfer must require auth");
}

#[test]
fn add_address_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    client.set_admin_cooldown(&admin, &1);

    env.set_auths(&[]);
    let addr = Address::generate(&env);
    let res = client.try_add_address(&admin, &addr);
    assert!(res.is_err(), "add_address must require auth");
}

#[test]
fn remove_address_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    client.set_admin_cooldown(&admin, &1);
    let addr = Address::generate(&env);
    client.add_address(&admin, &addr);
    env.ledger().set_timestamp(1_000_001);

    env.set_auths(&[]);
    let res = client.try_remove_address(&admin, &addr);
    assert!(res.is_err(), "remove_address must require auth");
}

#[test]
fn clear_all_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    client.set_admin_cooldown(&admin, &1);

    env.set_auths(&[]);
    let res = client.try_clear_all(&admin);
    assert!(res.is_err(), "clear_all must require auth");
}

#[test]
fn set_admin_cooldown_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_set_admin_cooldown(&admin, &300);
    assert!(res.is_err(), "set_admin_cooldown must require auth");
}

#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn is_whitelisted_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    let addr = Address::generate(&env);
    assert!(!client.is_whitelisted(&addr));
}

#[test]
fn get_whitelist_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert!(client.get_whitelist().is_empty());
}

#[test]
fn get_admin_cooldown_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin_cooldown(), 3600);
}

#[test]
fn admin_cooldown_remaining_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.admin_cooldown_remaining(), 0);
}

#[test]
fn is_admin_action_ready_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert!(client.is_admin_action_ready());
}

#[test]
fn get_last_critical_admin_action_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert!(client.get_last_critical_admin_action().is_none());
}

#[test]
fn admin_with_auth_can_call_all_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let client = create_contract(&env);
    client.init(&admin);

    client.set_admin_cooldown(&admin, &1);

    let addr = Address::generate(&env);
    client.add_address(&admin, &addr);
    assert!(client.is_whitelisted(&addr));

    env.ledger().set_timestamp(1_000_001);
    client.remove_address(&admin, &addr);
    assert!(!client.is_whitelisted(&addr));

    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.cancel_admin_transfer(&admin);
    client.set_admin(&admin, &new_admin);
    client.accept_admin();
    assert_eq!(client.get_admin(), new_admin);
}
