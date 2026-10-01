//! Cross-contract call safety tests for the `callora-upgrade` crate.
//!
//! Verifies that upgrade actions under callee failure (reverts, panics, and
//! contract-level errors) behave correctly with respect to storage rollback
//! and cooldown state, when exercised through a real Soroban cross-contract
//! call boundary.
//!
//! ## Coverage
//!
//! | Test                                                     | Scenario |
//! |----------------------------------------------------------|----------|
//! | `test_xcontract_check_and_record_upgrade_success`        | Happy-path: upgrade timestamp recorded |
//! | `test_xcontract_callee_revert_preserves_upgrade`         | `CooldownNotElapsed` error leaves storage unchanged |
//! | `test_xcontract_callee_overflow_revert_preserves`        | `Overflow` error leaves storage unchanged |
//! | `test_xcontract_callee_panic_rolls_back`                 | Callee panic triggers full Soroban rollback |
//! | `test_xcontract_set_cooldown_survives_xcontract_call`    | Cooldown value is durable across cross-contract calls |
//! | `test_xcontract_cooldown_resets_after_rollback`          | After a panic-rollback a fresh upgrade succeeds |
//! | `test_xcontract_repeated_upgrade_after_cooldown`         | Two sequential upgrades both persist correctly |
//!
//! Closes CalloraOrg/Callora-Contracts#899.

#![cfg(test)]

extern crate std;

use callora_upgrade::admin::{self, DEFAULT_COOLDOWN_SECONDS, MIN_COOLDOWN_SECONDS};
use callora_upgrade::errors::UpgradeError;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

// ===========================================================================
// Mock contract - UpgradeHarness
// ===========================================================================

/// Thin harness contract that delegates every call to the `admin` module so
/// the module's functions are exercised through the real Soroban XCF layer.
#[contract]
pub struct UpgradeHarness;

#[contractimpl]
impl UpgradeHarness {
    /// Delegate `init_admin` to the admin module.
    pub fn init_admin(env: Env, admin: Address) -> Result<(), UpgradeError> {
        admin::init_admin(&env, &admin)
    }

    /// Delegate `set_cooldown` to the admin module.
    pub fn set_cooldown(env: Env, caller: Address, cooldown: u64) -> Result<(), UpgradeError> {
        admin::set_cooldown(&env, &caller, cooldown)
    }

    /// Delegate `get_cooldown` to the admin module.
    pub fn get_cooldown(env: Env) -> u64 {
        admin::get_cooldown(&env)
    }

    /// Delegate `check_and_record_upgrade` to the admin module.
    pub fn check_and_record_upgrade(env: Env, caller: Address) -> Result<(), UpgradeError> {
        admin::check_and_record_upgrade(&env, &caller)
    }

    /// Record an upgrade then unconditionally panic, simulating a callee that
    /// crashes after the storage write has occurred.  Soroban's transaction
    /// semantics must roll back the storage mutation.
    ///
    /// NOTE: name is ≤ 32 chars as required by the Soroban host.
    pub fn upgrade_then_panic(env: Env, caller: Address) -> Result<(), UpgradeError> {
        admin::check_and_record_upgrade(&env, &caller)?;
        panic!("callee panicked after recording upgrade");
    }

    /// Read back the raw `last_upg_tm` timestamp from instance storage.
    pub fn get_last_upgrade_time(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "last_upg_tm"))
            .unwrap_or(0)
    }
}

// ===========================================================================
// Mock contract - UpgradeCaller
// ===========================================================================

/// Cross-contract caller that proxies every `UpgradeHarness` call.  Using a
/// separate contract ensures calls run through Soroban's real XCF layer.
#[contract]
pub struct UpgradeCaller;

#[contractimpl]
impl UpgradeCaller {
    /// Call `set_cooldown` on `upgrade` via cross-contract invocation.
    pub fn call_set_cooldown(env: Env, upgrade: Address, caller: Address, cooldown: u64) {
        let client = UpgradeHarnessClient::new(&env, &upgrade);
        client.set_cooldown(&caller, &cooldown);
    }

    /// Call `check_and_record_upgrade` on `upgrade` and propagate the result.
    pub fn call_check_upgrade(
        env: Env,
        upgrade: Address,
        caller: Address,
    ) -> Result<(), UpgradeError> {
        let client = UpgradeHarnessClient::new(&env, &upgrade);
        match client.try_check_and_record_upgrade(&caller) {
            Ok(Ok(())) => Ok(()),
            Err(Ok(e)) => Err(e),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    /// Call `upgrade_then_panic` on `upgrade`.  Always panics after the
    /// storage write to trigger Soroban rollback.
    pub fn call_upgrade_then_panic(env: Env, upgrade: Address, caller: Address) {
        let client = UpgradeHarnessClient::new(&env, &upgrade);
        client.upgrade_then_panic(&caller);
    }

    /// Read `get_cooldown` via cross-contract call.
    pub fn call_get_cooldown(env: Env, upgrade: Address) -> u64 {
        let client = UpgradeHarnessClient::new(&env, &upgrade);
        client.get_cooldown()
    }

    /// Read `get_last_upgrade_time` via cross-contract call.
    pub fn call_get_last_upgrade_time(env: Env, upgrade: Address) -> u64 {
        let client = UpgradeHarnessClient::new(&env, &upgrade);
        client.get_last_upgrade_time()
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

struct TestContext {
    env: Env,
    admin: Address,
    upgrade_id: Address,
    upgrade_client: UpgradeHarnessClient<'static>,
    caller_client: UpgradeCallerClient<'static>,
}

fn setup() -> TestContext {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let upgrade_id = env.register(UpgradeHarness, ());
    let upgrade_client = UpgradeHarnessClient::new(&env, &upgrade_id);

    let caller_id = env.register(UpgradeCaller, ());
    let caller_client = UpgradeCallerClient::new(&env, &caller_id);

    TestContext {
        env,
        admin,
        upgrade_id,
        upgrade_client,
        caller_client,
    }
}

// ===========================================================================
// Cross-contract success tests
// ===========================================================================

/// Happy path: upgrade is recorded across the cross-contract boundary.
/// The `last_upg_tm` timestamp must reflect the ledger timestamp at call time.
#[test]
fn test_xcontract_check_and_record_upgrade_success() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(100);

    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);

    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        100,
        "last_upg_tm must equal the ledger timestamp at call time"
    );
}

// ===========================================================================
// Callee failure tests — storage must not change on error
// ===========================================================================

/// Callee returns `CooldownNotElapsed` — the storage timestamp must remain
/// unchanged from the first successful upgrade.
#[test]
fn test_xcontract_callee_revert_preserves_upgrade() {
    let ctx = setup();

    // First successful upgrade.
    ctx.env.ledger().set_timestamp(100);
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);

    // One second before cooldown expires — must be rejected.
    ctx.env
        .ledger()
        .set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS - 1);
    let result = ctx
        .caller_client
        .try_call_check_upgrade(&ctx.upgrade_id, &ctx.admin);
    match result {
        Err(Ok(UpgradeError::CooldownNotElapsed)) => {}
        other => panic!("expected CooldownNotElapsed, got {other:?}"),
    }

    // Storage must still reflect the first successful upgrade timestamp.
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        100,
        "last_upg_tm must remain at first upgrade timestamp after CooldownNotElapsed"
    );

    // After the full cooldown elapses, the next upgrade is allowed.
    ctx.env
        .ledger()
        .set_timestamp(100 + DEFAULT_COOLDOWN_SECONDS);
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        100 + DEFAULT_COOLDOWN_SECONDS,
        "last_upg_tm must advance after a successful second upgrade"
    );
}

/// Callee returns `Overflow` (ledger clock went backwards) — storage must
/// remain unchanged from the first successful upgrade.
#[test]
fn test_xcontract_callee_overflow_revert_preserves() {
    let ctx = setup();

    // First successful upgrade.
    ctx.env.ledger().set_timestamp(200);
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);

    // Decrease the timestamp to force overflow in elapsed calculation.
    ctx.env.ledger().set_timestamp(199);
    let result = ctx
        .caller_client
        .try_call_check_upgrade(&ctx.upgrade_id, &ctx.admin);
    match result {
        Err(Ok(UpgradeError::Overflow)) => {}
        other => panic!("expected Overflow, got {other:?}"),
    }

    // Storage must still reflect the first successful upgrade timestamp.
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        200,
        "last_upg_tm must remain unchanged after Overflow error"
    );
}

/// Callee panics after writing the upgrade timestamp — Soroban must roll back
/// the storage mutation so the timestamp is unchanged.
#[test]
fn test_xcontract_callee_panic_rolls_back() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(500);

    let result = ctx
        .caller_client
        .try_call_upgrade_then_panic(&ctx.upgrade_id, &ctx.admin);
    assert!(result.is_err(), "expected panic/error from upgrade callee");

    // Storage must be as-if the call never happened.
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        0,
        "last_upg_tm must be 0 (unset) after callee panic rollback"
    );
}

// ===========================================================================
// Cooldown and state durability tests
// ===========================================================================

/// `set_cooldown` called via cross-contract is visible to the harness
/// contract's direct client as well (durable instance storage).
#[test]
fn test_xcontract_set_cooldown_survives_xcontract_call() {
    let ctx = setup();

    // set_cooldown is admin-only, so install the admin first.
    ctx.upgrade_client.init_admin(&ctx.admin);

    ctx.caller_client
        .call_set_cooldown(&ctx.upgrade_id, &ctx.admin, &MIN_COOLDOWN_SECONDS);

    assert_eq!(
        ctx.caller_client.call_get_cooldown(&ctx.upgrade_id),
        MIN_COOLDOWN_SECONDS,
        "cooldown set via cross-contract must be visible through caller proxy"
    );
    assert_eq!(
        ctx.upgrade_client.get_cooldown(),
        MIN_COOLDOWN_SECONDS,
        "cooldown set via cross-contract must be visible through direct client"
    );
}

/// After a panic-rollback, a subsequent upgrade at the same timestamp succeeds
/// because no timestamp was persisted.
#[test]
fn test_xcontract_cooldown_resets_after_rollback() {
    let ctx = setup();
    ctx.env.ledger().set_timestamp(500);

    // First call panics and rolls back.
    let _ = ctx
        .caller_client
        .try_call_upgrade_then_panic(&ctx.upgrade_id, &ctx.admin);

    // Immediately retry at the same timestamp — should succeed since storage
    // was rolled back to the "never upgraded" state.
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);

    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        500,
        "upgrade must succeed after rollback at the same timestamp"
    );
}

/// Two sequential upgrades separated by the full cooldown window both persist
/// their respective timestamps.
#[test]
fn test_xcontract_repeated_upgrade_after_cooldown() {
    let ctx = setup();

    ctx.env.ledger().set_timestamp(1_000);
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        1_000
    );

    ctx.env
        .ledger()
        .set_timestamp(1_000 + DEFAULT_COOLDOWN_SECONDS);
    ctx.caller_client
        .call_check_upgrade(&ctx.upgrade_id, &ctx.admin);
    assert_eq!(
        ctx.caller_client
            .call_get_last_upgrade_time(&ctx.upgrade_id),
        1_000 + DEFAULT_COOLDOWN_SECONDS,
        "second upgrade after full cooldown must update last_upg_tm"
    );
}
