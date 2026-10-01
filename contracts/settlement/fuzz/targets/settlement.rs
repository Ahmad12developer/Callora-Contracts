//! Fuzz target: batch settlement via `batch_settle`.
//!
//! The fuzzer generates arbitrary byte inputs and interprets them as a list of
//! [`SettleInput`] items (developer index, recipient mode, amount). These are
//! fed to `batch_settle` on a pre-initialized settlement contract that holds
//! USDC and has developers with non-zero balances.
//!
//! # Properties checked on every execution
//!
//! 1. **No unexpected panic** — if the call panics (e.g. because a recipient
//!    is the contract itself), all state must be rolled back by the Soroban
//!    runtime and the contract USDC + per-developer balances must be unchanged.
//!
//! 2. **Batch-size overflow** — when the number of settlement items exceeds 64,
//!    every outcome must be [`SettleOutcome::OtherError`] and no state may
//!    change.
//!
//! 3. **Outcome–precondition consistency** — each outcome must match the
//!    corresponding input:
//!    - [`SettleOutcome::Success`] only when amount > 0, the developer has
//!      sufficient balance, and the recipient is not the contract.
//!    - [`SettleOutcome::AmountNotPositive`] only when amount ≤ 0.
//!    - [`SettleOutcome::InsufficientBalance`] only when amount > 0 and the
//!      developer's balance (including prior successful settlements in the
//!      same batch) is less than the requested amount.
//!
//! 4. **Conservation** — the total USDC decrease at the contract must exactly
//!    equal the sum of amounts from all [`SettleOutcome::Success`] items, and
//!    each developer's balance must decrease by exactly the sum of their
//!    successful withdrawal amounts.
//!
//! # Running
//! ```bash
//! cargo fuzz run settlement
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec as SorobanVec};

use callora_settlement::batch::{SettleInput, SettleOutcome};
use callora_settlement::CalloraSettlement;

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Number of distinct developer addresses in the fixed recipient pool.
const DEV_POOL_SIZE: usize = 6;

/// Per-developer initial USDC balance credited via vault payment.
const INITIAL_DEV_BALANCE: i128 = 1_000_000;

/// Maximum batch size before the contract blanket-rejects with `OtherError`.
const MAX_BATCH_SIZE: u32 = 64;

/// Each settlement item is encoded as 10 bytes:
///   byte 0:       developer index (mod DEV_POOL_SIZE)
///   byte 1:       to-address mode
///                   0 → None (recipient defaults to developer)
///                   1 → admin
///                   2 → contract itself (provokes a panic)
///                   3+ → other developer (mod DEV_POOL_SIZE)
///   bytes 2..=9:  amount as little-endian i64 → i128
const BYTES_PER_INPUT: usize = 10;

// ---------------------------------------------------------------------------
// Fuzz harness
// ---------------------------------------------------------------------------

fuzz_target!(|data: &[u8]| {
    if data.len() < BYTES_PER_INPUT {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();

    // ── Setup ──────────────────────────────────────────────────────────
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);

    let contract_addr = env.register(CalloraSettlement, ());
    let client = callora_settlement::CalloraSettlementClient::new(&env, &contract_addr);

    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let usdc_admin = StellarAssetClient::new(&env, &usdc_addr);
    let usdc_token = TokenClient::new(&env, &usdc_addr);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    // Fixed developer pool – addresses are stable for this fuzzer invocation.
    let devs: std::vec::Vec<Address> = (0..DEV_POOL_SIZE)
        .map(|_| Address::generate(&env))
        .collect();

    // Credit each developer via vault payment with unique ledger seq.
    for (i, dev) in devs.iter().enumerate() {
        client.receive_payment(
            &vault,
            &INITIAL_DEV_BALANCE,
            &false,
            &Some(dev.clone()),
            &usdc_addr,
            &((i as u32 + 1) * 10),
        );
    }

    // Mint USDC to the contract so it can honour withdrawals.
    let total_dev_balance = INITIAL_DEV_BALANCE * DEV_POOL_SIZE as i128;
    usdc_admin.mint(&contract_addr, &total_dev_balance);

    // ── Parse fuzzer bytes into SettleInput items ──────────────────────
    let input_count = data.len() / BYTES_PER_INPUT;
    let mut settlements: SorobanVec<SettleInput> = SorobanVec::new(&env);

    for chunk in data.chunks(BYTES_PER_INPUT).take(input_count) {
        let dev_idx = (chunk[0] as usize) % DEV_POOL_SIZE;
        let to_mode = chunk[1];
        let amount = i128::from(i64::from_le_bytes(chunk[2..10].try_into().unwrap()));

        let to = match to_mode {
            0 => None,
            1 => Some(admin.clone()),
            2 => Some(contract_addr.clone()),
            _ => Some(devs[((to_mode as usize - 3) % DEV_POOL_SIZE)].clone()),
        };

        settlements.push_back(SettleInput {
            developer: devs[dev_idx].clone(),
            amount,
            to,
        });
    }

    let n = settlements.len();

    // ── Snapshot pre-state ─────────────────────────────────────────────
    let contract_usdc_before = usdc_token.balance(&contract_addr);
    let dev_balances_before: std::vec::Vec<i128> = devs
        .iter()
        .map(|d| client.get_developer_balance(d, &usdc_addr))
        .collect();

    // ── Execute (catch panics from e.g. `to == contract`) ──────────────
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.batch_settle(&settlements)
    }));

    let contract_usdc_after = usdc_token.balance(&contract_addr);
    let dev_balances_after: std::vec::Vec<i128> = devs
        .iter()
        .map(|d| client.get_developer_balance(d, &usdc_addr))
        .collect();

    // ── Panic path: verify full state rollback ─────────────────────────
    if result.is_err() {
        // Soroban rolls back all state when a contract call panics.
        assert_eq!(
            contract_usdc_after, contract_usdc_before,
            "Contract USDC balance changed after a panic"
        );
        for (i, bal) in dev_balances_after.iter().enumerate() {
            assert_eq!(
                *bal, dev_balances_before[i],
                "Developer {i} balance changed after a panic"
            );
        }
        return;
    }

    let outcomes = result.unwrap();

    // ── Batch-size overflow guard ──────────────────────────────────────
    if n > MAX_BATCH_SIZE {
        assert_eq!(
            outcomes.len(),
            n,
            "Outcome count must match input count when batch > 64"
        );
        for i in 0..n {
            assert_eq!(
                outcomes.get(i).unwrap(),
                SettleOutcome::OtherError,
                "Item {i} must be OtherError when batch > 64"
            );
        }
        assert_eq!(
            contract_usdc_after, contract_usdc_before,
            "Contract USDC changed on oversized batch"
        );
        for (i, bal) in dev_balances_after.iter().enumerate() {
            assert_eq!(
                *bal, dev_balances_before[i],
                "Developer {i} balance changed on oversized batch"
            );
        }
        return;
    }

    // ── Per-outcome consistency & conservation ─────────────────────────
    assert_eq!(outcomes.len(), n, "Outcome count must match input count");

    let mut expected_dev_deltas: std::vec::Vec<i128> = vec![0i128; DEV_POOL_SIZE];

    for i in 0..n {
        let input = settlements.get(i).unwrap();
        let outcome = outcomes.get(i).unwrap();

        let dev_idx = devs
            .iter()
            .position(|d| d == &input.developer)
            .expect("developer must exist in pool");

        // Running balance for this developer after prior settlements.
        let current_dev_balance = dev_balances_before[dev_idx] + expected_dev_deltas[dev_idx];

        match outcome {
            SettleOutcome::Success => {
                assert!(
                    input.amount > 0,
                    "Success with non-positive amount {}",
                    input.amount
                );
                assert!(
                    input.amount <= current_dev_balance,
                    "Success with amount {} > balance {}",
                    input.amount,
                    current_dev_balance
                );
                // The contract panics when recipient is itself, so a
                // successful outcome proves the recipient was different.
                assert_ne!(
                    input.to.as_ref(),
                    Some(&contract_addr),
                    "Success with contract as recipient"
                );

                expected_dev_deltas[dev_idx] -= input.amount;
            }
            SettleOutcome::AmountNotPositive => {
                assert!(
                    input.amount <= 0,
                    "AmountNotPositive with positive amount {}",
                    input.amount
                );
            }
            SettleOutcome::InsufficientBalance => {
                assert!(
                    input.amount > 0,
                    "InsufficientBalance with non-positive amount {}",
                    input.amount
                );
                assert!(
                    input.amount > current_dev_balance,
                    "InsufficientBalance with amount {} <= balance {}",
                    input.amount,
                    current_dev_balance
                );
            }
            SettleOutcome::ClaimWindowClosed
            | SettleOutcome::DailyWithdrawCapExceeded
            | SettleOutcome::DeveloperBalanceUnderflow
            | SettleOutcome::OtherError => {}
        }
    }

    // ── Conservation: contract USDC ────────────────────────────────────
    let actual_contract_delta = contract_usdc_after - contract_usdc_before;
    let expected_contract_delta: i128 = expected_dev_deltas.iter().sum();
    assert_eq!(
        actual_contract_delta, expected_contract_delta,
        "Contract USDC conservation violation: actual delta \
         {actual_contract_delta} != expected {expected_contract_delta}"
    );

    // ── Conservation: per-developer balances ───────────────────────────
    for i in 0..DEV_POOL_SIZE {
        let actual_delta = dev_balances_after[i] - dev_balances_before[i];
        assert_eq!(
            actual_delta, expected_dev_deltas[i],
            "Developer {i} balance conservation violation: \
             actual delta {actual_delta} != expected {}",
            expected_dev_deltas[i]
        );
    }
});
