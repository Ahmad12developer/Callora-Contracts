#![cfg(test)]

extern crate std;

use callora_registry::{CalloraRegistry, CalloraRegistryClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

fn create_contract(env: &Env) -> CalloraRegistryClient<'_> {
    let contract_id = env.register(CalloraRegistry, ());
    CalloraRegistryClient::new(env, &contract_id)
}

fn setup(env: &Env) -> (Address, Address, CalloraRegistryClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let catalog = Address::generate(env);
    let client = create_contract(env);
    client.init(&admin, &catalog);
    (admin, catalog, client)
}

#[test]
fn init_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let catalog = Address::generate(&env);
    let client = create_contract(&env);

    env.set_auths(&[]);
    let res = client.try_init(&admin, &catalog);
    assert!(res.is_err(), "init must require auth");
}

#[test]
fn register_offering_requires_auth() {
    let env = Env::default();
    let (admin, _catalog, client) = setup(&env);

    env.set_auths(&[]);
    let developer = Address::generate(&env);
    let offering_id = String::from_str(&env, "offering-1");
    let metadata = String::from_str(&env, "test metadata");
    let res = client.try_register_offering(&admin, &developer, &offering_id, &metadata);
    assert!(res.is_err(), "register_offering must require auth");
}

#[test]
fn register_offering_with_gate_requires_auth() {
    let env = Env::default();
    let (admin, _catalog, client) = setup(&env);

    env.set_auths(&[]);
    let developer = Address::generate(&env);
    let token = Address::generate(&env);
    let offering_id = String::from_str(&env, "offering-2");
    let metadata = String::from_str(&env, "test metadata");
    let res = client.try_register_offering_with_gate(
        &admin,
        &developer,
        &token,
        &100_i128,
        &offering_id,
        &metadata,
    );
    assert!(
        res.is_err(),
        "register_offering_with_gate must require auth"
    );
}

#[test]
fn is_offering_registered_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _catalog, client) = setup(&env);

    let id = String::from_str(&env, "nonexistent");
    let registered = client.is_offering_registered(&id);
    assert_eq!(registered, false);
}

#[test]
fn registered_count_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _catalog, client) = setup(&env);

    let count = client.registered_count();
    assert_eq!(count, 0);
}
