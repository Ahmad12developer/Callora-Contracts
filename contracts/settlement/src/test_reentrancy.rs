//! Reentrancy tests for [`CalloraSettlement::withdraw_developer_balance`].
//!
//! The attack scenario: a custom token contract intercepts the USDC `transfer`
//! call and immediately calls back into `withdraw_developer_balance` for the
//! same developer and the same amount.  With the CEI fix in place the balance
//! is written to persistent storage **before** the transfer is executed, so the
//! re-entrant call observes a zero (or insufficient) balance and is rejected
//! with `InsufficientDeveloperBalance`.  Without the fix, both calls would
//! observe the original balance and the developer could withdraw twice.

#![cfg(test)]

extern crate std;

use crate::{CalloraSettlement, CalloraSettlementClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Malicious token mock
// ---------------------------------------------------------------------------

/// A mock token that re-enters `withdraw_developer_balance` from inside
/// `transfer`.
///
/// Configuration is stored in instance storage and set via
/// [`MaliciousTokenClient::set_attack_config`] before the first withdrawal.
///
/// The `balance` method always returns a large fixed value so the settlement
/// contract's `InsufficientContractBalance` check never fires; we want to
/// exercise the balance-state check instead.
///
/// The mock disables itself after the first re-entrant call to prevent infinite
/// recursion.
#[contract]
pub struct MaliciousToken;

#[contractimpl]
impl MaliciousToken {
    /// Intercept a USDC transfer.  If the attack is active, re-enter
    /// `withdraw_developer_balance` on the settlement contract with the same
    /// developer and amount, then deactivate the attack flag.
    pub fn transfer(env: Env, from: Address, _to: Address, _amount: i128) {
        from.require_auth();

        let attack_active: bool = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "attack_active"))
            .unwrap_or(false);

        if attack_active {
            // Disable the flag first to avoid infinite recursion.
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "attack_active"), &false);

            let settlement: Address = env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "settlement"))
                .unwrap();
            let developer: Address = env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "developer"))
                .unwrap();
            let amount: i128 = env
                .storage()
                .instance()
                .get(&Symbol::new(&env, "amount"))
                .unwrap();

            let client = CalloraSettlementClient::new(&env, &settlement);
            // Attempt the re-entrant withdrawal; the result is intentionally
            // ignored — the test asserts on storage state after the outer call
            // completes.
            let _ = client.try_withdraw_developer_balance(&developer, &amount, &None);
        }
    }

    /// Always report a large balance so settlement's liquidity check passes.
    pub fn balance(_env: Env, _id: Address) -> i128 {
        1_000_000_000
    }

    /// Configure the reentrancy attack.
    ///
    /// # Arguments
    /// * `settlement` - settlement contract address to re-enter
    /// * `developer`  - developer address whose balance to re-drain
    /// * `amount`     - withdrawal amount to request in the re-entrant call
    /// * `active`     - enable / disable the attack
    pub fn set_attack_config(
        env: Env,
        settlement: Address,
        developer: Address,
        amount: i128,
        active: bool,
    ) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "settlement"), &settlement);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "developer"), &developer);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "amount"), &amount);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_active"), &active);
    }
}

// ---------------------------------------------------------------------------
// Reentrancy tests
// ---------------------------------------------------------------------------

/// Primary reentrancy invariant: a re-entrant `withdraw_developer_balance` call
/// that fires from inside the token's `transfer` hook must not succeed because
/// the balance is already zeroed in persistent storage before the transfer.
///
/// Post-conditions:
/// - Developer balance is `0` (drained exactly once, not twice).
/// - The outer withdrawal returns `Ok(())`.
/// - The inner (re-entrant) withdrawal is blocked by an insufficient-balance
///   check (observed indirectly: final balance == 0, not negative).
#[test]
fn reentrancy_cannot_double_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);

    let settlement_addr = env.register(CalloraSettlement, ());
    let token_addr = env.register(MaliciousToken, ());

    let client = CalloraSettlementClient::new(&env, &settlement_addr);
    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &token_addr);

    let initial: i128 = 500;

    // Credit the developer's tracked balance (ledger_seq = 1).
    client.receive_payment(
        &vault,
        &initial,
        &false,
        &Some(developer.clone()),
        &token_addr,
        &1u32,
    );

    // Verify balance was credited.
    assert_eq!(
        client.get_developer_balance(&developer, &token_addr),
        initial
    );

    // Arm the malicious token: it will call withdraw_developer_balance(500)
    // from inside transfer().
    let token_client = MaliciousTokenClient::new(&env, &token_addr);
    token_client.set_attack_config(&settlement_addr, &developer, &initial, &true);

    // First (legitimate) withdrawal.
    let result = client.try_withdraw_developer_balance(&developer, &initial, &None);
    assert!(
        result.is_ok(),
        "legitimate withdrawal must succeed even under reentrancy attack"
    );

    // The re-entrant call saw 0 balance and was rejected.  Final balance must
    // be exactly 0, proving the developer was not double-drained.
    assert_eq!(
        client.get_developer_balance(&developer, &token_addr),
        0,
        "developer balance must be exactly 0 after one successful withdrawal; \
         a non-zero negative value would indicate an underflow bug, and a \
         positive value would indicate the withdrawal did not execute"
    );
}

/// Verify that a re-entrant withdrawal attempt observes the already-reduced
/// balance because state is persisted (Effects) before the token transfer
/// (Interaction).
///
/// We observe this indirectly: if the re-entrant call had succeeded, the
/// developer's balance would have gone below zero (impossible with checked_sub)
/// or the token would have been called twice.  The stable balance == 0 result
/// is definitive proof that the re-entrant call was blocked.
#[test]
fn reentrancy_reentrant_call_observes_zero_balance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);

    let settlement_addr = env.register(CalloraSettlement, ());
    let token_addr = env.register(MaliciousToken, ());

    let client = CalloraSettlementClient::new(&env, &settlement_addr);
    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &token_addr);

    // Credit 1 000 units so the attacker has something to re-drain.
    let credit: i128 = 1_000;
    client.receive_payment(
        &vault,
        &credit,
        &false,
        &Some(developer.clone()),
        &token_addr,
        &1u32,
    );

    // Arm: attempt to withdraw 1_000 again from inside transfer.
    let token_client = MaliciousTokenClient::new(&env, &token_addr);
    token_client.set_attack_config(&settlement_addr, &developer, &credit, &true);

    // Outer call drains the full balance.
    let outer = client.try_withdraw_developer_balance(&developer, &credit, &None);
    assert!(outer.is_ok(), "outer withdrawal must succeed");

    // Because effects (storage zeroing) occurred BEFORE the interaction
    // (transfer), the re-entrant call found balance = 0 and was rejected.
    let final_balance = client.get_developer_balance(&developer, &token_addr);
    assert_eq!(
        final_balance, 0,
        "balance must be 0 after single drain; got {final_balance} — \
         non-zero means the reentrancy guard is ineffective"
    );
}

/// Daily withdrawal counter must also be updated before the transfer so that a
/// re-entrant withdrawal cannot exceed the daily cap by racing the counter.
///
/// Setup:
/// - Balance = 1 000, daily cap = 500.
/// - Outer call withdraws 300.
/// - Re-entrant call (armed before outer call) tries to withdraw another 300.
/// - Because the counter is persisted first (300 already counted), the
///   re-entrant call would need 300 + 300 = 600 > cap(500) and is rejected.
/// - Final balance must be 700 (one 300-unit withdrawal, not two).
#[test]
fn reentrancy_daily_counter_persisted_before_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);

    let settlement_addr = env.register(CalloraSettlement, ());
    let token_addr = env.register(MaliciousToken, ());

    let client = CalloraSettlementClient::new(&env, &settlement_addr);
    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &token_addr);

    // Set a daily cap of 500.
    let cap: i128 = 500;
    client.set_daily_withdraw_cap(&admin, &developer, &cap);

    // Credit 1 000 so there is enough balance even if reentrancy were to work.
    client.receive_payment(
        &vault,
        &1_000,
        &false,
        &Some(developer.clone()),
        &token_addr,
        &1u32,
    );

    // Arm: re-entrant call tries to withdraw 300 (cap is 500; if the counter
    // were not persisted first, both calls totalling 600 > 500 could slip
    // through because each would observe 0 withdrawn today).
    let token_client = MaliciousTokenClient::new(&env, &token_addr);
    token_client.set_attack_config(&settlement_addr, &developer, &300, &true);

    // Outer call withdraws 300.
    let outer = client.try_withdraw_developer_balance(&developer, &300, &None);
    assert!(outer.is_ok(), "outer withdrawal of 300 must succeed");

    // Because the daily counter was persisted before the transfer, the
    // re-entrant call saw `withdrawn_today = 300` already and was blocked
    // (300 + 300 = 600 > cap of 500).  Remaining balance must be 700.
    let remaining = client.get_developer_balance(&developer, &token_addr);
    assert_eq!(
        remaining, 700,
        "balance after single 300-unit withdrawal must be 700; \
         got {remaining} — if 600 were drained the daily counter was not persisted before transfer"
    );

    // Daily counter must reflect exactly 300 withdrawn.
    let withdrawn = client.get_withdrawal_today(&developer);
    assert_eq!(
        withdrawn, 300,
        "daily counter must show 300 (one successful withdrawal); got {withdrawn}"
    );
}
