#![cfg(test)]

extern crate std;

use callora_fee::{FeeConfig, FeeContract, FeeContractClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

fn create_contract(env: &Env) -> FeeContractClient<'_> {
    let contract_id = env.register(FeeContract, ());
    FeeContractClient::new(env, &contract_id)
}

fn setup(env: &Env) -> (Address, FeeContractClient<'_>) {
    env.mock_all_auths();
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
fn set_fee_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_set_fee(&admin, &250);
    assert!(res.is_err(), "set_fee must require auth");
}

#[test]
fn deposit_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let caller = Address::generate(&env);
    let res = client.try_deposit(&caller, &100);
    assert!(res.is_err(), "deposit must require auth");
}

#[test]
fn withdraw_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    let caller = Address::generate(&env);
    client.deposit(&caller, &1000);

    env.set_auths(&[]);
    let recipient = Address::generate(&env);
    let res = client.try_withdraw(&admin, &recipient, &100);
    assert!(res.is_err(), "withdraw must require auth");
}

#[test]
fn get_fee_config_does_not_require_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let config = client.get_fee_config();
    assert_eq!(
        config,
        FeeConfig {
            fee_bps: 0,
            max_fee_bps: 10_000
        }
    );
}

#[test]
fn get_accumulated_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    let accumulated = client.get_accumulated();
    assert_eq!(accumulated, 0);
}

#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn admin_with_auth_can_call_all_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let client = create_contract(&env);
    client.init(&admin);

    client.set_fee(&admin, &250);
    assert_eq!(client.get_fee_config().fee_bps, 250);

    let caller = Address::generate(&env);
    client.deposit(&caller, &1000);
    assert_eq!(client.get_accumulated(), 1000);

    let recipient = Address::generate(&env);
    let _ = client.try_withdraw(&admin, &recipient, &400);
}
