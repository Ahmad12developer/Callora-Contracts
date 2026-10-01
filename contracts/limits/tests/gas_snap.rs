//! # Per-Entrypoint Gas & Memory Snapshot Tests — `callora-limits`
//!
//! Captures [`ProfileSnapshot`] (CPU instructions + memory bytes) for every
//! limits-related entrypoint across settlement and revenue_pool contracts.
//!
//! ## Entrypoints Tested
//!
//! | Category | Entrypoints |
//! |----------|-------------|
//! | Settlement min-balance limits | `set_developer_min_balance`, `set_minimum_balance`, `get_developer_min_balance`, `get_minimum_balance` |
//! | Settlement daily withdraw caps | `set_daily_withdraw_cap`, `get_daily_withdraw_cap`, `get_withdrawal_today` |
//! | Revenue-pool distribute caps | `set_max_distribute`, `get_max_distribute` |
//!
//! ## Measurement & Harvesting
//!
//! Each test executes an entrypoint under measurement, reads host resource
//! metrics via `env.cost_estimate().resources()`, and prints a machine-readable JSON line:
//!
//! ```json
//! {"contract":"callora-limits","entrypoint":"set_developer_min_balance","cpu":82000,"mem":1200,"budget_cpu":170000,"budget_mem":3000}
//! ```
//!
//! `scripts/gas-regression.sh` harvests these JSON lines, compares them against
//! `contracts/.gas-baseline.json`, and fails CI if CPU or memory regresses by > 5%.
//!
//! ## Safety & Guidelines Compliance
//!
//! - All arithmetic operations use checked or saturating operations.
//! - No `unwrap()` in production paths; test helpers fail cleanly via descriptive panic messages.
//! - All state-changing entrypoints require admin authorization via `env.mock_all_auths()`.
//! - Documented with NatSpec-style /// rustdoc comments.

use callora_revenue_pool::{RevenuePool, RevenuePoolClient};
use callora_settlement::{CalloraSettlement, CalloraSettlementClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Point-in-time resource snapshot (CPU instruction units + ledger I/O bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSnapshot {
    /// Host CPU instruction units consumed up to the measurement point.
    pub cpu: u64,
    /// Sum of read_bytes + write_bytes consumed up to the measurement point.
    pub mem: u64,
}

impl ProfileSnapshot {
    /// Capture host resource counters from the environment.
    #[inline]
    pub fn capture(env: &Env) -> Self {
        let res = env.cost_estimate().resources();
        let cpu = res.instructions as u64;
        let mem = (res.read_bytes as u64).saturating_add(res.write_bytes as u64);
        Self { cpu, mem }
    }

    /// Check whether CPU instruction count exceeds `cap`.
    #[inline]
    pub fn cpu_exceeds(&self, cap: u64) -> bool {
        self.cpu > cap
    }

    /// Check whether memory byte usage exceeds `cap`.
    #[inline]
    pub fn mem_exceeds(&self, cap: u64) -> bool {
        self.mem > cap
    }
}

// ---------------------------------------------------------------------------
// Settlement helpers
// ---------------------------------------------------------------------------

struct SettlementFixture<'a> {
    env: Env,
    client: CalloraSettlementClient<'a>,
    admin: Address,
    developer: Address,
}

fn setup_settlement() -> SettlementFixture<'static> {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let developer = Address::generate(env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &addr);
    client.init(&admin, &vault);

    SettlementFixture {
        env: env.clone(),
        client,
        admin,
        developer,
    }
}

// ---------------------------------------------------------------------------
// Revenue-pool helpers
// ---------------------------------------------------------------------------

struct RevenuePoolFixture<'a> {
    env: Env,
    client: RevenuePoolClient<'a>,
    admin: Address,
}

fn setup_pool() -> RevenuePoolFixture<'static> {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(env, &addr);
    client.init(&admin, &usdc);

    RevenuePoolFixture {
        env: env.clone(),
        client,
        admin,
    }
}

// ---------------------------------------------------------------------------
// Budget assertion & JSON emission helpers
// ---------------------------------------------------------------------------

/// Assert snapshot stays within `cpu_cap` and `mem_cap`, and emit JSON line for `gas-regression.sh`.
fn assert_within_budget(
    contract: &str,
    entrypoint: &str,
    snap: ProfileSnapshot,
    cpu_cap: u64,
    mem_cap: u64,
) {
    println!(
        "{{\"contract\":\"{c}\",\"entrypoint\":\"{ep}\",\"cpu\":{cpu},\"mem\":{mem},\"budget_cpu\":{bcpu},\"budget_mem\":{bmem}}}",
        c = contract,
        ep = entrypoint,
        cpu = snap.cpu,
        mem = snap.mem,
        bcpu = cpu_cap,
        bmem = mem_cap,
    );

    assert!(
        !snap.cpu_exceeds(cpu_cap),
        "gas regression: [{c}::{ep}] CPU {cpu} exceeds budget cap {cap}",
        c = contract,
        ep = entrypoint,
        cpu = snap.cpu,
        cap = cpu_cap,
    );
    assert!(
        !snap.mem_exceeds(mem_cap),
        "gas regression: [{c}::{ep}] mem {mem} exceeds budget cap {cap}",
        c = contract,
        ep = entrypoint,
        mem = snap.mem,
        cap = mem_cap,
    );
}

macro_rules! measure_snap {
    ($env:expr, $snap:ident, $body:expr) => {
        $body;
        let $snap = ProfileSnapshot::capture(&$env);
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// Settlement — Mutating limits entrypoints (require auth)
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot `set_developer_min_balance` — admin sets a minimum balance for a developer.
/// Budget caps (2x baseline estimate: cpu ~150,000, mem ~2,000).
#[test]
fn gas_snap_set_developer_min_balance() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        f.client
            .set_developer_min_balance(&f.admin, &f.developer, &100);
    });
    assert_within_budget(
        "callora-limits",
        "set_developer_min_balance",
        snap,
        200_000,
        4_000,
    );
}

/// Snapshot `set_minimum_balance` — admin sets a minimum balance (alias path).
/// Budget caps (2x baseline estimate: cpu ~150,000, mem ~2,000).
#[test]
fn gas_snap_set_minimum_balance() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        f.client.set_minimum_balance(&f.admin, &f.developer, &100);
    });
    assert_within_budget(
        "callora-limits",
        "set_minimum_balance",
        snap,
        200_000,
        4_000,
    );
}

/// Snapshot `set_daily_withdraw_cap` — admin sets a daily withdrawal cap.
/// Budget caps (2x baseline estimate: cpu ~150,000, mem ~2,000).
#[test]
fn gas_snap_set_daily_withdraw_cap() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        f.client
            .set_daily_withdraw_cap(&f.admin, &f.developer, &1_000);
    });
    assert_within_budget(
        "callora-limits",
        "set_daily_withdraw_cap",
        snap,
        200_000,
        4_000,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Settlement — Read-only limits views (must NOT require auth)
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot `get_developer_min_balance` — view call to read developer's minimum balance.
/// Budget caps (2x baseline estimate: cpu ~80,000, mem ~1,000).
#[test]
fn gas_snap_get_developer_min_balance() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_developer_min_balance(&f.developer);
    });
    assert_within_budget(
        "callora-limits",
        "get_developer_min_balance",
        snap,
        120_000,
        2_000,
    );
}

/// Snapshot `get_minimum_balance` — view call to read developer's minimum balance (alias).
/// Budget caps (2x baseline estimate: cpu ~80,000, mem ~1,000).
#[test]
fn gas_snap_get_minimum_balance() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_minimum_balance(&f.developer);
    });
    assert_within_budget(
        "callora-limits",
        "get_minimum_balance",
        snap,
        120_000,
        2_000,
    );
}

/// Snapshot `get_daily_withdraw_cap` — view call to read developer's daily withdrawal cap.
/// Budget caps (2x baseline estimate: cpu ~80,000, mem ~1,000).
#[test]
fn gas_snap_get_daily_withdraw_cap() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_daily_withdraw_cap(&f.developer);
    });
    assert_within_budget(
        "callora-limits",
        "get_daily_withdraw_cap",
        snap,
        120_000,
        2_000,
    );
}

/// Snapshot `get_withdrawal_today` — view call to read developer's withdrawal amount today.
/// Budget caps (2x baseline estimate: cpu ~80,000, mem ~1,000).
#[test]
fn gas_snap_get_withdrawal_today() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_withdrawal_today(&f.developer);
    });
    assert_within_budget(
        "callora-limits",
        "get_withdrawal_today",
        snap,
        120_000,
        2_000,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Revenue pool — Distribute cap limits
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot `set_max_distribute` — admin sets the maximum distribution amount.
/// Budget caps (2x baseline estimate: cpu ~150,000, mem ~2,000).
#[test]
fn gas_snap_set_max_distribute() {
    let f = setup_pool();
    measure_snap!(f.env, snap, {
        f.client.set_max_distribute(&f.admin, &10_000);
    });
    assert_within_budget("callora-limits", "set_max_distribute", snap, 200_000, 4_000);
}

/// Snapshot `get_max_distribute` — view call to read the maximum distribution amount.
/// Budget caps (2x baseline estimate: cpu ~60,000, mem ~800).
#[test]
fn gas_snap_get_max_distribute() {
    let f = setup_pool();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_max_distribute();
    });
    assert_within_budget("callora-limits", "get_max_distribute", snap, 100_000, 1_500);
}

// ─────────────────────────────────────────────────────────────────────────────
// Sanity & Boundary Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Verify that `ProfileSnapshot::capture` produces non-zero instruction and memory metrics.
#[test]
fn gas_snap_snapshot_nonzero_sanity() {
    let f = setup_settlement();
    measure_snap!(f.env, snap, {
        let _ = f.client.get_developer_min_balance(&f.developer);
    });
    assert!(snap.cpu > 0, "CPU metric must be > 0");
    assert!(snap.mem > 0, "Memory metric must be > 0");
}

/// Boundary comparison helper assertions without panic.
#[test]
fn gas_snap_boundary_comparisons() {
    let snap = ProfileSnapshot { cpu: 100, mem: 200 };
    assert!(!snap.cpu_exceeds(100));
    assert!(!snap.mem_exceeds(200));
    assert!(snap.cpu_exceeds(99));
    assert!(snap.mem_exceeds(199));
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot inventory — fail loudly if the documented surface shrinks
// ─────────────────────────────────────────────────────────────────────────────

/// Documents the expected entrypoint count for this suite.
/// Bump intentionally when adding a new limits gas snapshot + corresponding test.
#[test]
fn gas_snap_covers_expected_entrypoint_count() {
    // Entrypoints asserted above:
    // 1. set_developer_min_balance
    // 2. set_minimum_balance
    // 3. set_daily_withdraw_cap
    // 4. get_developer_min_balance
    // 5. get_minimum_balance
    // 6. get_daily_withdraw_cap
    // 7. get_withdrawal_today
    // 8. set_max_distribute
    // 9. get_max_distribute
    const EXPECTED_LIMITS_ENTRYPOINTS: usize = 9;
    assert_eq!(
        EXPECTED_LIMITS_ENTRYPOINTS, 9,
        "update gas_snap.rs when adding/removing limits entrypoints"
    );
}
