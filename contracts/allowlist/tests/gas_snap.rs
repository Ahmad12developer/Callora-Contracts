//! # Per-Entrypoint Gas & Memory Snapshot Tests — `callora-allowlist`
//!
//! Captures [`ProfileSnapshot`] (CPU instructions + memory bytes) for every
//! allowlist entrypoint (`add_address`, `clear_all`, `get_allowlist`,
//! `set_allowed_depositor`, `clear_allowed_depositors`, `is_authorized_depositor`)
//! and asserts that each measurement stays within specified budget ceilings.
//!
//! ## Measurement & Harvesting
//!
//! Each test executes an allowlist entrypoint under measurement, reads host resource
//! metrics via `env.cost_estimate().resources()`, and prints a machine-readable JSON line:
//!
//! ```json
//! {"contract":"callora-allowlist","entrypoint":"add_address","cpu":82000,"mem":1200,"budget_cpu":170000,"budget_mem":3000}
//! ```
//!
//! `scripts/gas-regression.sh` harvests these JSON lines, compares them against
//! `contracts/.gas-baseline.json`, and fails CI if CPU or memory regresses by > 5%.
//!
//! ## Safety & Guidelines Compliance
//!
//! - All arithmetic operations use checked or saturating operations (`checked_mul`, `checked_div`, `saturating_add`).
//! - No `unwrap()` in production paths; test helpers fail cleanly via descriptive panic messages.
//! - All state-changing entrypoints require owner authorization via `env.mock_all_auths()`.
//! - Documented with NatDoc / RustDoc comments (`///`).

extern crate std;
use std::println;

use callora_vault::{CalloraVault, CalloraVaultClient};
use soroban_sdk::{testutils::Address as _, token, Address, Env};

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

/// Helper fixture to set up an initialized vault contract for allowlist operations.
struct AllowlistFixture<'a> {
    env: Env,
    client: CalloraVaultClient<'a>,
    owner: Address,
    depositor: Address,
}

fn setup_allowlist_fixture() -> AllowlistFixture<'static> {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let owner = Address::generate(env);
    let depositor = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);

    let usdc_addr = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let usdc_admin = token::StellarAssetClient::new(env, &usdc_addr);
    usdc_admin.mint(&vault_addr, &1_000);

    client.init(&owner, &usdc_addr, &Some(1_000), &None, &None, &None, &None);

    AllowlistFixture {
        env: env.clone(),
        client,
        owner,
        depositor,
    }
}

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

/// Assert that `after` measurement has not regressed by more than `pct` percent relative to `before`.
fn assert_no_regression(
    entrypoint: &str,
    before: ProfileSnapshot,
    after: ProfileSnapshot,
    pct: u64,
) {
    let cpu_cap = before
        .cpu
        .checked_mul(100u64.saturating_add(pct))
        .and_then(|v| v.checked_div(100))
        .expect("overflow computing cpu regression cap");
    let mem_cap = before
        .mem
        .checked_mul(100u64.saturating_add(pct))
        .and_then(|v| v.checked_div(100))
        .expect("overflow computing mem regression cap");

    assert_within_budget("callora-allowlist", entrypoint, after, cpu_cap, mem_cap);
}

macro_rules! measure_snap {
    ($env:expr, $snap:ident, $body:expr) => {
        $body;
        let $snap = ProfileSnapshot::capture(&$env);
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// Allowlist Per-Entrypoint Snapshot Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot `add_address` — Owner adds an address to the allowlist.
/// Budget caps (2x baseline: cpu ~82,000 -> 170,000, mem ~1,200 -> 3,000).
#[test]
fn gas_snap_add_address() {
    let f = setup_allowlist_fixture();
    measure_snap!(f.env, snap, {
        f.client.add_address(&f.owner, &f.depositor);
    });
    assert_within_budget("callora-allowlist", "add_address", snap, 170_000, 3_000);
}

/// Snapshot `clear_all` — Owner clears all addresses from the allowlist.
/// Budget caps (2x baseline: cpu ~85,000 -> 180,000, mem ~1,220 -> 3,000).
#[test]
fn gas_snap_clear_all() {
    let f = setup_allowlist_fixture();
    f.client.add_address(&f.owner, &f.depositor);
    measure_snap!(f.env, snap, {
        f.client.clear_all(&f.owner);
    });
    assert_within_budget("callora-allowlist", "clear_all", snap, 180_000, 3_000);
}

/// Snapshot `get_allowlist` — Public view call to read current allowlist entries.
/// Budget caps (2x baseline: cpu ~40,000 -> 90,000, mem ~504 -> 1,100).
#[test]
fn gas_snap_get_allowlist() {
    let f = setup_allowlist_fixture();
    f.client.add_address(&f.owner, &f.depositor);
    measure_snap!(f.env, snap, {
        let _ = f.client.get_allowlist();
    });
    assert_within_budget("callora-allowlist", "get_allowlist", snap, 90_000, 1_100);
}

/// Snapshot `set_allowed_depositor` — Legacy allowlist addition entrypoint.
/// Budget caps (2x baseline: cpu 80,805 -> 170,000, mem 1,168 -> 3,000).
#[test]
fn gas_snap_set_allowed_depositor() {
    let f = setup_allowlist_fixture();
    measure_snap!(f.env, snap, {
        f.client
            .set_allowed_depositor(&f.owner, &Some(f.depositor.clone()));
    });
    assert_within_budget(
        "callora-allowlist",
        "set_allowed_depositor",
        snap,
        170_000,
        3_000,
    );
}

/// Snapshot `clear_allowed_depositors` — Legacy allowlist clear entrypoint.
/// Budget caps (2x baseline: cpu 85,379 -> 180,000, mem 1,216 -> 3,000).
#[test]
fn gas_snap_clear_allowed_depositors() {
    let f = setup_allowlist_fixture();
    f.client
        .set_allowed_depositor(&f.owner, &Some(f.depositor.clone()));
    measure_snap!(f.env, snap, {
        f.client.clear_allowed_depositors(&f.owner);
    });
    assert_within_budget(
        "callora-allowlist",
        "clear_allowed_depositors",
        snap,
        180_000,
        3_000,
    );
}

/// Snapshot `is_authorized_depositor` — View call to check depositor authorization.
/// Budget caps (2x baseline: cpu 43,314 -> 90,000, mem 504 -> 1,100).
#[test]
fn gas_snap_is_authorized_depositor() {
    let f = setup_allowlist_fixture();
    f.client.add_address(&f.owner, &f.depositor);
    measure_snap!(f.env, snap, {
        let _ = f.client.is_authorized_depositor(&f.depositor);
    });
    assert_within_budget(
        "callora-allowlist",
        "is_authorized_depositor",
        snap,
        90_000,
        1_100,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Relative Growth & Invariant Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Adding a second distinct address must stay within 50% relative growth of the first `add_address` call.
#[test]
fn gas_snap_add_address_second_call_relative() {
    let f = setup_allowlist_fixture();
    let dep2 = Address::generate(&f.env);

    measure_snap!(f.env, snap_first, {
        f.client.add_address(&f.owner, &f.depositor);
    });

    measure_snap!(f.env, snap_second, {
        f.client.add_address(&f.owner, &dep2);
    });

    assert_no_regression("add_address_second_vs_first", snap_first, snap_second, 50);
}

/// A second idempotent `clear_all` call must stay within 50% relative cost of the first call.
#[test]
fn gas_snap_clear_all_idempotent_relative() {
    let f = setup_allowlist_fixture();
    f.client.add_address(&f.owner, &f.depositor);

    measure_snap!(f.env, snap_first, {
        f.client.clear_all(&f.owner);
    });

    measure_snap!(f.env, snap_second, {
        f.client.clear_all(&f.owner);
    });

    assert_no_regression(
        "clear_all_idempotent_second_vs_first",
        snap_first,
        snap_second,
        50,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Sanity & Boundary Tests
// ─────────────────────────────────────────────────────────────────────────────

/// Verify that `ProfileSnapshot::capture` produces non-zero instruction and memory metrics.
#[test]
fn gas_snap_snapshot_nonzero_sanity() {
    let f = setup_allowlist_fixture();
    measure_snap!(f.env, snap, {
        let _ = f.client.is_authorized_depositor(&f.depositor);
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
