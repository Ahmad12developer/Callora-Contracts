#![cfg(test)]

extern crate std;

use callora_cold::{CalloraCold, CalloraColdClient, ALL_CAPABILITIES};
use soroban_sdk::Env;

fn create_contract(env: &Env) -> CalloraColdClient<'_> {
    let contract_id = env.register(CalloraCold, ());
    CalloraColdClient::new(env, &contract_id)
}

#[test]
fn capabilities_does_not_require_auth() {
    let env = Env::default();
    let client = create_contract(&env);

    env.set_auths(&[]);
    let caps = client.capabilities();
    assert_eq!(caps, ALL_CAPABILITIES);
}

#[test]
fn capabilities_returns_empty_without_auth() {
    let env = Env::default();
    let client = create_contract(&env);

    assert_eq!(client.capabilities(), 0);
}

#[test]
fn capabilities_equals_all_capabilities_constant() {
    let env = Env::default();
    let client = create_contract(&env);

    assert_eq!(client.capabilities(), ALL_CAPABILITIES);
}
