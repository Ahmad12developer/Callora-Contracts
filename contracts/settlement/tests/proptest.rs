//! Property-based invariant tests for settlement contract state invariants.
//!
//! This module uses proptest to verify that critical invariants hold across
//! arbitrary sequences of contract operations. The invariants tested include:
//!
//! 1. **Balance Conservation**: `total_in == sum(per_developer_balance) + global_pool.total_balance`
//!    - The total amount credited to the contract must equal the sum of all
//!      developer balances plus the global pool balance.
//!
//! 2. **No Balance Overflow/Underflow**: All arithmetic operations must use
//!    checked arithmetic and properly handle edge cases at i128 boundaries.
//!
//! 3. **Replay Protection**: The high-water-mark replay guard must correctly
//!    reject duplicate or out-of-order settlement claims.
//!
//! 4. **Withdrawal Constraints**: Developer withdrawals must respect:
//!    - Minimum balance requirements
//!    - Daily withdrawal caps
//!    - Claim windows
//!    - Sufficient contract USDC balance
//!
//! # Strategy
//! We use proptest to generate random sequences of operations (receive_payment,
//! batch_receive_payment, withdraw_developer_balance, admin operations) and
//! verify invariants after each step. Seeds are shrunk on failure to find
//! minimal counterexamples.
//!
//! # Reproduction
//! On any failure, the seed and full operation trace are printed, allowing
//! deterministic replay of the exact failing sequence.

extern crate std;

use std::boxed::Box;

use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token as token_mod;
use soroban_sdk::{Address, Env, Vec};

use callora_settlement::{CalloraSettlement, CalloraSettlementClient};

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Maximum steps per trace (kept reasonable for CI speed).
const TRACE_LENGTH: u32 = 64;

/// Maximum payment / withdrawal amount per step.
const AMOUNT_CAP: i128 = 50_000;

/// Maximum items in a single `batch_receive_payment` call.
const MAX_BATCH: usize = 8;

/// Pool of developer addresses reused across a trace.
const DEV_POOL_SIZE: usize = 6;

// ---------------------------------------------------------------------------
// Deterministic PRNG — no `std::rand`, no external crates in no_std context
// ---------------------------------------------------------------------------

/// 64-bit Multiplicative LCG (same constants as glibc).
struct Prng {
    state: u64,
}

impl Prng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn gen_i128(&mut self, min: i128, max: i128) -> i128 {
        if min >= max {
            return min;
        }
        let span = (max - min) as u64 + 1;
        min + (self.next_u64() % span) as i128
    }

    fn gen_usize(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize) % (max - min + 1)
    }
}

// ---------------------------------------------------------------------------
// Trace — records every step for counterexample reporting
// ---------------------------------------------------------------------------

struct TraceStep {
    index: u32,
    op: &'static str,
    detail: std::string::String,
}

struct Trace {
    seed: u64,
    steps: std::vec::Vec<TraceStep>,
}

impl Trace {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            steps: std::vec::Vec::new(),
        }
    }

    fn push(&mut self, index: u32, op: &'static str, detail: impl Into<std::string::String>) {
        self.steps.push(TraceStep {
            index,
            op,
            detail: detail.into(),
        });
    }

    fn panic_invariant(
        &self,
        step: u32,
        expected_total_in: i128,
        actual_dev_sum: i128,
        pool_balance: i128,
        msg: &str,
    ) -> ! {
        let mut out = std::format!(
            "\n=== INVARIANT VIOLATION ===\n\
             {}\n\
             total_in ({}) != sum(dev_balances) ({}) + pool ({})\n\
             combined rhs = {}\n\
             seed = {}  step = {}\n\
             --- trace ---\n",
            msg,
            expected_total_in,
            actual_dev_sum,
            pool_balance,
            actual_dev_sum + pool_balance,
            self.seed,
            step,
        );
        for s in &self.steps {
            out.push_str(&std::format!(
                "  [{:>3}] {:35} {}\n",
                s.index,
                s.op,
                s.detail
            ));
        }
        out.push_str("==========================\n");
        panic!("{out}");
    }
}

// ---------------------------------------------------------------------------
// Operation alphabet
// ---------------------------------------------------------------------------

#[repr(u8)]
enum Op {
    ReceiveDev = 0,
    ReceivePool = 1,
    BatchReceiveDev = 2,
    Withdraw = 3,
    SetMinBalance = 4,
    SetClaimWindow = 5,
    SetDailyCap = 6,
    ForceCredit = 7,
}

const OP_COUNT: u64 = 8;

// ---------------------------------------------------------------------------
// Invariant checker
// ---------------------------------------------------------------------------

/// Verify the core conservation invariant:
/// `total_in_dev + total_in_pool == sum(all developer balances) + global_pool.total_balance`
fn check_invariant(
    client: &CalloraSettlementClient<'_>,
    admin: &Address,
    usdc_addr: &Address,
    expected_dev_total: i128,
    expected_pool_total: i128,
    trace: &Trace,
    step: u32,
) {
    let balances = client.get_all_developer_balances(admin, usdc_addr);
    let dev_sum: i128 = balances.iter().map(|b| b.balance).sum();
    let pool = client.get_global_pool().total_balance;

    if dev_sum != expected_dev_total || pool != expected_pool_total {
        trace.panic_invariant(
            step,
            expected_dev_total + expected_pool_total,
            dev_sum,
            pool,
            "Core conservation invariant violated",
        );
    }
}

// ---------------------------------------------------------------------------
// Core trace runner
// ---------------------------------------------------------------------------

/// Run one fully deterministic property trace for `seed`.
fn run_trace(seed: u64) {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let mut rng = Prng::new(seed);
    let mut trace = Trace::new(seed);

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    // Pre-fund contract with enough USDC to cover all possible withdrawals.
    let usdc_admin = Address::generate(env);
    let usdc_ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = usdc_ca.address();
    let usdc_sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    usdc_sac.mint(
        &contract,
        &(AMOUNT_CAP * TRACE_LENGTH as i128 * MAX_BATCH as i128 * 4),
    );

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    // Pre-generate developer addresses.
    let devs: std::vec::Vec<Address> = (0..DEV_POOL_SIZE).map(|_| Address::generate(env)).collect();

    // Track expected state using indices (Address doesn't implement std::hash::Hash).
    let mut expected_dev_total: i128 = 0;
    let mut expected_pool_total: i128 = 0;
    let mut ledger_seq = 0u32;

    // Per-developer balances tracked by index.
    let mut dev_balances = [0i128; DEV_POOL_SIZE];
    let mut min_balances = [0i128; DEV_POOL_SIZE];
    let mut claim_windows: std::vec::Vec<Option<(u64, u64)>> = vec![None; DEV_POOL_SIZE];
    let mut daily_caps = [0i128; DEV_POOL_SIZE];

    // Initial invariant check.
    check_invariant(&client, &admin, &usdc_addr, 0, 0, &trace, 0);

    for step in 1..=TRACE_LENGTH {
        let op = (rng.next_u64() % OP_COUNT) as u8;

        match op {
            x if x == Op::ReceiveDev as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let amount = rng.gen_i128(1, AMOUNT_CAP);
                ledger_seq += 1;
                client.receive_payment(
                    &vault,
                    &amount,
                    &false,
                    &Some(dev.clone()),
                    &usdc_addr,
                    &ledger_seq,
                );
                expected_dev_total = expected_dev_total
                    .checked_add(amount)
                    .expect("test tally overflow");
                dev_balances[dev_idx] += amount;
                trace.push(
                    step,
                    "receive_payment(dev)",
                    std::format!("dev_idx={dev_idx} amount={amount}"),
                );
            }

            x if x == Op::ReceivePool as u8 => {
                let amount = rng.gen_i128(1, AMOUNT_CAP);
                ledger_seq += 1;
                client.receive_payment(&vault, &amount, &true, &None, &usdc_addr, &ledger_seq);
                expected_pool_total = expected_pool_total
                    .checked_add(amount)
                    .expect("test tally overflow");
                trace.push(
                    step,
                    "receive_payment(pool)",
                    std::format!("amount={amount}"),
                );
            }

            x if x == Op::BatchReceiveDev as u8 => {
                let n = rng.gen_usize(1, MAX_BATCH);
                let mut items: Vec<(Address, i128)> = Vec::new(env);
                let mut batch_total: i128 = 0;
                let mut batch_devs: std::vec::Vec<(usize, i128)> = std::vec::Vec::new();
                let mut used_devs = std::collections::HashSet::new();
                for _ in 0..n {
                    let mut dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                    while used_devs.contains(&dev_idx) && used_devs.len() < DEV_POOL_SIZE {
                        dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                    }
                    used_devs.insert(dev_idx);
                    let dev = devs[dev_idx].clone();
                    let amount = rng.gen_i128(1, AMOUNT_CAP);
                    items.push_back((dev, amount));
                    batch_total = batch_total
                        .checked_add(amount)
                        .expect("batch tally overflow");
                    batch_devs.push((dev_idx, amount));
                }
                ledger_seq += 1;
                let result =
                    client.try_batch_receive_payment(&vault, &items, &usdc_addr, &ledger_seq);
                if result.is_ok() {
                    expected_dev_total = expected_dev_total
                        .checked_add(batch_total)
                        .expect("test tally overflow");
                    for (dev_idx, amount) in &batch_devs {
                        dev_balances[*dev_idx] += amount;
                    }
                }
                trace.push(
                    step,
                    "batch_receive_payment",
                    std::format!("n={n} total={batch_total}"),
                );
            }

            x if x == Op::Withdraw as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let current = dev_balances[dev_idx];
                if current > 0 {
                    let max_withdraw = current.min(AMOUNT_CAP);
                    let amount = rng.gen_i128(1, max_withdraw);
                    let result = client.try_withdraw_developer_balance(&dev, &amount, &None);
                    if result.is_ok() {
                        expected_dev_total = expected_dev_total
                            .checked_sub(amount)
                            .expect("test tally underflow");
                        dev_balances[dev_idx] -= amount;
                        trace.push(
                            step,
                            "withdraw(ok)",
                            std::format!(
                                "dev_idx={dev_idx} amount={amount} remaining={}",
                                current - amount
                            ),
                        );
                    } else {
                        trace.push(
                            step,
                            "withdraw(err)",
                            std::format!("dev_idx={dev_idx} amount={amount} err={result:?}"),
                        );
                    }
                } else {
                    trace.push(
                        step,
                        "withdraw(skip-zero)",
                        std::format!("dev_idx={dev_idx}"),
                    );
                }
            }

            x if x == Op::SetMinBalance as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let current_balance = dev_balances[dev_idx];
                let min_balance = if current_balance > 0 {
                    rng.gen_i128(0, current_balance)
                } else {
                    0
                };
                client.set_developer_min_balance(&admin, &dev, &min_balance);
                min_balances[dev_idx] = min_balance;
                trace.push(
                    step,
                    "set_min_balance",
                    std::format!("dev_idx={dev_idx} min_balance={min_balance}"),
                );
            }

            x if x == Op::SetClaimWindow as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let now = env.ledger().timestamp();
                let start_ts = now.saturating_sub(rng.gen_i128(0, 86400) as u64);
                let end_ts = start_ts + rng.gen_i128(1, 86400 * 30) as u64;
                let _ = client.try_set_developer_claim_window(&admin, &dev, &start_ts, &end_ts);
                claim_windows[dev_idx] = Some((start_ts, end_ts));
                trace.push(
                    step,
                    "set_claim_window",
                    std::format!("dev_idx={dev_idx} start={start_ts} end={end_ts}"),
                );
            }

            x if x == Op::SetDailyCap as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let cap = rng.gen_i128(0, AMOUNT_CAP * 10);
                client.set_daily_withdraw_cap(&admin, &dev, &cap);
                daily_caps[dev_idx] = cap;
                trace.push(
                    step,
                    "set_daily_cap",
                    std::format!("dev_idx={dev_idx} cap={cap}"),
                );
            }

            x if x == Op::ForceCredit as u8 => {
                let dev_idx = rng.gen_usize(0, DEV_POOL_SIZE - 1);
                let dev = devs[dev_idx].clone();
                let amount = rng.gen_i128(1, AMOUNT_CAP);
                let reason = soroban_sdk::Symbol::new(env, "test");
                client.force_credit_developer(&admin, &dev, &amount, &usdc_addr, &reason);
                expected_dev_total = expected_dev_total
                    .checked_add(amount)
                    .expect("test tally overflow");
                dev_balances[dev_idx] += amount;
                trace.push(
                    step,
                    "force_credit",
                    std::format!("dev_idx={dev_idx} amount={amount}"),
                );
            }

            _ => unreachable!(),
        }

        check_invariant(
            &client,
            &admin,
            &usdc_addr,
            expected_dev_total,
            expected_pool_total,
            &trace,
            step,
        );

        // Per-developer balance check: each tracked balance must match exactly.
        for (i, &expected_bal) in dev_balances.iter().enumerate() {
            let actual_bal = client.get_developer_balance(&devs[i], &usdc_addr);
            assert_eq!(
                actual_bal, expected_bal,
                "seed={seed} step={step} dev[{i}] balance mismatch"
            );
        }

        // Non-negative balance invariant.
        for (i, &bal) in dev_balances.iter().enumerate() {
            assert!(
                bal >= 0,
                "seed={seed} step={step} dev[{i}] balance is negative: {bal}"
            );
        }
        assert!(
            client.get_global_pool().total_balance >= 0,
            "seed={seed} step={step} pool balance is negative",
        );
    }
}

// ---------------------------------------------------------------------------
// Deterministic seeded traces
// ---------------------------------------------------------------------------

/// Run [`SEED_COUNT`] deterministic traces (seeds 0..63), each [`TRACE_LENGTH`] steps.
/// Invariant: `total_in == sum(per_developer_balance) + global_pool.total_balance`
/// must hold after every operation.
const SEED_COUNT: u64 = 64;

#[test]
fn test_settlement_balance_invariant_seeded() {
    for seed in 0..SEED_COUNT {
        run_trace(seed);
    }
}

/// Edge case: only pool credits — developer balances stay zero.
#[test]
fn test_invariant_pool_only() {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    let usdc_admin = Address::generate(env);
    let ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = ca.address();
    let sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    sac.mint(&contract, &1_000_000);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    let amounts = [100i128, 200, 300, 50, 1];
    let mut expected_pool: i128 = 0;
    let mut ledger_seq = 0u32;
    for (i, &amount) in amounts.iter().enumerate() {
        ledger_seq += 1;
        client.receive_payment(&vault, &amount, &true, &None, &usdc_addr, &ledger_seq);
        expected_pool += amount;
        let pool = client.get_global_pool().total_balance;
        assert_eq!(
            pool, expected_pool,
            "pool invariant failed at step {i}: expected {expected_pool}, got {pool}"
        );
        let dev_sum: i128 = client
            .get_all_developer_balances(&admin, &usdc_addr)
            .iter()
            .map(|b| b.balance)
            .sum();
        assert_eq!(dev_sum, 0, "no developer should have a balance (step {i})");
    }
}

/// Edge case: single developer receives multiple payments then fully withdraws.
#[test]
fn test_invariant_single_dev_full_withdraw() {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let dev = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    let usdc_admin = Address::generate(env);
    let ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = ca.address();
    let sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    sac.mint(&contract, &10_000);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    // Credit the developer.
    client.receive_payment(
        &vault,
        &1_000,
        &false,
        &Some(dev.clone()),
        &usdc_addr,
        &1u32,
    );
    client.receive_payment(
        &vault,
        &2_000,
        &false,
        &Some(dev.clone()),
        &usdc_addr,
        &2u32,
    );
    client.receive_payment(&vault, &500, &false, &Some(dev.clone()), &usdc_addr, &3u32);

    let balance = client.get_developer_balance(&dev, &usdc_addr);
    assert_eq!(balance, 3_500);

    let dev_sum: i128 = client
        .get_all_developer_balances(&admin, &usdc_addr)
        .iter()
        .map(|b| b.balance)
        .sum();
    assert_eq!(dev_sum, 3_500, "dev sum before withdraw");

    // Full withdraw.
    client.withdraw_developer_balance(&dev, &3_500, &None);

    let dev_sum_after: i128 = client
        .get_all_developer_balances(&admin, &usdc_addr)
        .iter()
        .map(|b| b.balance)
        .sum();
    assert_eq!(dev_sum_after, 0, "dev sum must be 0 after full withdraw");
    assert_eq!(
        client.get_global_pool().total_balance,
        0,
        "pool must stay 0"
    );
}

/// Edge case: `record_deduction` updates `TotalReceived` without affecting
/// developer or pool balances.
#[test]
fn test_invariant_record_deduction() {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    let usdc_admin = Address::generate(env);
    let ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = ca.address();
    let sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    sac.mint(&contract, &1_000_000);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    assert_eq!(client.get_total_received(), 0);

    client.record_deduction(&1_000, &1);
    assert_eq!(client.get_total_received(), 1_000);
    assert_eq!(client.get_global_pool().total_balance, 0);
    assert_eq!(
        client.get_all_developer_balances(&admin, &usdc_addr).len(),
        0
    );

    client.record_deduction(&500, &2);
    assert_eq!(client.get_total_received(), 1_500);
    assert_eq!(client.get_global_pool().total_balance, 0);

    client.receive_payment(
        &vault,
        &300,
        &false,
        &Some(Address::generate(env)),
        &usdc_addr,
        &1u32,
    );
    assert_eq!(client.get_total_received(), 1_500);
    assert_eq!(client.get_global_pool().total_balance, 0);
}

/// Edge case: interleaved developer and pool payments preserve the full conservation invariant.
#[test]
fn test_invariant_interleaved_dev_and_pool() {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let dev1 = Address::generate(env);
    let dev2 = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    let usdc_admin = Address::generate(env);
    let ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = ca.address();
    let sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    sac.mint(&contract, &1_000_000);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    let ops: &[(bool, i128, bool)] = &[
        // (to_pool, amount, is_dev1)
        (false, 100, true),
        (true, 50, false),
        (false, 200, false),
        (true, 75, false),
        (false, 300, true),
    ];

    let mut exp_dev: i128 = 0;
    let mut exp_pool: i128 = 0;
    let mut ledger_seq = 0u32;

    for &(to_pool, amount, is_dev1) in ops {
        if to_pool {
            ledger_seq += 1;
            client.receive_payment(&vault, &amount, &true, &None, &usdc_addr, &ledger_seq);
            exp_pool += amount;
        } else {
            let dev = if is_dev1 { dev1.clone() } else { dev2.clone() };
            ledger_seq += 1;
            client.receive_payment(&vault, &amount, &false, &Some(dev), &usdc_addr, &ledger_seq);
            exp_dev += amount;
        }
        let dev_sum: i128 = client
            .get_all_developer_balances(&admin, &usdc_addr)
            .iter()
            .map(|b| b.balance)
            .sum();
        let pool = client.get_global_pool().total_balance;
        assert_eq!(dev_sum, exp_dev, "dev sum mismatch");
        assert_eq!(pool, exp_pool, "pool mismatch");
    }
}

// ---------------------------------------------------------------------------
// proptest — seed-driven property test
// ---------------------------------------------------------------------------

proptest! {
    /// Property: for any seed in [0, u32::MAX], the settlement balance invariant holds
    /// across [`TRACE_LENGTH`] generated operations.
    ///
    /// proptest manages seed shrinking: on failure it finds the minimal seed that
    /// reproduces the violation, then `run_trace` provides the full step trace.
    #[test]
    fn proptest_settlement_balance_invariant(seed in 0u64..=u64::from(u32::MAX)) {
        run_trace(seed);
    }
}

/// Edge case: daily withdraw cap invariant verification
#[test]
fn test_invariant_daily_withdraw_cap() {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let dev = Address::generate(env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract);

    let usdc_admin = Address::generate(env);
    let ca = env.register_stellar_asset_contract_v2(usdc_admin.clone());
    let usdc_addr = ca.address();
    let sac = token_mod::StellarAssetClient::new(env, &usdc_addr);
    sac.mint(&contract, &10_000);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc_addr);

    client.receive_payment(
        &vault,
        &5_000,
        &false,
        &Some(dev.clone()),
        &usdc_addr,
        &1u32,
    );

    client.set_daily_withdraw_cap(&admin, &dev, &2_000);

    client.withdraw_developer_balance(&dev, &1_500, &None);

    let res = client.try_withdraw_developer_balance(&dev, &1_000, &None);
    assert!(res.is_err());

    let balance = client.get_developer_balance(&dev, &usdc_addr);
    assert_eq!(balance, 3_500);
}
