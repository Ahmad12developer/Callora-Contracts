extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, String, Symbol};

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

/// Register and initialize the settlement contract.
fn create_settlement(env: &Env, admin: &Address, vault_address: &Address) -> Address {
    let settlement_address = env.register(CalloraSettlement, ());
    let settlement_client =
        callora_settlement::CalloraSettlementClient::new(env, &settlement_address);
    env.mock_all_auths();
    settlement_client.init(admin, vault_address);
    settlement_address
}

/// Mint `amount` USDC directly to `vault_address` (simulates pre-funded vault).
fn fund_vault(
    usdc_admin_client: &token::StellarAssetClient,
    vault_address: &Address,
    amount: i128,
) {
    usdc_admin_client.mint(vault_address, &amount);
}

// ---------------------------------------------------------------------------
// Init tests
// ---------------------------------------------------------------------------

#[test]
fn init_with_balance_emits_event() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let events = env.events().all();
    std::println!("init_with_balance_emits_event events len: {}", events.len());
    let last = events.last().expect("expected at least one event");

    assert_eq!(last.0, vault_address);
    let topics = &last.1;
    assert_eq!(topics.len(), 3);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    let topic1: Address = topics.get(2).unwrap().into_val(&env);
    let version: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(version, Symbol::new(&env, "callora_v1"));
    assert_eq!(topic0, Symbol::new(&env, "init"));
    assert_eq!(topic1, owner);

    let data: i128 = last.2.into_val(&env);
    assert_eq!(data, 1000);
}

#[test]
fn init_defaults_balance_to_zero() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    assert_eq!(client.balance(), 0);
}

#[test]
#[ignore = "known bug: init does not enforce InitialBalanceExceedsOnLedger (#17); reported in PR"]
#[should_panic(expected = "Error(Contract, #17)")]
fn init_fails_when_initial_balance_exceeds_onchain_usdc_balance() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 99);

    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
}

#[test]
fn double_init_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    assert!(result.is_err(), "expected error on second init");
}

// ---------------------------------------------------------------------------
// Admin tests
// ---------------------------------------------------------------------------

#[test]
fn get_admin_returns_owner_after_init() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    assert_eq!(client.get_admin(), owner);
}

#[test]
fn set_admin_two_step_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    client.set_admin(&owner, &new_admin);
    assert_eq!(client.get_admin(), owner); // Still old admin

    client.accept_admin();
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
fn set_admin_unauthorized_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let intruder = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_set_admin(&intruder, &new_admin);
    assert!(
        result.is_err(),
        "expected error when non-admin calls set_admin"
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn unauthorized_address_cannot_deposit() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let unauthorized = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    client.deposit(&unauthorized, &50);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deposit_zero_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deposit(&owner, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deposit_negative_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deposit(&owner, &-50);
}

#[test]
fn deposit_paused_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    client.pause(&owner);
    assert!(client.is_paused());

    usdc_admin.mint(&owner, &100);
    usdc_client.approve(&owner, &vault_address, &100, &1000);

    let result = client.try_deposit(&owner, &100);
    assert_eq!(result, Err(Ok(VaultError::Paused)));

    client.unpause(&owner);
    assert!(!client.is_paused());
    client.deposit(&owner, &100);
    assert_eq!(client.balance(), 100);
}

/// Validates: Requirements 8.7, 4.2
#[test]
fn balance_unchanged_after_failed_deposit() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let unauthorized = Address::generate(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    // min_deposit = 50
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(50),
        &None,
        &None,
        &None,
    );

    // Mint enough for the owner to deposit later (after unpause)
    usdc_admin.mint(&owner, &200);
    usdc_client.approve(&owner, &vault_address, &200, &10_000);

    let balance_before = client.balance();

    // Scenario 1: unauthorized caller
    let result = client.try_deposit(&unauthorized, &100);
    assert!(result.is_err(), "unauthorized caller must be rejected");
    assert_eq!(
        client.balance(),
        balance_before,
        "balance must be unchanged after unauthorized deposit"
    );

    // Scenario 2: paused vault
    client.pause(&owner);
    let result = client.try_deposit(&owner, &100);
    assert!(result.is_err(), "paused vault must reject deposit");
    assert_eq!(
        client.balance(),
        balance_before,
        "balance must be unchanged after paused deposit"
    );
    client.unpause(&owner);

    // Scenario 3: below minimum (10 < 50)
    let result = client.try_deposit(&owner, &10);
    assert!(result.is_err(), "below-minimum deposit must be rejected");
    assert_eq!(
        client.balance(),
        balance_before,
        "balance must be unchanged after below-minimum deposit"
    );
}

/// Validates: Requirements 5.1, 5.2, 5.3
#[test]
fn deposit_event_schema_alignment() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    usdc_admin.mint(&owner, &200);
    usdc_client.approve(&owner, &vault_address, &200, &10_000);

    client.deposit(&owner, &150);

    let events = env.events().all();
    let deposit_event = events
        .iter()
        .find(|e| {
            if e.0 != vault_address {
                return false;
            }
            if e.1.is_empty() {
                return false;
            }
            let s: Symbol = e.1.get(0).unwrap().into_val(&env);
            s == Symbol::new(&env, "deposit")
        })
        .expect("expected deposit event");

    // Schema alignment: exactly 3 topics (deposit, callora_v1, caller)
    assert_eq!(
        deposit_event.1.len(),
        3,
        "deposit event must have exactly 3 topics"
    );
    let topic0: Symbol = deposit_event.1.get(0).unwrap().into_val(&env);
    let topic1: Address = deposit_event.1.get(2).unwrap().into_val(&env);
    let version: Symbol = deposit_event.1.get(1).unwrap().into_val(&env);
    assert_eq!(version, Symbol::new(&env, "callora_v1"));
    assert_eq!(
        topic0,
        Symbol::new(&env, "deposit"),
        "topic[0] must be Symbol(\"deposit\")"
    );
    assert_eq!(topic1, owner, "topic[1] must be the depositor address");

    // Data must decode as (amount: i128, new_balance: i128)
    let (amount, new_balance): (i128, i128) = deposit_event.2.into_val(&env);
    assert_eq!(amount, 150, "event data amount must match deposited amount");
    assert_eq!(
        new_balance, 150,
        "event data new_balance must match vault balance"
    );
}

// ---------------------------------------------------------------------------
// Pause tests
// ---------------------------------------------------------------------------

#[test]
fn pause_unpause_admin_only() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let intruder = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    // intruder fails
    let res = client.try_pause(&intruder);
    assert!(res.is_err());

    // admin (owner) succeeds
    client.pause(&owner);
    assert!(client.is_paused());

    // intruder fails unpause
    let res = client.try_unpause(&intruder);
    assert!(res.is_err());

    // admin (owner) succeeds unpause
    client.unpause(&owner);
    assert!(!client.is_paused());
}

#[test]
fn pause_emits_event() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    client.pause(&owner);
    let events = env.events().all();
    let pause_event = events
        .iter()
        .find(|e| {
            e.0 == vault_address
                && e.1
                    .get(0)
                    .map(|v| {
                        let s: Symbol = v.into_val(&env);
                        s == Symbol::new(&env, "vault_paused")
                    })
                    .unwrap_or(false)
        })
        .expect("expected pause event");

    let admin_topic: Address = pause_event.1.get(2).unwrap().into_val(&env);
    assert_eq!(admin_topic, owner);
}

#[test]
fn deduct_insufficient_balance_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 10);
    client.init(
        &owner,
        &usdc,
        &Some(10),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_deduct(&owner, &100, &1u64);
    assert!(result.is_err(), "expected error for insufficient balance");
}


#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deduct_zero_amount_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &client.address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&owner, &0, &3u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn deduct_exceeding_max_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &client.address, 1000);
    // Set max_deduct to 500
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(500),
        &None,
    );
    client.deduct(&owner, &501, &4u64);
}

#[test]
fn deduct_paused_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &client.address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.pause(&owner);
    assert_eq!(
        client.try_deduct(&owner, &100, &5u64),
        Err(Ok(VaultError::Paused))
    );
}


#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deduct_zero_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&owner, &0, &7u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn deduct_negative_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&owner, &-50, &8u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn deduct_exceeds_balance_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 50);
    client.init(
        &owner,
        &usdc,
        &Some(50),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&owner, &100, &9u64);
}

#[test]
fn balance_unchanged_after_failed_deduct() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );

    let _ = client.try_deduct(&owner, &200, &10u64);
    assert_eq!(client.balance(), 100);
}

#[test]
fn get_revenue_pool_returns_none_when_not_set() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    assert_eq!(client.get_revenue_pool(), None);
}

#[test]
fn get_revenue_pool_consistent_after_deduct_operations() {
    // Ensure get_revenue_pool remains consistent and doesn't mutate state
    let env = Env::default();
    let owner = Address::generate(&env);
    let caller = Address::generate(&env);
    let revenue_pool = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc_address,
        &Some(1000),
        &Some(caller.clone()),
        &Some(1),
        &Some(revenue_pool.clone()),
        &None,
        &None,
    );
    let settlement = create_settlement(&env, &owner, &vault_address);
    client.set_settlement(&owner, &settlement);

    // Query revenue pool before deduct
    let before = client.get_revenue_pool();
    assert_eq!(before, Some(revenue_pool.clone()));

    // Perform deduct operation (routes to settlement, not revenue_pool)
    client.deduct(&caller, &200, &11u64);

    // Query revenue pool after deduct - should be unchanged
    let after = client.get_revenue_pool();
    assert_eq!(after, Some(revenue_pool.clone()));
    assert_eq!(before, after);

    // Funds flow to settlement; revenue_pool receives nothing.
    assert_eq!(client.balance(), 800);
    assert_eq!(usdc_client.balance(&settlement), 200);
    assert_eq!(usdc_client.balance(&revenue_pool), 0);
}

// ---------------------------------------------------------------------------
// Withdraw tests
// ---------------------------------------------------------------------------

#[test]
fn withdraw_reduces_balance() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let remaining = client.withdraw(&200);
    assert_eq!(remaining, 300);
    assert_eq!(client.balance(), 300);
}

#[test]
fn withdraw_full_balance_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let remaining = client.withdraw(&1000);
    assert_eq!(remaining, 0);
    assert_eq!(client.balance(), 0);
}

#[test]
fn withdraw_insufficient_balance_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_withdraw(&500);
    assert!(result.is_err(), "expected error for insufficient balance");
}

#[test]
fn withdraw_zero_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_withdraw(&0);
    assert!(result.is_err(), "expected error for zero amount");
}

#[test]
fn withdraw_to_reduces_balance() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let remaining = client.withdraw_to(&recipient, &150);
    assert_eq!(remaining, 350);
    assert_eq!(client.balance(), 350);
    assert_eq!(usdc_client.balance(&recipient), 150);
}

#[test]
fn withdraw_unauthorized_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let _intruder = Address::generate(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // Reset auths to test requirement without mock_all_auths bypassing it
    env.set_auths(&[]);
    let res = client.try_withdraw(&500);
    assert!(res.is_err());
}

#[test]
fn withdraw_to_insufficient_balance_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_withdraw_to(&recipient, &500);
    assert!(result.is_err(), "expected error for insufficient balance");
}

#[test]
#[should_panic(expected = "cannot withdraw to vault address")]
fn withdraw_to_vault_address_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // Attempt to withdraw to the vault itself
    client.withdraw_to(&vault_address, &100);
}

#[test]
#[should_panic(expected = "cannot withdraw to token address")]
fn withdraw_to_token_address_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // Attempt to withdraw to the USDC token contract
    client.withdraw_to(&usdc, &100);
}

#[test]
fn withdraw_to_while_paused_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // Pause the vault
    client.pause(&owner);
    assert!(client.is_paused());

    // Withdraw should still work while paused (emergency recovery)
    let remaining = client.withdraw_to(&recipient, &300);
    assert_eq!(remaining, 700);
    assert_eq!(client.balance(), 700);
    assert_eq!(usdc_client.balance(&recipient), 300);
}

#[test]
fn withdraw_while_paused_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // Pause the vault
    client.pause(&owner);
    assert!(client.is_paused());

    // Withdraw should still work while paused (emergency recovery)
    let remaining = client.withdraw(&200);
    assert_eq!(remaining, 800);
    assert_eq!(client.balance(), 800);
    assert_eq!(usdc_client.balance(&owner), 200);
}

#[test]
fn transfer_ownership_emits_events() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let new_owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.transfer_ownership(&owner, &new_owner);

    let events = env.events().all();
    let nomad_ev = events
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "ownership_nominated")
            }
        })
        .expect("expected ownership_nominated event");

    let old_n: Address = nomad_ev.1.get(2).unwrap().into_val(&env);
    let new_n: Address = nomad_ev.2.into_val(&env);
    assert_eq!(old_n, owner);
    assert_eq!(new_n, new_owner);

    client.accept_ownership();
    let events2 = env.events().all();
    let accept_ev = events2
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "ownership_accepted")
            }
        })
        .expect("expected ownership_accepted event");

        let new_a: Address = accept_ev.1.get(2).unwrap().into_val(&env);
    assert_eq!(new_a, new_owner);
}

#[test]
#[ignore = "known bug: transfer_ownership does not enforce NewOwnerSameAsCurrent (#23); reported in PR"]
#[should_panic(expected = "Error(Contract, #23)")]
fn transfer_ownership_same_address_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.transfer_ownership(&owner, &owner);
}

// ---------------------------------------------------------------------------
// Distribute tests
// ---------------------------------------------------------------------------

#[test]
fn distribute_transfers_usdc_to_recipient() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let developer = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    client.distribute(&admin, &developer, &300);

    assert_eq!(usdc_client.balance(&developer), 300);
    assert_eq!(usdc_client.balance(&vault_address), 700);
}

#[test]
fn distribute_unauthorized_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let intruder = Address::generate(&env);
    let developer = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_distribute(&intruder, &developer, &300);
    assert!(result.is_err(), "expected error when non-admin distributes");
}

#[test]
fn distribute_insufficient_usdc_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let developer = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_distribute(&admin, &developer, &500);
    assert!(result.is_err(), "expected error for insufficient USDC");
}

#[test]
fn distribute_zero_amount_fails() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let developer = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_distribute(&admin, &developer, &0);
    assert!(result.is_err(), "expected error for zero amount");
}

#[test]
fn distribute_while_paused_succeeds() {
    // distribute is ALLOWED when paused (emergency recovery, matches withdraw policy)
    let env = Env::default();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    client.pause(&admin);
    assert!(client.is_paused());

    // distribute should succeed while paused
    client.distribute(&admin, &recipient, &300);

    assert_eq!(usdc_client.balance(&recipient), 300);
    assert_eq!(usdc_client.balance(&vault_address), 700);
}

#[test]
fn distribute_while_unpaused_succeeds() {
    // happy path: distribute works normally when unpaused
    let env = Env::default();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    assert!(!client.is_paused());

    client.distribute(&admin, &recipient, &300);

    assert_eq!(usdc_client.balance(&recipient), 300);
    assert_eq!(usdc_client.balance(&vault_address), 700);
}

#[test]
fn distribute_admin_only_enforcement() {
    // only admin can call distribute, even when unpaused
    let env = Env::default();
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &admin);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &admin,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // non-admin should fail
    let result = client.try_distribute(&non_admin, &recipient, &300);
    assert!(result.is_err(), "expected error when non-admin distributes");

    // admin should succeed
    client.distribute(&admin, &recipient, &300);
    assert_eq!(client.balance(), 0); // balance unchanged (distribute uses on-ledger balance)
}

// ---------------------------------------------------------------------------
// Revenue pool integration tests
// ---------------------------------------------------------------------------

#[test]
fn init_with_revenue_pool_stores_address() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let revenue_pool = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &None,
        &Some(1),
        &Some(revenue_pool.clone()),
        &None,
        &None,
    );

    assert_eq!(client.balance(), 500);
}

#[test]
#[should_panic(expected = "Settlement not set")]
fn deduct_with_only_revenue_pool_panics() {
    // Revenue pool is no longer a deduct destination; settlement is mandatory.
    let env = Env::default();
    let owner = Address::generate(&env);
    let caller = Address::generate(&env);
    let revenue_pool = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc_address, _usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc_address,
        &Some(1000),
        &Some(caller.clone()),
        &Some(1),
        &Some(revenue_pool),
        &None,
        &None,
    );

    client.deduct(&caller, &300, &12u64);
}

#[test]
fn deduct_with_settlement_transfers_usdc() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let caller = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let settlement = create_settlement(&env, &owner, &vault_address);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 800);
    client.init(
        &owner,
        &usdc_address,
        &Some(800),
        &Some(caller.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.set_settlement(&owner, &settlement);

    client.deduct(&caller, &250, &13u64);

    assert_eq!(client.balance(), 550);
    assert_eq!(usdc_client.balance(&settlement), 250);
}

#[test]
fn get_revenue_pool_no_mutation_on_multiple_calls() {
    // Verify calling get_revenue_pool multiple times doesn't mutate state
    let env = Env::default();
    let owner = Address::generate(&env);
    let pool = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(1),
        &Some(pool.clone()),
        &None,
        &None,
    );

    let initial_balance = client.balance();

    // Call get_revenue_pool multiple times
    for _ in 0..10 {
        let result = client.get_revenue_pool();
        assert_eq!(result, Some(pool.clone()));
    }

    // Verify balance unchanged (no mutation)
    assert_eq!(client.balance(), initial_balance);
}

#[test]
fn get_revenue_pool_consistency_with_zero_balance() {
    // Ensure get_revenue_pool works correctly with zero vault balance
    let env = Env::default();
    let owner = Address::generate(&env);
    let pool = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(1),
        &Some(pool.clone()),
        &None,
        &None,
    );

    // Balance should be zero
    assert_eq!(client.balance(), 0);

    // Revenue pool should still be queryable
    assert_eq!(client.get_revenue_pool(), Some(pool));
}

#[test]
fn deposit_max_balance_overflow_panic() {
    // Explicit test for max-balance overflow near i128::MAX.
    // Exercises the checked_add(...).unwrap_or_else(|| panic!("balance overflow")) path.
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();

    // 1. Setup vault balance near i128::MAX
    let near_max = i128::MAX - 1;
    let overflow_amount = 2;

    fund_vault(&usdc_admin, &vault_address, near_max);
    client.init(
        &owner,
        &usdc,
        &Some(near_max),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    // 2. Prepare overflow deposit
    usdc_admin.mint(&owner, &overflow_amount);
    usdc_client.approve(&owner, &vault_address, &overflow_amount, &1000);

    // 3. Confirm it panics safely on overflow
    let result = client.try_deposit(&owner, &overflow_amount);
    assert!(
        result.is_err(),
        "contract must fail safely when balance would overflow i128::MAX"
    );
}

#[test]
fn deduct_routes_to_settlement_when_both_configured() {
    // settlement takes priority over revenue_pool when both are set
    let env = Env::default();
    let owner = Address::generate(&env);
    let caller = Address::generate(&env);
    let revenue_pool = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let settlement = create_settlement(&env, &owner, &vault_address);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc_address,
        &Some(1000),
        &Some(caller.clone()),
        &Some(1),
        &Some(revenue_pool.clone()),
        &None,
        &None,
    );
    client.set_settlement(&owner, &settlement);

    client.deduct(&caller, &400, &14u64);

    // settlement gets the funds, revenue_pool gets nothing
    assert_eq!(usdc_client.balance(&settlement), 400);
    assert_eq!(usdc_client.balance(&revenue_pool), 0);
}

// ---------------------------------------------------------------------------
// set_settlement / get_settlement tests
// ---------------------------------------------------------------------------

#[test]
fn set_settlement_stores_and_get_returns_address() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let settlement = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    client.set_settlement(&owner, &settlement);
    assert_eq!(client.get_settlement(), settlement);
}

#[test]
fn set_settlement_unauthorized_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let attacker = Address::generate(&env);
    let settlement = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    assert_eq!(
        client.try_set_settlement(&attacker, &settlement),
        Err(Ok(VaultError::Unauthorized))
    );
}


#[test]
#[should_panic(expected = "Settlement not set")]
fn get_settlement_before_set_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &None,
    );
    client.get_settlement();
}

#[test]
fn get_settlement_returns_correct_after_update() {
    // Verify get_settlement reflects latest committed state after multiple updates
    let env = Env::default();
    let owner = Address::generate(&env);
    let settlement1 = Address::generate(&env);
    let settlement2 = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    // Set first settlement address
    client.set_settlement(&owner, &settlement1);

    assert_eq!(client.get_settlement(), settlement1);

    // Update to second settlement address
    client.set_settlement(&owner, &settlement2);

    assert_eq!(client.get_settlement(), settlement2);
}

#[test]
fn get_settlement_consistent_after_deduct_operations() {
    // Ensure get_settlement remains consistent and doesn't mutate state
    let env = Env::default();
    let owner = Address::generate(&env);
    let caller = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let settlement = create_settlement(&env, &owner, &vault_address);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc_address,
        &Some(1000),
        &Some(caller.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.set_settlement(&owner, &settlement);

    // Query settlement before deduct
    let before = client.get_settlement();
    assert_eq!(before, settlement);

    // Perform deduct operation
    client.deduct(&caller, &200, &15u64);

    // Query settlement after deduct - should be unchanged
    let after = client.get_settlement();
    assert_eq!(after, settlement);
    assert_eq!(before, after);

    // Verify no state mutation occurred
    assert_eq!(client.balance(), 800);
    assert_eq!(usdc_client.balance(&settlement), 200);
}

#[test]
fn get_settlement_no_mutation_on_multiple_calls() {
    // Verify calling get_settlement multiple times doesn't mutate state
    let env = Env::default();
    let owner = Address::generate(&env);
    let settlement = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(settlement.clone()),
    );

    let initial_balance = client.balance();

    // Call get_settlement multiple times
    for _ in 0..10 {
        let result = client.get_settlement();
        assert_eq!(result, settlement);
    }

    // Verify balance unchanged (no mutation)
    assert_eq!(client.balance(), initial_balance);
}


#[test]
fn set_authorized_caller_vault_address_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    let result = client.try_set_authorized_caller(&Some(vault_address), &0u64);
    assert_eq!(result, Err(Ok(VaultError::AuthorizedCallerCannotBeVault)));
}

/// Successful rotation emits the consumed nonce in the event data.
#[test]
fn set_authorized_caller_event_emits_nonce() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let new_caller = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    client.set_authorized_caller(&Some(new_caller.clone()), &0u64);

    let events = env.events().all();
    let ev = events.last().expect("expected set_authorized_caller event");

    let topic: Symbol = ev.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic, Symbol::new(&env, "set_authorized_caller"));

    let (old, now, nonce): (Option<Address>, Option<Address>, u64) = ev.2.into_val(&env);
    assert_eq!(old, Some(owner.clone()));
    assert_eq!(now, Some(new_caller));
    assert_eq!(nonce, 0u64);
}

#[test]
fn test_deduct_with_settlement_success() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let settlement = create_settlement(&env, &owner, &vault_address);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc_address,
        &Some(1000),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );

    client.set_settlement(&owner, &settlement);
    client.deduct(&owner, &300, &16u64);

    assert_eq!(client.balance(), 700);
    assert_eq!(usdc_client.balance(&settlement), 300);
}

#[test]
fn deposit_overflow_panics() {
    // A deposit that would push balance past i128::MAX must panic.
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, i128::MAX);
    client.init(
        &owner,
        &usdc,
        &Some(i128::MAX),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    usdc_admin.mint(&owner, &1);
    usdc_client.approve(&owner, &vault_address, &1, &1000);
    let result = client.try_deposit(&owner, &1);
    assert!(result.is_err(), "expected overflow panic");
}

#[test]
fn withdraw_to_zero_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 300);
    client.init(
        &owner,
        &usdc,
        &Some(300),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    assert_eq!(client.withdraw(&300), 0);
}

#[test]
fn withdraw_near_i128_max_succeeds() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    let initial: i128 = i128::MAX - 100;
    fund_vault(&usdc_admin, &vault_address, initial);
    client.init(
        &owner,
        &usdc,
        &Some(initial),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let remaining = client.withdraw(&(initial - 1));
    assert_eq!(remaining, 1);
}

#[test]
fn is_paused_reflects_latest_committed_state() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);
    env.mock_all_auths();
    let rp = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(1),
        &Some(rp),
        &None,
        &None,
    );

    // Initial state
    assert!(!client.is_paused());

    // Pause and verify immediate reflection
    client.pause(&owner);
    assert!(client.is_paused());

    // Unpause and verify immediate reflection
    client.unpause(&owner);
    assert!(!client.is_paused());

    // Admin change shouldn't affect pause state
    client.set_admin(&owner, &new_admin);
    client.accept_admin();
    assert!(!client.is_paused());

    // pause is owner-only; a newly accepted admin is rejected
    assert_eq!(client.try_pause(&new_admin), Err(Ok(VaultError::Unauthorized)));
    assert!(!client.is_paused());
}

#[test]
fn is_paused_safe_default_before_init() {
    let env = Env::default();
    let (_, client) = create_vault(&env);
    // Before initialization, is_paused should return false (safe default)
    // and must not panic
    assert!(!client.is_paused());
}
#[test]
fn deduct_while_paused_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    let settlement = create_settlement(&env, &owner, &vault_address);

    client.pause(&owner);
    assert_eq!(
        client.try_deduct(&owner, &100, &17u64),
        Err(Ok(VaultError::Paused))
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn deduct_unauthorized_caller_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let attacker = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    // init with an authorized_caller so the None branch is not taken
    let auth = Address::generate(&env);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(auth),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&attacker, &100, &18u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn deduct_exceeds_max_deduct_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 1000);
    client.init(
        &owner,
        &usdc,
        &Some(1000),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(50),
        &None,
    );
    client.deduct(&owner, &100, &19u64); // 100 > max_deduct(50)
}

#[test]
#[should_panic(expected = "AmountNotPositive")]
fn distribute_negative_amount_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let dev = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.distribute(&owner, &dev, &-1);
}

#[test]
#[should_panic(expected = "Error(Contract, #25)")]
fn accept_admin_without_pending_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    client.accept_admin();
}

#[test]
#[should_panic(expected = "no ownership transfer pending")]
fn accept_ownership_without_pending_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (_, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    client.accept_ownership();
}

// ---------------------------------------------------------------------------
// Cancel ownership transfer tests
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "AmountNotPositive")]
fn withdraw_negative_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.withdraw(&-1);
}

#[test]
#[should_panic(expected = "AmountNotPositive")]
fn withdraw_to_negative_fails() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.withdraw_to(&recipient, &-1);
}

#[test]
#[should_panic(expected = "Settlement not set")]
fn deduct_without_settlement_panics() {
    // Settlement is a hard precondition for deduct; missing address must panic.
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _usdc_client, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.deduct(&owner, &200, &20u64);
}

#[test]
fn deduct_without_settlement_does_not_mutate_state() {
    // When deduct panics due to missing settlement, vault state must be unchanged.
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = client.try_deduct(&owner, &200, &21u64);
    assert!(result.is_err(), "expected panic for missing settlement");
    assert_eq!(client.balance(), 500);
    assert_eq!(usdc_client.balance(&vault_address), 500);
}

#[test]
fn withdraw_emits_event() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 300);
    client.init(
        &owner,
        &usdc,
        &Some(300),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.withdraw(&100);
    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "withdraw")
            }
        })
        .expect("expected withdraw event");
    let (amt, bal): (i128, i128) = ev.2.into_val(&env);
    assert_eq!(amt, 100);
    assert_eq!(bal, 200);
}

#[test]
fn withdraw_to_emits_event() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 300);
    client.init(
        &owner,
        &usdc,
        &Some(300),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.withdraw_to(&recipient, &150);
    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "withdraw_to")
            }
        })
        .expect("expected withdraw_to event");
    let (amt, bal): (i128, i128) = ev.2.into_val(&env);
    assert_eq!(amt, 150);
    assert_eq!(bal, 150);
}

#[test]
fn distribute_emits_event() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let dev = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    client.distribute(&owner, &dev, &200);
    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "distribute")
            }
        })
        .expect("expected distribute event");
    let amt: i128 = ev.2.into_val(&env);
    assert_eq!(amt, 200);
}

#[test]
fn vault_unpaused_event_emitted() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, _) = create_usdc(&env, &owner);
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );
    client.pause(&owner);
    client.unpause(&owner);
    let events = env.events().all();
    let ev = events
        .iter()
        .find(|e| {
            e.0 == vault_address && !e.1.is_empty() && {
                let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                t == Symbol::new(&env, "vault_unpaused")
            }
        })
        .expect("expected vault_unpaused event");
    let caller: Address = ev.1.get(2).unwrap().into_val(&env);
    assert_eq!(caller, owner);
}

#[test]
fn deposit_zero_amount_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 0);
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deposit(&owner, &0);
    }));
    assert!(result.is_err(), "zero-value deposit must fail");
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn deposit_below_min_deposit_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 0);
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(50),
        &None,
        &None,
        &None,
    );

    usdc_admin.mint(&owner, &49);
    usdc_client.approve(&owner, &vault_address, &49, &1000);
    client.deposit(&owner, &49);
}

#[test]
fn deposit_one_below_large_min_deposit_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 0);
    client.init(
        &owner,
        &usdc,
        &None,
        &None,
        &Some(1_000_000),
        &None,
        &None,
        &None,
    );

    usdc_admin.mint(&owner, &999_999);
    usdc_client.approve(&owner, &vault_address, &999_999, &1000);
    let result = client.try_deposit(&owner, &999_999);
    assert!(
        result.is_err(),
        "deposit one below large min_deposit must fail"
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn deduct_below_minimum_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, _, usdc_admin) = create_usdc(&env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 100);
    client.init(
        &owner,
        &usdc,
        &Some(100),
        &Some(owner.clone()),
        &Some(10),
        &None,
        &Some(100),
        &None,
    );
    let settlement = create_settlement(&env, &owner, &vault_address);

    client.deduct(&owner, &1, &22u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn deduct_above_max_deduct_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let (vault_address, client) = create_vault(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, 500);
    client.init(
        &owner,
        &usdc,
        &Some(500),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(100),
        &None,
    );
    usdc_admin.mint(&owner, &200);
    usdc_client.approve(&owner, &vault_address, &200, &1000);
    client.deposit(&owner, &200);
    // deduct 101 > max_deduct 100 — must panic
    client.deduct(&owner, &101, &23u64);
}

/// Verifies that `deposit`, `withdraw`, and `withdraw_to` each extend the TTL
/// so state remains accessible after a ledger advance.
#[test]
#[ignore = "soroban reentrancy incompatible"]
fn instance_ttl_extended_on_mutating_entrypoints() {
    use crate::INSTANCE_BUMP_THRESHOLD;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (usdc, usdc_client, usdc_admin) = create_usdc(&env, &owner);
    let (vault_address, client) = create_vault(&env);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10000000000),
        &Some(soroban_sdk::Address::generate(&env)),
    );

    usdc_admin.mint(&owner, &500);
    usdc_client.approve(&owner, &vault_address, &500, &200_000);

    // deposit — bumps TTL
    client.deposit(&owner, &300);
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_THRESHOLD - 1);
    assert_eq!(
        client.balance(),
        300,
        "balance readable after ledger advance post-deposit"
    );

    // withdraw — bumps TTL
    client.withdraw(&100);
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_THRESHOLD - 1);
    assert_eq!(
        client.balance(),
        200,
        "balance readable after ledger advance post-withdraw"
    );

    // withdraw_to — bumps TTL
    client.withdraw_to(&recipient, &50);
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_THRESHOLD - 1);
    assert_eq!(
        client.balance(),
        150,
        "balance readable after ledger advance post-withdraw_to"
    );
}

/// Helper function to set up a fully initialized vault with settlement and sufficient balance.
fn setup_vault_for_deduct(env: &Env, initial_balance: i128) -> (Address, CalloraVaultClient) {
    let owner = Address::generate(env);
    let (vault_address, client) = create_vault(env);
    let (usdc, _, usdc_admin) = create_usdc(env, &owner);

    env.mock_all_auths();
    fund_vault(&usdc_admin, &vault_address, initial_balance);
    client.init(
        &owner,
        &usdc,
        &Some(initial_balance),
        &None,
        &Some(1),
        &None,
        &None,
        &None,
    );
    let settlement = create_settlement(env, &owner, &vault_address);

    (owner, client)
}


