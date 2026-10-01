//! Tests for the `simulate_claim` read-only preview.
//!
//! Covers the issue's acceptance criteria:
//! - A frozen developer's simulation returns `DeveloperFrozen`, matching a
//!   real claim.
//! - A withdrawal that would breach the configured minimum balance is
//!   reported by the simulation as `MinBalanceViolation`, matching a real
//!   claim.
//! - The simulation extends no TTLs: neither persistent entries
//!   (claim window, minimum balance, developer balance) nor the contract
//!   instance.
//! - The success preview returns the expected `ClaimSimulation` fields and
//!   leaves tokens, balances, and daily counters untouched.

#![cfg(test)]

use crate::{
    CalloraSettlement, CalloraSettlementClient, SettlementError, StorageKey, INSTANCE_BUMP_AMOUNT,
    INSTANCE_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, Symbol};

/// Build a settlement contract with a registered USDC token.
/// Returns `(env, contract_id, admin, vault, usdc)`.
fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.init(&admin, &vault);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    client.set_usdc_token(&admin, &usdc);
    (env, contract_id, admin, vault, usdc)
}

/// Credit a developer's tracked balance and mint the matching tokens to the
/// settlement contract so simulated liquidity checks can pass.
fn credit(
    env: &Env,
    contract_id: &Address,
    admin: &Address,
    developer: &Address,
    usdc: &Address,
    amount: i128,
) {
    let client = CalloraSettlementClient::new(env, contract_id);
    client.force_credit_developer(admin, developer, &amount, usdc, &Symbol::new(env, "credit"));
    token::StellarAssetClient::new(env, usdc).mint(contract_id, &amount);
}

#[test]
fn simulate_claim_frozen_developer_returns_developer_frozen() {
    let (env, contract_id, admin, _vault, usdc) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);
    credit(&env, &contract_id, &admin, &developer, &usdc, 500);

    client.freeze_developer(&admin, &developer, &Symbol::new(&env, "audit"));

    assert_eq!(
        client.try_simulate_claim(&developer, &100i128, &None),
        Err(Ok(SettlementError::DeveloperFrozen))
    );
    // A real claim rejects the frozen developer with the same error.
    assert_eq!(
        client.try_withdraw_developer_balance(&developer, &100i128, &None),
        Err(Ok(SettlementError::DeveloperFrozen))
    );

    client.unfreeze_developer(&admin, &developer);
    assert!(client
        .try_simulate_claim(&developer, &100i128, &None)
        .is_ok());
}

#[test]
fn simulate_claim_min_balance_violation_reported() {
    let (env, contract_id, admin, _vault, usdc) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);
    credit(&env, &contract_id, &admin, &developer, &usdc, 500);
    client.set_developer_min_balance(&admin, &developer, &400i128);

    // 500 - 200 = 300 would fall below the 400 minimum.
    assert_eq!(
        client.try_simulate_claim(&developer, &200i128, &None),
        Err(Ok(SettlementError::MinBalanceViolation))
    );
    // The real claim path reports the identical typed error.
    assert_eq!(
        client.try_withdraw_developer_balance(&developer, &200i128, &None),
        Err(Ok(SettlementError::MinBalanceViolation))
    );

    // 500 - 100 = 400 lands exactly on the minimum and passes.
    let sim = client
        .try_simulate_claim(&developer, &100i128, &None)
        .unwrap()
        .unwrap();
    assert_eq!(sim.remaining_balance, 400);
}

#[test]
fn simulate_claim_does_not_extend_persistent_ttls() {
    let (env, contract_id, admin, _vault, usdc) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);
    credit(&env, &contract_id, &admin, &developer, &usdc, 500);
    client.set_developer_min_balance(&admin, &developer, &100i128);
    client.set_developer_claim_window(&admin, &developer, &0u64, &u64::MAX);

    // Drop persistent entry TTLs just below the bump threshold so that any
    // extension performed by the simulation becomes observable.
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let window_key = StorageKey::DeveloperClaimWindow(developer.clone());
    let min_key = StorageKey::DeveloperMinBalance(developer.clone());
    let balance_key = StorageKey::DeveloperBalance(developer.clone(), usdc.clone());

    let (window_ttl, min_ttl, balance_ttl, inst_ttl) = env.as_contract(&contract_id, || {
        (
            env.storage().persistent().get_ttl(&window_key),
            env.storage().persistent().get_ttl(&min_key),
            env.storage().persistent().get_ttl(&balance_key),
            env.storage().instance().get_ttl(),
        )
    });
    assert!(window_ttl < PERSISTENT_BUMP_THRESHOLD);
    assert!(min_ttl < PERSISTENT_BUMP_THRESHOLD);
    assert!(balance_ttl < PERSISTENT_BUMP_THRESHOLD);

    let sim = client
        .try_simulate_claim(&developer, &100i128, &None)
        .unwrap()
        .unwrap();
    assert_eq!(sim.remaining_balance, 400);

    let (window_ttl_after, min_ttl_after, balance_ttl_after, inst_ttl_after) =
        env.as_contract(&contract_id, || {
            (
                env.storage().persistent().get_ttl(&window_key),
                env.storage().persistent().get_ttl(&min_key),
                env.storage().persistent().get_ttl(&balance_key),
                env.storage().instance().get_ttl(),
            )
        });
    assert_eq!(window_ttl_after, window_ttl);
    assert_eq!(min_ttl_after, min_ttl);
    assert_eq!(balance_ttl_after, balance_ttl);
    assert_eq!(inst_ttl_after, inst_ttl);
}

#[test]
fn simulate_claim_does_not_extend_instance_ttl() {
    let (env, contract_id, _admin, _vault, _usdc) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);

    // Age the contract instance below its bump threshold. Persistent entries
    // seeded afterwards stay fresh; validation still runs the freeze, claim
    // window, minimum-balance, and balance reads before failing on the
    // empty developer balance.
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &StorageKey::DeveloperMinBalance(developer.clone()),
            &100i128,
        );
    });

    let inst_ttl = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
    assert!(inst_ttl < INSTANCE_BUMP_THRESHOLD);

    assert_eq!(
        client.try_simulate_claim(&developer, &100i128, &None),
        Err(Ok(SettlementError::MinBalanceViolation))
    );

    let inst_ttl_after = env.as_contract(&contract_id, || env.storage().instance().get_ttl());
    assert_eq!(inst_ttl_after, inst_ttl);
}

#[test]
fn simulate_claim_returns_preview_without_side_effects() {
    let (env, contract_id, admin, _vault, usdc) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);
    let recipient = Address::generate(&env);
    credit(&env, &contract_id, &admin, &developer, &usdc, 500);
    client.set_daily_withdraw_cap(&admin, &developer, &500i128);

    let sim = client
        .try_simulate_claim(&developer, &200i128, &Some(recipient.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(sim.developer, developer);
    assert_eq!(sim.amount, 200);
    assert_eq!(sim.recipient, recipient);
    assert_eq!(sim.token, usdc);
    assert_eq!(sim.current_balance, 500);
    assert_eq!(sim.remaining_balance, 300);
    assert_eq!(sim.contract_balance, 500);
    assert_eq!(sim.daily_withdraw_cap, 500);
    assert_eq!(sim.withdrawn_today, 0);
    assert_eq!(sim.withdrawn_today_after, 200);

    // No tokens moved and no tracked state was mutated by the preview.
    let token_client = token::Client::new(&env, &usdc);
    assert_eq!(token_client.balance(&contract_id), 500);
    assert_eq!(token_client.balance(&recipient), 0);
    assert_eq!(client.get_developer_balance(&developer, &usdc), 500);
    assert_eq!(client.get_withdrawal_today(&developer), 0);
}
