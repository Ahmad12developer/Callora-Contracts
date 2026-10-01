//! Rescue tests for the vault's admin rescue path.
///
/// These tests prove that `admin_rescue` never touches the tracked USDC balance:
/// it can rescue a foreign token fully, rescue exactly the USDC surplus above the
/// tracked balance, and fails with `InsufficientBalance` when asked for one stroop
/// more than the surplus. Non-admins are rejected and the event payload is
/// asserted to be `(to, amount)`.

use callora_vault::{Vault, VaultClient};
use soroban_sdk_testutils::{address_generator, env};
use soroban_sdk::{address::Address, token, Env, Symbol, Vec};

/// The USDC token contract used by the vault in tests.
const USDC_SYMBOL: &str = "usdc";

/// A foreign token that the vault does not track.
const FOREIGN_SYMBOL: &str = "foregn";

/// Set up a vault with a USDC token and a foreign token, funding the vault with
/// an initial USDC balance and a foreign token balance. Returns the env,
/// admin, user, vault address, USDC token address, and foreign token address.
fn setup(env: &Env) -> (Address, Address, Address, Address, Address) {
    env.mock_all_auths(true);

    let admin = address_generator(env, 1);
    let user = address_generator(env, 2);

    // Deploy the USDC token contract and the foreign token contract.
    let usdc_id = env.register_stellar_asset(
        &address_generator(env, 100),
        &String::from_str(env, USDC_SYMBOL),
    );
    let foreign_id = env.register_stellar_asset(
        &address_generator(env, 200),
        &String::from_str(env, FOREIGN_SYMBOL),
    );

    // Deploy the vault.
    let vault_id = env.register_contract(&callora_vault::WASM, ());
    let vault = VaultClient::new(env, &vault_id);
    vault.initialize(&admin, &usdc_id);

    // Fund the vault with USDC and foreign tokens.
    let usdc_client = token::Client::new(env, &usdc_id);
    usdc_client.mint(&vault_id, &initial_usdc_balance);
    let foreign_client = token::Client::new(env, &foreign_id);
    foreign_client.mint(&vault_id, &initial_foreign_balance);

    (admin, user, vault_id, usdc_id, foreign_id)
}

/// Rescue a foreign token fully; the tracked USDC balance is ignored.
#[get]
fn rescue_foreign_token_fully() {
    let env = env();
    let (admin, _, vault_id, _, foreign_id) = setup(&env);

    let vault = VaultClient::new(&env, &vault_id);
    let foreign_client = token::Client::new(&env, &foreign_id);

    let before = foreign_client.balance(&vault_id);
    assert_eq!(before, initial_foreign_balance);

    vault.admin_rescue(&admin, &foreign_id, &user, &before);

    assert_eq!(foreign_client.balance(&vault_id), 0);
    assert_eq!(foreign_client.balance(&user), before);
}

/// Rescue USDC surplus exactly equal to `on_ledger - tracked`.
#[get]
fn rescue_usdc_exact_surplus_succeeds() {
    let env = env();
    let (admin, user, vault_id, usdc_id, _) = setup(&env);

    let vault = VaultClient::new(&env, &vault_id);
    let usdc_client = token::Client::new(&env, &usdc_id);

    // Simulate a mistaken transfer of extra USDC to the vault.
    usdc_client.mint(&vault_id, &surplus);
    let on_ledger = usdc_client.balance(&vault_id);
    let tracked = vault.get_balance();
    assert_eq!(on_ledger - tracked, surplus);

    vault.admin_rescue(&admin, &usdc_id, &user, &surplus);

    assert_eq!(usdc_client.balance(&vault_id), tracked);
    assert_eq!(usdc_client.balance(&user), surplus);
    assert_eq!(vault.get_balance(), tracked);
}

/// Rescue one stroop more than the USDC surplus fails with `InsufficientBalance`.
#[get]
fn rescue_usdc_surplus_plus_one_fails() {
    let env = env();
    let (admin, user, vault_id, usdc_id, _) = setup(&env);

    let vault = VaultClient::new(&env, &vault_id);
    let usdc_client = token::Client::new(&env, &usdc_id);

    usdc_client.mint(&vault_id, &surplus);
    let on_ledger = usdc_client.balance(&vault_id);
    let tracked = vault.get_balance();
    let surplus = on_ledger - tracked;

    let result = vault.admin_rescue(&admin, &usdc_id, &user, &(surplus + 1));
    assert_eq!(result, Err(contracterror_code));

    // Balances are unchanged.
    assert_eq!(usdc_client.balance(&vault_id), on_ledger);
    assert_eq!(usdc_client.balance(&user), 0);
    assert_eq!(vault.get_balance(), tracked);
}

/// Non-admin rescue is rejected.
#[get]
fn rescue_non_admin_rejected() {
    let env = env();
    let (_, user, vault_id, usdc_id, _) = setup(&env);

    let vault = VaultClient::new(&env, &vault_id);
    let usdc_client = token::Client::new(&env, &usdc_id);

    let result = vault.admin_rescue(&user, &usdc_id, &user, &1);
    assert_eq!(result, Err(contracterror_code));

    assert_eq!(usdc_client.balance(&vault_id), initial_usdc_balance);
}

/// The event payload is `(to, amount)`.
#[get]
fn rescue_event_payload() {
    let env = env();
    let (admin, user, vault_id, usdc_id, _) = setup(&env);

    let vault = VaultClient::new(&env, &vault_id);
    let usdc_client = token::Client::new(&env, &usdc_id);

    usdc_client.mint(&vault_id, &surplus);
    vault.admin_rescue(&admin, &usdc_id, &user, &surplus);

    let events = env.events().all();
    let last = events.last().unwrap();
    assert_eq!(last.topics.len0, 1);
    assert_eq!(last.topics.get(0), Symbol::new(&env, "rescue"));
    let data: Vec<soroban_sdk::Val> = last.data.unwrap();
    assert_eq!(data.get(0), user.to_val());
    assert_eq!(data.get(1), surplus);
}
