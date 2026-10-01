//! Kani proof harnesses for `CalloraVault::deduct` balance conservation.
//!
//! The vault's successful `deduct` path moves `amount` out of the tracked vault
//! balance and credits that same `amount` to settlement. These harnesses model
//! that state transition with the same arithmetic preconditions enforced by the
//! contract and prove that the combined accounting total is unchanged.
//!
//! # Current deduct signature
//!
//! ```rust,ignore
//! pub fn deduct(env: Env, caller: Address, amount: i128, request_id: u64) { … }
//! pub fn batch_deduct(env: Env, caller: Address, items: Vec<(i128, u64)>) { … }
//! ```
//!
//! `request_id: u64` is an idempotency token that is forwarded intact to
//! `settlement_client.record_deduction`.  It is **not** part of the balance
//! arithmetic.  The model therefore accepts `request_id` as a parameter but
//! does not use it in any computation, and the proof
//! [`kani_deduct_conserves_total_supply`] verifies that the `request_id` range
//! has no effect on the conservation property.
//!
//! Run with:
//! ```bash
//! cargo kani --package callora-vault --harness kani_deduct_conserves_total_supply
//! cargo kani --package callora-vault --harness kani_deduct_request_id_conserves_total_supply
//! cargo kani --package callora-vault --harness kani_batch_deduct_conserves_total_supply
//! ```

/// Two-field model of the vault + settlement accounting state.
///
/// `vault_balance`    — the value stored under `DataKey::Balance`.
/// `settlement_credit` — a running total of all amounts forwarded to settlement
///                       (tracks what the settlement contract has received for
///                       the purposes of total-supply conservation).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeductState {
    vault_balance: i128,
    settlement_credit: i128,
}

impl DeductState {
    /// Combined supply: must be invariant across a successful deduction.
    fn total_supply(self) -> i128 {
        self.vault_balance
            .checked_add(self.settlement_credit)
            .expect("modeled supply total must fit in i128")
    }
}

/// Model a single successful `deduct(amount, request_id)`.
///
/// Mirrors the contract's pre-condition checks and the arithmetic path only.
/// `request_id: u64` is accepted for signature parity with the contract but
/// is not used in balance arithmetic — forwarding it to settlement is a
/// cross-contract effect that Kani does not need to model here.
fn model_successful_deduct(
    state: DeductState,
    amount: i128,
    max_deduct: i128,
    _request_id: u64,
) -> DeductState {
    assert!(
        state.vault_balance >= 0,
        "vault balance must be non-negative"
    );
    assert!(
        state.settlement_credit >= 0,
        "settlement credit must be non-negative"
    );
    assert!(amount > 0, "deduct amount must be positive");
    assert!(max_deduct > 0, "max_deduct must be positive");
    assert!(
        amount <= max_deduct,
        "deduct amount must respect max_deduct"
    );
    assert!(state.vault_balance >= amount, "deduct must be funded");

    // Mirrors: let new_bal = current_bal.checked_sub(amount).unwrap();
    // and:     settlement_client.record_deduction(&amount, &request_id);
    DeductState {
        vault_balance: state
            .vault_balance
            .checked_sub(amount)
            .expect("funded deduct cannot underflow"),
        settlement_credit: state
            .settlement_credit
            .checked_add(amount)
            .expect("modeled settlement credit cannot overflow"),
    }
}

/// Model a successful `batch_deduct(items: Vec<(i128, u64)>)` over two items.
///
/// The contract validates all items before any state mutation (all-or-nothing),
/// accumulates a running total with `checked_add`, then subtracts once with
/// `checked_sub`.  This model mirrors that pattern for a 2-item batch.
fn model_successful_batch_deduct(
    state: DeductState,
    amount1: i128,
    request_id1: u64,
    amount2: i128,
    request_id2: u64,
    max_deduct: i128,
) -> DeductState {
    assert!(state.vault_balance >= 0, "vault balance must be non-negative");
    assert!(state.settlement_credit >= 0, "settlement credit must be non-negative");
    assert!(max_deduct > 0, "max_deduct must be positive");

    // Validation pass — mirrors the contract's loop before any mutation.
    assert!(amount1 > 0 && amount1 <= max_deduct, "item 1: invalid deduct amount");
    assert!(amount2 > 0 && amount2 <= max_deduct, "item 2: invalid deduct amount");

    // Running-total accumulation — mirrors: total_amount = total_amount.checked_add(amount).unwrap()
    let total = amount1
        .checked_add(amount2)
        .expect("batch total must not overflow");

    assert!(state.vault_balance >= total, "batch deduct must be funded");

    // Single balance update — mirrors: let new_bal = current_bal.checked_sub(total_amount).unwrap()
    let new_vault_balance = state
        .vault_balance
        .checked_sub(total)
        .expect("funded batch deduct cannot underflow");

    // Settlement credits for both items (request_ids forwarded, not used in arithmetic).
    let _ = request_id1;
    let _ = request_id2;
    let new_settlement_credit = state
        .settlement_credit
        .checked_add(total)
        .expect("modeled settlement credit cannot overflow");

    DeductState {
        vault_balance: new_vault_balance,
        settlement_credit: new_settlement_credit,
    }
}

// ---------------------------------------------------------------------------
// Kani proof harnesses
// ---------------------------------------------------------------------------

/// Prove: a successful `deduct` conserves the combined vault+settlement total.
///
/// For all symbolic `(balance, settlement_credit, amount, max_deduct, request_id)`
/// satisfying the contract's pre-conditions, `after.total_supply() == before.total_supply()`.
#[cfg(kani)]
#[kani::proof]
fn kani_deduct_conserves_total_supply() {
    let vault_balance: i128 = kani::any();
    let settlement_credit: i128 = kani::any();
    let amount: i128 = kani::any();
    let max_deduct: i128 = kani::any();
    let request_id: u64 = kani::any(); // full u64 range — must not affect result

    kani::assume(vault_balance >= 0);
    kani::assume(settlement_credit >= 0);
    kani::assume(amount > 0);
    kani::assume(max_deduct > 0);
    kani::assume(amount <= max_deduct);
    kani::assume(vault_balance >= amount);
    // Both the pre-state total and post-state settlement credit are modeled as
    // i128 values, matching the contract's checked arithmetic domain.
    kani::assume(vault_balance <= i128::MAX - settlement_credit);
    kani::assume(settlement_credit <= i128::MAX - amount);

    let before = DeductState {
        vault_balance,
        settlement_credit,
    };
    let before_total = before.total_supply();

    let after = model_successful_deduct(before, amount, max_deduct, request_id);

    assert_eq!(
        after.total_supply(),
        before_total,
        "successful deduct must conserve total supply"
    );
    assert_eq!(
        after.vault_balance,
        vault_balance - amount,
        "vault balance must decrease by exactly amount"
    );
    assert_eq!(
        after.settlement_credit,
        settlement_credit + amount,
        "settlement credit must increase by exactly amount"
    );
}

/// Prove: varying `request_id` has no effect on the conservation property.
///
/// Two deductions with identical `(balance, amount)` but different `request_id`
/// values must yield identical post-deduct `DeductState`.
#[cfg(kani)]
#[kani::proof]
fn kani_deduct_request_id_conserves_total_supply() {
    let vault_balance: i128 = kani::any();
    let settlement_credit: i128 = kani::any();
    let amount: i128 = kani::any();
    let max_deduct: i128 = kani::any();
    let request_id_a: u64 = kani::any();
    let request_id_b: u64 = kani::any();

    kani::assume(vault_balance >= 0);
    kani::assume(settlement_credit >= 0);
    kani::assume(amount > 0);
    kani::assume(max_deduct > 0);
    kani::assume(amount <= max_deduct);
    kani::assume(vault_balance >= amount);
    kani::assume(vault_balance <= i128::MAX - settlement_credit);
    kani::assume(settlement_credit <= i128::MAX - amount);

    let before = DeductState { vault_balance, settlement_credit };

    let after_a = model_successful_deduct(before, amount, max_deduct, request_id_a);
    let after_b = model_successful_deduct(before, amount, max_deduct, request_id_b);

    assert_eq!(
        after_a.vault_balance,
        after_b.vault_balance,
        "vault balance must not depend on request_id"
    );
    assert_eq!(
        after_a.settlement_credit,
        after_b.settlement_credit,
        "settlement credit must not depend on request_id"
    );
    assert_eq!(
        after_a.total_supply(),
        after_b.total_supply(),
        "total supply must not depend on request_id"
    );
}

/// Prove: a successful `batch_deduct` over a 2-item Vec<(i128, u64)> conserves
/// the combined vault+settlement total.
#[cfg(kani)]
#[kani::proof]
fn kani_batch_deduct_conserves_total_supply() {
    let vault_balance: i128 = kani::any();
    let settlement_credit: i128 = kani::any();
    let amount1: i128 = kani::any();
    let amount2: i128 = kani::any();
    let request_id1: u64 = kani::any();
    let request_id2: u64 = kani::any();
    let max_deduct: i128 = kani::any();

    kani::assume(vault_balance >= 0);
    kani::assume(settlement_credit >= 0);
    kani::assume(max_deduct > 0);
    kani::assume(amount1 > 0 && amount1 <= max_deduct);
    kani::assume(amount2 > 0 && amount2 <= max_deduct);
    // Both amounts fit in i128 running total.
    kani::assume(amount1 <= i128::MAX - amount2);
    let total = amount1 + amount2;
    kani::assume(vault_balance >= total);
    kani::assume(vault_balance <= i128::MAX - settlement_credit);
    kani::assume(settlement_credit <= i128::MAX - total);

    let before = DeductState { vault_balance, settlement_credit };
    let before_total = before.total_supply();

    let after = model_successful_batch_deduct(
        before, amount1, request_id1, amount2, request_id2, max_deduct,
    );

    assert_eq!(
        after.total_supply(),
        before_total,
        "batch deduct must conserve total supply"
    );
    assert_eq!(
        after.vault_balance,
        vault_balance - amount1 - amount2,
        "vault balance must decrease by sum of batch amounts"
    );
    assert_eq!(
        after.settlement_credit,
        settlement_credit + amount1 + amount2,
        "settlement credit must increase by sum of batch amounts"
    );
}

// ---------------------------------------------------------------------------
// Unit tests for the model itself (run under `cargo test`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modeled_deduct_moves_amount_without_changing_total() {
        let before = DeductState {
            vault_balance: 1_000,
            settlement_credit: 250,
        };

        let after = model_successful_deduct(before, 400, 500, 42_u64);

        assert_eq!(after.vault_balance, 600);
        assert_eq!(after.settlement_credit, 650);
        assert_eq!(after.total_supply(), before.total_supply());
    }

    #[test]
    fn modeled_deduct_request_id_does_not_change_balance() {
        let before = DeductState {
            vault_balance: 500,
            settlement_credit: 100,
        };
        let after_a = model_successful_deduct(before, 200, 500, 1_u64);
        let after_b = model_successful_deduct(before, 200, 500, u64::MAX);
        assert_eq!(after_a.vault_balance, after_b.vault_balance);
        assert_eq!(after_a.settlement_credit, after_b.settlement_credit);
    }

    #[test]
    #[should_panic(expected = "deduct must be funded")]
    fn modeled_deduct_rejects_unfunded_amount_before_mutation() {
        let before = DeductState {
            vault_balance: 99,
            settlement_credit: 0,
        };
        let _ = model_successful_deduct(before, 100, 100, 0_u64);
    }

    #[test]
    #[should_panic(expected = "deduct amount must respect max_deduct")]
    fn modeled_deduct_rejects_amount_above_max_deduct() {
        let before = DeductState {
            vault_balance: 100,
            settlement_credit: 0,
        };
        let _ = model_successful_deduct(before, 100, 99, 0_u64);
    }

    #[test]
    fn modeled_batch_deduct_conserves_total() {
        let before = DeductState {
            vault_balance: 1_000,
            settlement_credit: 0,
        };
        let after = model_successful_batch_deduct(before, 300, 1_u64, 200, 2_u64, 500);
        assert_eq!(after.vault_balance, 500);
        assert_eq!(after.settlement_credit, 500);
        assert_eq!(after.total_supply(), before.total_supply());
    }

    #[test]
    #[should_panic(expected = "batch deduct must be funded")]
    fn modeled_batch_deduct_rejects_unfunded_total() {
        let before = DeductState {
            vault_balance: 400,
            settlement_credit: 0,
        };
        // 300 + 200 = 500 > 400
        let _ = model_successful_batch_deduct(before, 300, 0_u64, 200, 0_u64, 500);
    }
}
