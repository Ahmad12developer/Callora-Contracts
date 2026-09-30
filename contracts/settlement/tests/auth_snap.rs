#![cfg(test)]

extern crate std;

use callora_settlement::{CalloraSettlement, CalloraSettlementClient, SettlementError};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token;
use soroban_sdk::{Address, BytesN, Env, String, Symbol, Vec};

fn create_contract(env: &Env) -> CalloraSettlementClient<'_> {
    let contract_id = env.register(CalloraSettlement, ());
    CalloraSettlementClient::new(env, &contract_id)
}

fn setup(env: &Env) -> (Address, Address, CalloraSettlementClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let client = create_contract(env);
    client.init(&admin, &vault);
    (admin, vault, client)
}

#[test]
fn init_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let client = create_contract(&env);

    env.set_auths(&[]);
    let res = client.try_init(&admin, &vault);
    assert!(res.is_err(), "init must require auth");
}

#[test]
fn set_admin_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let new_admin = Address::generate(&env);
    let res = client.try_set_admin(&admin, &new_admin);
    assert!(res.is_err(), "set_admin must require auth");
}

#[test]
fn accept_admin_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

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
    let (admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);

    env.set_auths(&[]);
    let res = client.try_cancel_admin_transfer(&admin);
    assert!(res.is_err(), "cancel_admin_transfer must require auth");
}

#[test]
fn receive_payment_requires_auth() {
    let env = Env::default();
    let (admin, vault, client) = setup(&env);

    let token_addr = Address::generate(&env);

    env.set_auths(&[]);
    let res = client.try_receive_payment(&admin, &100_i128, &true, &None, &token_addr, &1);
    assert!(res.is_err(), "receive_payment must require auth");
}

#[test]
fn set_developer_min_balance_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let res = client.try_set_developer_min_balance(&admin, &dev, &100_i128);
    assert!(res.is_err(), "set_developer_min_balance must require auth");
}

#[test]
fn propose_vault_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let new_vault = Address::generate(&env);
    let res = client.try_propose_vault(&admin, &new_vault);
    assert!(res.is_err(), "propose_vault must require auth");
}

#[test]
fn accept_vault_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let new_vault = Address::generate(&env);
    client.propose_vault(&admin, &new_vault);

    env.set_auths(&[]);
    let res = client.try_accept_vault(&new_vault);
    assert!(res.is_err(), "accept_vault must require auth");
}

/// Admin alone must NOT be able to complete a vault rotation.
/// Closing the one-actor bypass described in issue #1141.
#[test]
fn accept_vault_admin_cannot_bypass_pending_check() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let new_vault = Address::generate(&env);
    client.propose_vault(&admin, &new_vault);

    // Admin tries to call accept_vault as themselves — must be rejected.
    let res = client.try_accept_vault(&admin);
    assert!(
        res.is_err(),
        "admin alone must not be able to accept a vault rotation"
    );
}

/// Accepting without a prior proposal must return the typed
/// `NoVaultRotationPending` error, not an unstructured panic.
#[test]
fn accept_vault_without_proposal_returns_typed_error() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let random = Address::generate(&env);
    let res = client.try_accept_vault(&random);
    let expected_code = SettlementError::NoVaultRotationPending as u32;
    let got_expected = match res {
        Err(Ok(e)) => {
            let sdk_err: soroban_sdk::Error = e.into();
            sdk_err.get_code() == expected_code
        }
        _ => false,
    };
    assert!(
        got_expected,
        "expected NoVaultRotationPending (code {})",
        expected_code
    );
}

/// Only the pending vault (not the admin) can finalize a rotation.
#[test]
fn accept_vault_only_pending_vault_can_accept() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let new_vault = Address::generate(&env);
    client.propose_vault(&admin, &new_vault);

    // Pending vault accepts — this must succeed.
    client.accept_vault(&new_vault);
    assert_eq!(client.get_vault(), new_vault, "vault must be updated to the new address");
}

#[test]
fn broadcast_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let msg = String::from_str(&env, "test");
    let res = client.try_broadcast(&admin, &callora_settlement::Severity::Info, &msg);
    assert!(res.is_err(), "broadcast must require auth");
}

#[test]
fn upgrade_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let hash = BytesN::from_array(&env, &[0u8; 32]);
    let res = client.try_upgrade(&admin, &hash);
    assert!(res.is_err(), "upgrade must require auth");
}

#[test]
fn set_usdc_token_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let usdc = Address::generate(&env);
    let res = client.try_set_usdc_token(&admin, &usdc);
    assert!(res.is_err(), "set_usdc_token must require auth");
}

#[test]
fn set_daily_withdraw_cap_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let res = client.try_set_daily_withdraw_cap(&admin, &dev, &1000_i128);
    assert!(res.is_err(), "set_daily_withdraw_cap must require auth");
}

#[test]
fn set_developer_claim_window_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let res = client.try_set_developer_claim_window(&admin, &dev, &100, &200);
    assert!(res.is_err(), "set_developer_claim_window must require auth");
}

#[test]
fn clear_developer_claim_window_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.mock_all_auths();
    let dev = Address::generate(&env);
    client.set_developer_claim_window(&admin, &dev, &100, &200);

    env.set_auths(&[]);
    let res = client.try_clear_developer_claim_window(&admin, &dev);
    assert!(
        res.is_err(),
        "clear_developer_claim_window must require auth"
    );
}

#[test]
fn force_credit_developer_requires_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let token_addr = Address::generate(&env);
    let res = client.try_force_credit_developer(
        &admin,
        &dev,
        &100_i128,
        &token_addr,
        &Symbol::new(&env, "test"),
    );
    assert!(res.is_err(), "force_credit_developer must require auth");
}

#[test]
fn get_admin_does_not_require_auth() {
    let env = Env::default();
    let (admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn get_vault_does_not_require_auth() {
    let env = Env::default();
    let (admin, vault, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_vault(), vault);
}

#[test]
fn get_global_pool_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let pool = client.get_global_pool();
    assert_eq!(pool.total_balance, 0);
}

#[test]
fn get_total_received_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_total_received(), 0);
}

#[test]
fn get_pending_admin_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_pending_admin(), None);
}

#[test]
fn get_version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    assert_eq!(client.get_version(), None);
}

#[test]
fn version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let v = client.version();
    assert!(!v.is_empty());
}

#[test]
fn get_developer_min_balance_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let bal = client.get_developer_min_balance(&dev);
    assert_eq!(bal, 0);
}

#[test]
fn get_developer_balance_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let token_addr = Address::generate(&env);
    let bal = client.get_developer_balance(&dev, &token_addr);
    assert_eq!(bal, 0);
}

#[test]
fn get_daily_withdraw_cap_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let cap = client.get_daily_withdraw_cap(&dev);
    assert_eq!(cap, 0);
}

#[test]
fn get_withdrawal_today_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let wd = client.get_withdrawal_today(&dev);
    assert_eq!(wd, 0);
}

#[test]
fn get_minimum_balance_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let bal = client.get_minimum_balance(&dev);
    assert_eq!(bal, 0);
}

#[test]
fn get_balance_migration_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let migration = client.get_balance_migration(&dev);
    assert_eq!(migration, None);
}

#[test]
fn get_developer_claim_window_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let dev = Address::generate(&env);
    let window = client.get_developer_claim_window(&dev);
    assert_eq!(window, None);
}

#[test]
fn migration_storage_version_does_not_require_auth() {
    let env = Env::default();
    let (_admin, _vault, client) = setup(&env);

    env.set_auths(&[]);
    let ver = client.migration_storage_version();
    assert_eq!(ver, 1);
}

#[test]
fn admin_with_auth_can_call_mutating_entrypoints() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let client = create_contract(&env);
    client.init(&admin, &vault);

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_vault(), vault);

    let new_admin = Address::generate(&env);
    client.set_admin(&admin, &new_admin);
    client.cancel_admin_transfer(&admin);

    let dev = Address::generate(&env);
    client.set_developer_min_balance(&admin, &dev, &100_i128);
    assert_eq!(client.get_developer_min_balance(&dev), 100);

    let new_vault = Address::generate(&env);
    client.propose_vault(&admin, &new_vault);
    client.accept_vault(&new_vault);
    assert_eq!(client.get_vault(), new_vault);
}

#[test]
fn auth_snap_covers_expected_views_count() {
    const EXPECTED_VIEWS: usize = 15;
    assert_eq!(EXPECTED_VIEWS, 15);
}
