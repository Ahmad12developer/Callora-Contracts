//! Sweep/pause/upgrade lifecycle against the global admin cool-off (#1125).
//!
//! `execute_pause`, `execute_upgrade` and `execute_sweep` all call
//! [`admin::guard`], which records `LastCriticalAdminAction` and blocks any
//! other critical execution until the cool-off has elapsed. That guard is the
//! only thing stopping an attacker holding the admin key from chaining
//! pause + sweep + upgrade in a single block once several proposals have
//! matured, so these tests pin it from the public contract surface:
//!
//! * two matured proposals executed back-to-back — the second fails with
//!   `AdminCooldownActive` and changes nothing, in either order;
//! * the upgrade path hits the same guard;
//! * the second execution succeeds exactly at the cool-off boundary;
//! * `get_last_critical_admin_action` reports the right action symbol/time;
//! * a failed execution (and a no-op execution) never arms the cool-off;
//! * `cancel_sweep` is idempotent and its `existing.is_some()` payload is
//!   `false` when nothing is pending;
//! * cancel → re-propose restarts the timelock and still respects the
//!   cool-off.
//!
//! Run with `cargo test -p callora-vault timelock`.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{token, Address, BytesN, Env, IntoVal, Symbol};

use super::*;

const T0: u64 = 1_700_000_000;
/// Shortest allowed timelock, so proposals mature quickly.
const TIMELOCK: u64 = timelock::MIN_TIMELOCK_SECONDS;
/// Cool-off shorter than the timelock unless a test says otherwise.
const COOLDOWN: u64 = 600;
const VAULT_USDC: i128 = 1_000;
const SWEEP: i128 = 100;

struct Ctx<'a> {
    env: &'a Env,
    client: CalloraVaultClient<'a>,
    vault: Address,
    admin: Address,
    recipient: Address,
    usdc: token::Client<'a>,
}

/// Initialised vault (owner is admin), funded on-ledger, short timelock and
/// a 600 s admin cool-off.
fn setup(env: &Env) -> Ctx<'_> {
    env.ledger().set_timestamp(T0);
    env.mock_all_auths();

    let admin = Address::generate(env);
    let recipient = Address::generate(env);
    let vault = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let usdc = token::Client::new(env, &usdc_addr);

    client.init(
        &admin,
        &usdc_addr,
        &None,
        &None,
        &Some(1i128),
        &None,
        &None,
        &None,
    );
    token::StellarAssetClient::new(env, &usdc_addr).mint(&vault, &VAULT_USDC);
    client.set_timelock_window(&admin, &TIMELOCK);
    client.set_admin_cooldown(&admin, &COOLDOWN);

    Ctx {
        env,
        client,
        vault,
        admin,
        recipient,
        usdc,
    }
}

fn at(ctx: &Ctx, ts: u64) {
    ctx.env.ledger().set_timestamp(ts);
}

fn last_action(ctx: &Ctx) -> Option<(Symbol, u64)> {
    ctx.client
        .get_last_critical_admin_action()
        .map(|a| (a.action, a.executed_at))
}

fn sym(ctx: &Ctx, s: &str) -> Symbol {
    Symbol::new(ctx.env, s)
}

/// Data payload of the most recent event whose first topic is `topic`.
fn last_event_bool(ctx: &Ctx, topic: Symbol) -> bool {
    let events = ctx.env.events().all();
    let (_, topics, data) = events
        .iter()
        .rev()
        .find(|(_, topics, _)| {
            let t0: Symbol = topics.get(0).unwrap().into_val(ctx.env);
            t0 == topic
        })
        .expect("event emitted");
    assert!(!topics.is_empty());
    data.into_val(ctx.env)
}

// ── back-to-back executions ─────────────────────────────────────────────────

#[test]
fn timelock_cooldown_sweep_then_pause_blocked_until_boundary() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.propose_sweep(&ctx.admin, &ctx.recipient, &SWEEP);
    ctx.client.propose_pause(&ctx.admin);

    let matured = T0 + TIMELOCK;
    at(&ctx, matured);
    ctx.client.execute_sweep(&ctx.admin);
    assert_eq!(last_action(&ctx), Some((sym(&ctx, "sweep"), matured)));
    assert_eq!(ctx.usdc.balance(&ctx.recipient), SWEEP);

    // Second matured proposal in the same block: blocked, nothing changes.
    assert_eq!(
        ctx.client.try_execute_pause(&ctx.admin),
        Err(Ok(VaultError::AdminCooldownActive))
    );
    assert!(!ctx.client.is_paused());
    assert!(
        ctx.client.get_pending_pause().is_some(),
        "proposal kept for later"
    );
    assert_eq!(last_action(&ctx), Some((sym(&ctx, "sweep"), matured)));

    // One second before the boundary: still blocked.
    at(&ctx, matured + COOLDOWN - 1);
    assert_eq!(ctx.client.admin_cooldown_remaining(), 1);
    assert!(!ctx.client.is_admin_action_ready());
    assert_eq!(
        ctx.client.try_execute_pause(&ctx.admin),
        Err(Ok(VaultError::AdminCooldownActive))
    );

    // Exactly at the boundary: succeeds and re-arms with the new action.
    at(&ctx, matured + COOLDOWN);
    assert!(ctx.client.is_admin_action_ready());
    ctx.client.execute_pause(&ctx.admin);
    assert!(ctx.client.is_paused());
    assert_eq!(
        last_action(&ctx),
        Some((sym(&ctx, "pause"), matured + COOLDOWN))
    );
    assert_eq!(ctx.client.admin_cooldown_remaining(), COOLDOWN);
}

#[test]
fn timelock_cooldown_pause_then_sweep_moves_no_funds_until_boundary() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.propose_pause(&ctx.admin);
    ctx.client.propose_sweep(&ctx.admin, &ctx.recipient, &SWEEP);

    let matured = T0 + TIMELOCK;
    at(&ctx, matured);
    ctx.client.execute_pause(&ctx.admin);
    assert_eq!(last_action(&ctx), Some((sym(&ctx, "pause"), matured)));

    assert_eq!(
        ctx.client.try_execute_sweep(&ctx.admin),
        Err(Ok(VaultError::AdminCooldownActive))
    );
    // The blocked sweep transferred nothing and kept its proposal.
    assert_eq!(ctx.usdc.balance(&ctx.recipient), 0);
    assert_eq!(ctx.usdc.balance(&ctx.vault), VAULT_USDC);
    assert!(ctx.client.get_pending_sweep().is_some());

    at(&ctx, matured + COOLDOWN);
    ctx.client.execute_sweep(&ctx.admin);
    assert_eq!(ctx.usdc.balance(&ctx.recipient), SWEEP);
    assert_eq!(ctx.usdc.balance(&ctx.vault), VAULT_USDC - SWEEP);
    assert!(ctx.client.get_pending_sweep().is_none());
    assert_eq!(
        last_action(&ctx),
        Some((sym(&ctx, "sweep"), matured + COOLDOWN))
    );
}

#[test]
fn timelock_cooldown_upgrade_path_is_guarded_too() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.propose_pause(&ctx.admin);
    ctx.client
        .propose_upgrade(&ctx.admin, &BytesN::from_array(ctx.env, &[7u8; 32]));

    at(&ctx, T0 + TIMELOCK);
    ctx.client.execute_pause(&ctx.admin);
    // The guard runs before the WASM swap, so the matured upgrade is refused.
    assert_eq!(
        ctx.client.try_execute_upgrade(&ctx.admin),
        Err(Ok(VaultError::AdminCooldownActive))
    );
    assert_eq!(last_action(&ctx).map(|(a, _)| a), Some(sym(&ctx, "pause")));
}

// ── what does NOT arm the cool-off ──────────────────────────────────────────

#[test]
fn timelock_cooldown_failed_execution_does_not_arm_cooldown() {
    let env = Env::default();
    let ctx = setup(&env);
    // More than the vault holds: validation fails before the guard.
    ctx.client
        .propose_sweep(&ctx.admin, &ctx.recipient, &(VAULT_USDC + 1));
    ctx.client.propose_pause(&ctx.admin);
    at(&ctx, T0 + TIMELOCK);

    assert_eq!(
        ctx.client.try_execute_sweep(&ctx.admin),
        Err(Ok(VaultError::InsufficientBalance))
    );
    assert_eq!(last_action(&ctx), None);
    assert!(ctx.client.is_admin_action_ready());

    // So another critical action can still run immediately.
    ctx.client.execute_pause(&ctx.admin);
    assert_eq!(last_action(&ctx).map(|(a, _)| a), Some(sym(&ctx, "pause")));
}

#[test]
fn timelock_cooldown_execute_pause_on_already_paused_vault_does_not_arm_cooldown() {
    let env = Env::default();
    let ctx = setup(&env);
    ctx.client.propose_pause(&ctx.admin);
    ctx.client.pause(&ctx.admin); // direct owner pause
    at(&ctx, T0 + TIMELOCK);

    ctx.client.execute_pause(&ctx.admin); // idempotent no-op path
    assert!(ctx.client.get_pending_pause().is_none());
    assert_eq!(last_action(&ctx), None);
}

// ── cancel_sweep idempotency & re-propose ───────────────────────────────────

#[test]
fn timelock_cooldown_cancel_sweep_is_idempotent_with_is_some_payload() {
    let env = Env::default();
    let ctx = setup(&env);
    let cancelled = events::event_sweep_cancelled(ctx.env);

    // Nothing pending: Ok, payload false.
    ctx.client.cancel_sweep(&ctx.admin);
    assert!(!last_event_bool(&ctx, cancelled.clone()));

    // Pending: payload true, proposal cleared.
    ctx.client.propose_sweep(&ctx.admin, &ctx.recipient, &SWEEP);
    ctx.client.cancel_sweep(&ctx.admin);
    assert!(last_event_bool(&ctx, cancelled.clone()));
    assert!(ctx.client.get_pending_sweep().is_none());

    // Repeating is harmless and reports false again.
    ctx.client.cancel_sweep(&ctx.admin);
    assert!(!last_event_bool(&ctx, cancelled));

    // Cancelling never arms the cool-off, and there is nothing to execute.
    assert_eq!(last_action(&ctx), None);
    at(&ctx, T0 + TIMELOCK);
    assert_eq!(
        ctx.client.try_execute_sweep(&ctx.admin),
        Err(Ok(VaultError::ProposalNotFound))
    );
}

#[test]
fn timelock_cooldown_cancel_and_repropose_restarts_timelock_and_respects_cooldown() {
    let env = Env::default();
    let ctx = setup(&env);
    // Cool-off longer than the timelock so the two windows can be told apart.
    let cooldown = TIMELOCK * 2;
    ctx.client.set_admin_cooldown(&ctx.admin, &cooldown);

    ctx.client.propose_pause(&ctx.admin);
    let t1 = T0 + TIMELOCK;
    at(&ctx, t1);
    ctx.client.execute_pause(&ctx.admin);

    ctx.client.propose_sweep(&ctx.admin, &ctx.recipient, &SWEEP);
    ctx.client.cancel_sweep(&ctx.admin);
    at(&ctx, t1 + 100);
    ctx.client.propose_sweep(&ctx.admin, &ctx.recipient, &SWEEP);
    assert_eq!(
        ctx.client.get_pending_sweep().unwrap().execute_after,
        t1 + 100 + TIMELOCK
    );

    // The first proposal's deadline no longer counts.
    at(&ctx, t1 + TIMELOCK);
    assert_eq!(
        ctx.client.try_execute_sweep(&ctx.admin),
        Err(Ok(VaultError::TimelockNotExpired))
    );

    // Re-proposal matured, but the cool-off from the pause is still active.
    at(&ctx, t1 + 100 + TIMELOCK);
    assert_eq!(
        ctx.client.try_execute_sweep(&ctx.admin),
        Err(Ok(VaultError::AdminCooldownActive))
    );

    at(&ctx, t1 + cooldown);
    ctx.client.execute_sweep(&ctx.admin);
    assert_eq!(ctx.usdc.balance(&ctx.recipient), SWEEP);
    assert_eq!(last_action(&ctx), Some((sym(&ctx, "sweep"), t1 + cooldown)));
}
