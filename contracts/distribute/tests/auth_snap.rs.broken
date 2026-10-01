#![cfg(test)]

extern crate std;

use callora_distribute::{
    CalloraDistribute, CalloraDistributeClient, DistributeError, BatchItem,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, Symbol, Vec};

fn create_contract(env: &Env) -> CalloraDistributeClient<'_> {
    let contract_id = env.register(CalloraDistribute, ());
    CalloraDistributeClient::new(env, &contract_id)
}

fn setup(env: &Env) -> (Address, CalloraDistributeClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let client = create_contract(env);
    client.init(&admin, &10);
    (admin, client)
}

#[test]
fn init_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let client = create_contract(&env);

    env.set_auths(&[]);
    let res = client.try_init(&admin, &10);
    assert!(res.is_err(), "init must require auth");
}

#[test]
fn set_global_cap_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_set_global_cap(&admin, &20);
    assert!(res.is_err(), "set_global_cap must require auth");
}

#[test]
fn open_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let account = Address::generate(&env);
    let res = client.try_open(&admin, &account, &Symbol::new(&env, "test"));
    assert!(res.is_err(), "open must require auth");
}

#[test]
fn close_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    let account = Address::generate(&env);
    let cat = Symbol::new(&env, "test");
    client.open(&admin, &account, &cat);

    env.set_auths(&[]);
    let res = client.try_close(&admin, &account, &cat);
    assert!(res.is_err(), "close must require auth");
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
fn pause_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let res = client.try_pause(&admin);
    assert!(res.is_err(), "pause must require auth");
}

#[test]
fn unpause_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.mock_all_auths();
    client.pause(&admin);

    env.set_auths(&[]);
    let res = client.try_unpause(&admin);
    assert!(res.is_err(), "unpause must require auth");
}

#[test]
fn upgrade_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let dummy = BytesN::from_array(&env, &[0u8; 32]);
    let res = client.try_upgrade(&admin, &dummy);
    assert!(res.is_err(), "upgrade must require auth");
}

#[test]
fn broadcast_requires_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    let msg = soroban_sdk::String::from_str(&env, "test");
    let res = client.try_broadcast(
        &admin,
        &callora_distribute::Severity::Info,
        &msg,
    );
    assert!(res.is_err(), "broadcast must require auth");
}

#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin().unwrap(), admin);
}

#[test]
fn get_global_cap_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_global_cap(), 10);
}

#[test]
fn is_paused_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert!(!client.is_paused());
}

#[test]
fn get_pending_admin_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_pending_admin(), None);
}

#[test]
fn get_version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_version(), None);
}

#[test]
fn admin_with_auth_can_call_all_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let client = create_contract(&env);
    client.init(&admin, &10);

    assert_eq!(client.get_global_cap(), 10);

    client.set_global_cap(&admin, &20);
    assert_eq!(client.get_global_cap(), 20);

    let account = Address::generate(&env);
    let cat = Symbol::new(&env, "test");
    client.open(&admin, &account, &cat);
    assert_eq!(client.get_account_count(&account), 1);

    client.close(&admin, &account, &cat);
    assert_eq!(client.get_account_count(&account), 0);

    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.accept_admin();
    assert_eq!(client.get_admin().unwrap(), new_admin);
}
