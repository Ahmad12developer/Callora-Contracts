//! # Per-Entrypoint Gas & Memory Snapshot Tests — `callora-topics`
//!
//! Captures CPU instruction count and memory byte usage for every public
//! entrypoint of the [`CalloraTopics`] contract and asserts that each
//! measurement stays within specified budget ceilings.
//!
//! ## Measurement & Harvesting
//!
//! Each test executes a topics entrypoint under measurement, reads host
//! resource metrics via `env.cost_estimate().resources()`, and prints a
//! machine-readable JSON line to stdout:
//!
//! ```json
//! {"contract":"callora-topics","entrypoint":"init","cpu":82000,"mem":1200,"budget_cpu":170000,"budget_mem":3000}
//! ```
//!
//! `scripts/gas-regression.sh` harvests these JSON lines, compares them
//! against `contracts/.gas-baseline.json`, and fails CI if CPU or memory
//! regresses by > 5 %.
//!
//! ## Coverage
//!
//! | Entrypoint         | Auth required | Test                               |
//! |--------------------|---------------|------------------------------------|
//! | `init`             | ✓             | [`gas_snap_init`]                  |
//! | `register_topic`   | ✓             | [`gas_snap_register_topic`]        |
//! | `deactivate`       | ✓             | [`gas_snap_deactivate`]            |
//! | `get_topic`        | ✗             | [`gas_snap_get_topic`]             |
//! | `is_active`        | ✗             | [`gas_snap_is_active`]             |
//! | `topic_count`      | ✗             | [`gas_snap_topic_count`]           |
//! | `get_admin`        | ✗             | [`gas_snap_get_admin`]             |
//!
//! Closes CalloraOrg/Callora-Contracts#913.

extern crate std;
use std::println;

use callora_topics::{CalloraTopics, CalloraTopicsClient};
use soroban_sdk::{testutils::Address as _, Address, Env, String, Symbol};

// ---------------------------------------------------------------------------
// ProfileSnapshot — lightweight resource counter capture
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of host resource counters (CPU + memory).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSnapshot {
    /// Host CPU instruction units consumed up to the measurement point.
    pub cpu: u64,
    /// Sum of `read_bytes + write_bytes` consumed up to the measurement point.
    pub mem: u64,
}

impl ProfileSnapshot {
    /// Capture host resource counters from the environment at this instant.
    #[inline]
    pub fn capture(env: &Env) -> Self {
        let res = env.cost_estimate().resources();
        let cpu = res.instructions as u64;
        let mem = (res.read_bytes as u64).saturating_add(res.write_bytes as u64);
        Self { cpu, mem }
    }

    /// Returns `true` if the CPU count exceeds `cap`.
    #[inline]
    pub fn cpu_exceeds(&self, cap: u64) -> bool {
        self.cpu > cap
    }

    /// Returns `true` if the memory byte count exceeds `cap`.
    #[inline]
    pub fn mem_exceeds(&self, cap: u64) -> bool {
        self.mem > cap
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Shared test fixture: a deployed + initialized [`CalloraTopics`] contract.
struct TopicsFixture<'a> {
    env: Env,
    client: CalloraTopicsClient<'a>,
    admin: Address,
}

/// Build a static-lifetime fixture.  The `Env` is `Box::leak`ed so the
/// [`CalloraTopicsClient`] lifetime can be `'static` across all tests.
fn setup() -> TopicsFixture<'static> {
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let admin = Address::generate(env);
    let contract_id = env.register(CalloraTopics, ());
    let client = CalloraTopicsClient::new(env, &contract_id);
    client.init(&admin);

    TopicsFixture {
        env: env.clone(),
        client,
        admin,
    }
}

/// Convenience: register a single topic with a given name in `f`.
fn register_one(f: &TopicsFixture<'_>, name: Symbol) {
    let desc = String::from_str(&f.env, "test topic");
    let owner = Address::generate(&f.env);
    f.client.register_topic(&f.admin, &name, &desc, &owner);
}

// ---------------------------------------------------------------------------
// Budget assertion helpers
// ---------------------------------------------------------------------------

/// Assert a snapshot stays within `cpu_cap`/`mem_cap` and emit a JSON line
/// for `gas-regression.sh`.
fn assert_within_budget(entrypoint: &str, snap: ProfileSnapshot, cpu_cap: u64, mem_cap: u64) {
    println!(
        "{{\"contract\":\"callora-topics\",\"entrypoint\":\"{ep}\",\"cpu\":{cpu},\"mem\":{mem},\
         \"budget_cpu\":{bcpu},\"budget_mem\":{bmem}}}",
        ep = entrypoint,
        cpu = snap.cpu,
        mem = snap.mem,
        bcpu = cpu_cap,
        bmem = mem_cap,
    );
    assert!(
        !snap.cpu_exceeds(cpu_cap),
        "gas regression: [callora-topics::{ep}] CPU {cpu} exceeds budget {cap}",
        ep = entrypoint,
        cpu = snap.cpu,
        cap = cpu_cap,
    );
    assert!(
        !snap.mem_exceeds(mem_cap),
        "gas regression: [callora-topics::{ep}] mem {mem} exceeds budget {cap}",
        ep = entrypoint,
        mem = snap.mem,
        cap = mem_cap,
    );
}

/// Verify that `after` has not grown by more than `pct`% relative to `before`.
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
    assert_within_budget(entrypoint, after, cpu_cap, mem_cap);
}

// ---------------------------------------------------------------------------
// Measurement macro
// ---------------------------------------------------------------------------

macro_rules! measure_snap {
    ($env:expr, $snap:ident, $body:expr) => {
        $body;
        let $snap = ProfileSnapshot::capture(&$env);
    };
}

// ===========================================================================
// Per-entrypoint gas snapshot tests
// ===========================================================================

/// Snapshot `init` — deploy and initialize the topics contract.
///
/// Budget caps are set at 2× a representative baseline to allow headroom
/// for minor Soroban host updates while still catching regressions.
#[test]
fn gas_snap_init() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(CalloraTopics, ());
    let client = CalloraTopicsClient::new(&env, &contract_id);

    measure_snap!(env, snap, {
        client.init(&admin);
    });
    assert_within_budget("init", snap, 500_000, 10_000);
}

/// Snapshot `register_topic` — admin registers a new topic.
#[test]
fn gas_snap_register_topic() {
    let f = setup();
    let name = Symbol::new(&f.env, "payments");
    let desc = String::from_str(&f.env, "payment events topic");
    let owner = Address::generate(&f.env);

    measure_snap!(f.env, snap, {
        f.client.register_topic(&f.admin, &name, &desc, &owner);
    });
    assert_within_budget("register_topic", snap, 600_000, 12_000);
}

/// Snapshot `deactivate` — admin deactivates an existing topic.
#[test]
fn gas_snap_deactivate() {
    let f = setup();
    let name = Symbol::new(&f.env, "deact_topic");
    register_one(&f, name.clone());

    measure_snap!(f.env, snap, {
        f.client.deactivate(&f.admin, &name);
    });
    assert_within_budget("deactivate", snap, 600_000, 12_000);
}

/// Snapshot `get_topic` — view call to fetch a registered topic record.
///
/// This is a hot read path; the TTL bump is also measured here.
#[test]
fn gas_snap_get_topic() {
    let f = setup();
    let name = Symbol::new(&f.env, "get_topic");
    register_one(&f, name.clone());

    measure_snap!(f.env, snap, {
        let record = f.client.get_topic(&name);
        assert_eq!(record.name, name);
    });
    assert_within_budget("get_topic", snap, 400_000, 8_000);
}

/// Snapshot `is_active` — view call to check topic activation status.
#[test]
fn gas_snap_is_active() {
    let f = setup();
    let name = Symbol::new(&f.env, "is_active");
    register_one(&f, name.clone());

    measure_snap!(f.env, snap, {
        let active = f.client.is_active(&name);
        assert!(active, "freshly registered topic must be active");
    });
    assert_within_budget("is_active", snap, 400_000, 8_000);
}

/// Snapshot `topic_count` — view call to retrieve total registered count.
#[test]
fn gas_snap_topic_count() {
    let f = setup();
    register_one(&f, Symbol::new(&f.env, "count_top"));

    measure_snap!(f.env, snap, {
        let count = f.client.topic_count();
        assert_eq!(count, 1);
    });
    assert_within_budget("topic_count", snap, 300_000, 6_000);
}

/// Snapshot `get_admin` — view call to retrieve the admin address.
#[test]
fn gas_snap_get_admin() {
    let f = setup();

    measure_snap!(f.env, snap, {
        let admin = f.client.get_admin();
        assert_eq!(admin, f.admin);
    });
    assert_within_budget("get_admin", snap, 300_000, 6_000);
}

// ===========================================================================
// Relative growth & invariant tests
// ===========================================================================

/// A second `register_topic` call must not cost more than 50% more than the
/// first, confirming O(1) complexity for individual registrations.
#[test]
fn gas_snap_register_topic_second_call_relative() {
    let f = setup();
    let desc = String::from_str(&f.env, "desc");
    let owner = Address::generate(&f.env);

    measure_snap!(f.env, snap_first, {
        f.client
            .register_topic(&f.admin, &Symbol::new(&f.env, "first_top"), &desc, &owner);
    });

    let owner2 = Address::generate(&f.env);
    measure_snap!(f.env, snap_second, {
        f.client
            .register_topic(&f.admin, &Symbol::new(&f.env, "second_top"), &desc, &owner2);
    });

    assert_no_regression(
        "register_topic_second_vs_first",
        snap_first,
        snap_second,
        50,
    );
}

/// Repeated `get_topic` calls on the same record must not regress relative
/// to the first call (idempotent TTL bump cost).
#[test]
fn gas_snap_get_topic_repeated_relative() {
    let f = setup();
    let name = Symbol::new(&f.env, "rep_get");
    register_one(&f, name.clone());

    measure_snap!(f.env, snap_first, {
        let _ = f.client.get_topic(&name);
    });

    measure_snap!(f.env, snap_second, {
        let _ = f.client.get_topic(&name);
    });

    assert_no_regression(
        "get_topic_repeated_second_vs_first",
        snap_first,
        snap_second,
        50,
    );
}

// ===========================================================================
// Sanity & boundary tests
// ===========================================================================

/// `ProfileSnapshot::capture` must return non-zero metrics after a real call.
#[test]
fn gas_snap_snapshot_nonzero_sanity() {
    let f = setup();
    let name = Symbol::new(&f.env, "sanity_chk");
    register_one(&f, name.clone());

    measure_snap!(f.env, snap, {
        let _ = f.client.is_active(&name);
    });
    assert!(snap.cpu > 0, "CPU metric must be > 0 after a real call");
    assert!(snap.mem > 0, "Memory metric must be > 0 after a real call");
}

/// Boundary: `cpu_exceeds` / `mem_exceeds` are inclusive-boundary checks.
#[test]
fn gas_snap_boundary_comparisons() {
    let snap = ProfileSnapshot { cpu: 100, mem: 200 };
    assert!(!snap.cpu_exceeds(100), "equal to cap must NOT exceed");
    assert!(!snap.mem_exceeds(200), "equal to cap must NOT exceed");
    assert!(snap.cpu_exceeds(99), "one above cap must exceed");
    assert!(snap.mem_exceeds(199), "one above cap must exceed");
}

/// Snapshot `register_topic` with a description at the 256-byte maximum.
///
/// Validates that the worst-case input for description validation fits
/// within a reasonable gas budget and that the call succeeds (i.e. a
/// description of exactly 256 printable ASCII bytes is accepted).
#[test]
fn gas_snap_register_topic_max_description() {
    extern crate std;
    use std::string::String as StdString;

    let f = setup();

    // Build a 256-byte description consisting entirely of visible ASCII 'a'
    // characters — the maximum length accepted by normalize_visible_ascii.
    let max_desc_std: StdString = "a".repeat(256);
    let desc = String::from_str(&f.env, max_desc_std.as_str());
    let owner = Address::generate(&f.env);
    let name = Symbol::new(&f.env, "max_desc");

    measure_snap!(f.env, snap, {
        f.client.register_topic(&f.admin, &name, &desc, &owner);
    });

    // Budget is set slightly above the short-description baseline to account
    // for the extra work of copying and scanning 256 bytes.  A regression
    // here means the description validation path became unexpectedly heavier.
    assert_within_budget("register_topic_max_description", snap, 700_000, 14_000);
}
