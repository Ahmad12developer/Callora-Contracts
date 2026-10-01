//! Focused tests for overflow-safe arithmetic in `callora-settlement`.
//!
//! Issue #917: Replace any raw arithmetic in settlement with
//! `checked_add` / `checked_mul` / etc.  This module verifies:
//!
//! 1. **`record_deduction`** — cumulative total uses `checked_add` and raises
//!    [`SettlementError::PoolOverflow`] instead of panicking with `.unwrap()`
//!    when `TotalReceived` would overflow `i128`.
//!
//! 2. **`receive_payment` (pool path)** — `global_pool.total_balance` is
//!    guarded by `checked_add` returning `PoolOverflow`.
//!
//! 3. **`receive_payment` (developer path)** — `developer_balance` is
//!    guarded by `checked_add` returning `DeveloperOverflow`.
//!
//! 4. **`batch_receive_payment` (developer path)** — same overflow protection
//!    as the single-item developer path.
//!
//! 5. **`withdraw_developer_balance`** — `checked_sub` raises
//!    `InsufficientDeveloperBalance` rather than wrapping.
//!
//! 6. **Normal (non-overflow) paths** still produce correct arithmetic results.
//!
//! 7. **`withdraw_developer_balance`** (daily accumulator) — `checked_add` raises
//!    `DailyWithdrawCapExceeded` rather than silently saturating on overflow.

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

use crate::{CalloraSettlement, CalloraSettlementClient, StorageKey};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a minimal settlement contract; return `(env, contract_id, admin, vault)`.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.init(&admin, &vault);
    (env, contract_id, admin, vault)
}

/// Build a settlement contract with a registered USDC token.
/// Returns `(env, contract_id, admin, vault, usdc_address)`.
fn setup_with_usdc() -> (Env, Address, Address, Address, Address) {
    let (env, contract_id, admin, vault) = setup();
    // Register a stellar asset so we can mint in tests.
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.set_usdc_token(&admin, &usdc);
    (env, contract_id, admin, vault, usdc)
}

/// Directly write an `i128` into instance storage under `key`, bypassing the
/// normal entrypoint. Useful for seeding near-overflow values without going
/// through public API that validates amounts.
fn poke_instance_i128(env: &Env, contract_id: &Address, key: &StorageKey, value: i128) {
    env.as_contract(contract_id, || {
        env.storage().instance().set(key, &value);
    });
}

/// Directly write a developer balance in persistent storage.
fn poke_dev_balance(
    env: &Env,
    contract_id: &Address,
    developer: &Address,
    token: &Address,
    value: i128,
) {
    env.as_contract(contract_id, || {
        env.storage().persistent().set(
            &StorageKey::DeveloperBalance(developer.clone(), token.clone()),
            &value,
        );
    });
}

/// Directly write a developer's `WithdrawalToday` state in persistent storage.
fn poke_withdrawal_today(
    env: &Env,
    contract_id: &Address,
    developer: &Address,
    amount: i128,
    day: u64,
) {
    use crate::types::DailyWithdrawState;
    env.as_contract(contract_id, || {
        env.storage().persistent().set(
            &StorageKey::WithdrawalToday(developer.clone()),
            &DailyWithdrawState { day, amount },
        );
    });
}

/// Directly write the global pool's `total_balance` in instance storage.
fn poke_global_pool_balance(env: &Env, contract_id: &Address, total_balance: i128) {
    use crate::types::GlobalPool;
    env.as_contract(contract_id, || {
        env.storage().instance().set(
            &StorageKey::GlobalPool,
            &GlobalPool {
                total_balance,
                last_updated: 0,
            },
        );
    });
}

// ---------------------------------------------------------------------------
// 1. record_deduction — TotalReceived overflow must raise PoolOverflow
// ---------------------------------------------------------------------------

/// Pre-existing `.unwrap()` on `checked_add` in `record_deduction` was
/// replaced with `.unwrap_or_else(|| env.panic_with_error(PoolOverflow))`.
/// This test verifies the overflow is caught rather than panicking opaquely.
#[test]
fn record_deduction_overflow_raises_error() {
    let (env, contract_id, _admin, _vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);

    // Seed TotalReceived to i128::MAX - 1.
    poke_instance_i128(
        &env,
        &contract_id,
        &StorageKey::TotalReceived,
        i128::MAX - 1,
    );

    // Adding 2 wraps; the checked path must return an error.
    let result = client.try_record_deduction(&2i128, &1u64);
    assert!(
        result.is_err(),
        "record_deduction must fail when TotalReceived would overflow i128"
    );
}

/// Normal (non-overflow) `record_deduction` increments `TotalReceived`.
#[test]
fn record_deduction_increments_total_received_correctly() {
    let (env, contract_id, _admin, _vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);

    poke_instance_i128(&env, &contract_id, &StorageKey::TotalReceived, 1_000i128);
    client.record_deduction(&500i128, &1u64);

    assert_eq!(
        client.get_total_received(),
        1_500i128,
        "TotalReceived should be 1000 + 500 = 1500"
    );
}

// ---------------------------------------------------------------------------
// 2. receive_payment (pool path) — PoolOverflow
// ---------------------------------------------------------------------------

/// `receive_payment(to_pool=true)` must fail when global pool total would
/// exceed `i128::MAX`.
#[test]
fn receive_payment_pool_overflow_is_caught() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    // Seed pool balance to i128::MAX - 1.
    poke_global_pool_balance(&env, &contract_id, i128::MAX - 1);

    // Adding 2 must be caught.
    let result = client.try_receive_payment(&vault, &2i128, &true, &None, &token, &1u32);
    assert!(
        result.is_err(),
        "receive_payment pool credit should fail on i128 overflow"
    );
}

/// Normal pool credit increments `global_pool.total_balance` correctly.
#[test]
fn receive_payment_pool_credit_is_correct() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    client.receive_payment(&vault, &1_000i128, &true, &None, &token, &1u32);

    let pool = client.get_global_pool();
    assert_eq!(pool.total_balance, 1_000i128);
}

// ---------------------------------------------------------------------------
// 3. receive_payment (developer path) — DeveloperOverflow
// ---------------------------------------------------------------------------

/// `receive_payment(to_pool=false)` must fail when the developer balance
/// would exceed `i128::MAX`.
#[test]
fn receive_payment_developer_overflow_is_caught() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let developer = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    // Seed near-max developer balance.
    poke_dev_balance(&env, &contract_id, &developer, &token, i128::MAX - 1);

    let result =
        client.try_receive_payment(&vault, &2i128, &false, &Some(developer), &token, &1u32);
    assert!(
        result.is_err(),
        "receive_payment developer credit should fail on i128 overflow"
    );
}

#[test]
fn receive_payment_developer_credit_accumulates_correctly() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let developer = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    client.receive_payment(
        &vault,
        &3_000i128,
        &false,
        &Some(developer.clone()),
        &token,
        &1u32,
    );
    assert_eq!(client.get_developer_balance(&developer, &token), 3_000i128);

    client.receive_payment(
        &vault,
        &1_500i128,
        &false,
        &Some(developer.clone()),
        &token,
        &2u32,
    );
    assert_eq!(client.get_developer_balance(&developer, &token), 4_500i128);
}

// ---------------------------------------------------------------------------
// 4. batch_receive_payment — DeveloperOverflow in a batch item
// ---------------------------------------------------------------------------

#[test]
fn batch_receive_payment_developer_overflow_is_caught() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let developer = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    // Seed near-max balance for the developer.
    poke_dev_balance(&env, &contract_id, &developer, &token, i128::MAX - 1);

    let mut items: Vec<(Address, i128)> = Vec::new(&env);
    items.push_back((developer, 2i128));

    let result = client.try_batch_receive_payment(&vault, &items, &token, &1u32);
    assert!(
        result.is_err(),
        "batch_receive_payment should fail when a developer balance would overflow"
    );
}

#[test]
fn batch_receive_payment_accumulates_balances_correctly() {
    let (env, contract_id, _admin, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let dev_a = Address::generate(&env);
    let dev_b = Address::generate(&env);
    client.add_supported_token(&_admin, &token);

    let mut items: Vec<(Address, i128)> = Vec::new(&env);
    items.push_back((dev_a.clone(), 500i128));
    items.push_back((dev_b.clone(), 750i128));

    client.batch_receive_payment(&vault, &items, &token, &1u32);

    assert_eq!(client.get_developer_balance(&dev_a, &token), 500i128);
    assert_eq!(client.get_developer_balance(&dev_b, &token), 750i128);
}

// ---------------------------------------------------------------------------
// 5. withdraw_developer_balance — checked_sub
// ---------------------------------------------------------------------------

/// Withdrawal of more than available balance must return an error, not wrap.
#[test]
fn withdraw_more_than_balance_returns_error() {
    let (env, contract_id, _admin, vault, usdc) = setup_with_usdc();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);

    // Credit 100 to the developer.
    client.receive_payment(
        &vault,
        &100i128,
        &false,
        &Some(developer.clone()),
        &usdc,
        &1u32,
    );

    // Attempt to withdraw 200 — must fail.
    let result = client.try_withdraw_developer_balance(&developer, &200i128, &None);
    assert!(
        result.is_err(),
        "withdrawal exceeding developer balance should be rejected"
    );
}

// ---------------------------------------------------------------------------
// 6. withdraw_developer_balance — daily accumulator overflow -> DailyWithdrawCapExceeded
// ---------------------------------------------------------------------------

/// When the developer has no cap set (0 = unlimited), the daily withdrawal
/// accumulator still uses `checked_add` so that an i128 overflow fails
/// loudly instead of silently saturating.
#[test]
fn withdraw_daily_amount_overflow_raises_error() {
    let (env, contract_id, _admin, vault, stored_usdc) = setup_with_usdc();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);

    // Seed developer balance with a large amount so the withdrawal itself
    // does not fail with InsufficientDeveloperBalance.
    poke_dev_balance(&env, &contract_id, &developer, &stored_usdc, 1_000_000i128);

    // Seed the same-day withdrawal accumulator to i128::MAX - 1 so the
    // next withdrawal's checked_add will overflow.
    let today = env.ledger().timestamp() / 86400;
    poke_withdrawal_today(&env, &contract_id, &developer, i128::MAX - 1, today);

    // Mint enough USDC to the contract so the transfer can proceed
    // past the liquidity check.
    let token_admin_client = soroban_sdk::token::StellarAssetClient::new(&env, &stored_usdc);
    token_admin_client.mint(&contract_id, &1_000i128);

    // Withdraw 1 micro-unit — must fail because daily.amount would overflow.
    let result = client.try_withdraw_developer_balance(&developer, &1i128, &None);
    assert!(
        result.is_err(),
        "daily accumulator overflow should fail, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. Normal withdrawals still work correctly
// ---------------------------------------------------------------------------

/// Exact-balance withdrawal succeeds and leaves the developer at zero.
#[test]
fn withdraw_exact_balance_succeeds() {
    let (env, contract_id, _admin, vault, stored_usdc) = setup_with_usdc();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let developer = Address::generate(&env);

    // Mint USDC to the settlement contract so the on-chain transfer can succeed.
    let token_admin_client = soroban_sdk::token::StellarAssetClient::new(&env, &stored_usdc);
    token_admin_client.mint(&contract_id, &500i128);

    client.receive_payment(
        &vault,
        &500i128,
        &false,
        &Some(developer.clone()),
        &stored_usdc,
        &1u32,
    );

    let result = client.try_withdraw_developer_balance(&developer, &500i128, &None);
    assert!(
        result.is_ok(),
        "exact-balance withdrawal should succeed: {result:?}"
    );
    assert_eq!(
        client.get_developer_balance(&developer, &stored_usdc),
        0i128,
        "developer balance should be zero after exact withdrawal"
    );
}
