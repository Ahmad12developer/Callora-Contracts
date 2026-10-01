//! # Vault Escape-Hatch Timelock Module (Issue #482)
//!
//! Guards three critical admin actions (`pause`, `upgrade`, `sweep`) with a
//! mandatory delay between **proposal** and **execution**.
//!
//! Each action is independently gated by its own persistent-storage proposal
//! slot, so multiple orthogonal proposals can be staged concurrently without
//! overwriting each other.
//!
//! ## Flow
//! 1. **Propose** (`propose_pause`, `propose_upgrade`, `propose_sweep`) — admin
//!    snapshots the intended action and records `proposed_at` + `execute_after`.
//! 2. **Wait** — at least `TimelockWindow` seconds elapse on the ledger clock.
//! 3. **Execute** (`execute_pause`, …) — admin invokes the action after the
//!    window expires. The pending proposal is consumed atomically.
//! 4. **Cancel** (`cancel_pause`, …) — admin may abort before execution.
//!
//! ## Window Configuration
//! `TimelockWindow` is stored in instance storage and may be changed by the
//! admin via [`set_timelock_window`]. The default is **48 hours**. Valid
//! values are bounded between **1 hour** and **30 days** to prevent
//! misconfiguration (too short → no protection, too long → stuck funds).
//!
//! ## Storage Layout
//! - `PendingPause`         (persistent)
//! - `PendingUpgrade`       (persistent, holds `BytesN<32>` wasm hash)
//! - `PendingSweep`         (persistent, holds destination + amount)
//! - `TimelockWindow`       (instance, `u64` seconds)

use soroban_sdk::{contracttype, Address, BytesN, Env, Map, Symbol, TryFromVal, Val};

/// Default timelock delay — 48 hours, as required by issue #482.
pub const DEFAULT_TIMELOCK_SECONDS: u64 = 172_800;

/// Lower bound: 1 hour. Anything shorter would defeat the purpose.
pub const MIN_TIMELOCK_SECONDS: u64 = 3_600;

/// Upper bound: 30 days. Larger windows risk long-stuck emergency actions.
pub const MAX_TIMELOCK_SECONDS: u64 = 2_592_000;

/// Grace period during which an unlocked proposal may be executed (7 days).
/// After `execute_after + PROPOSAL_GRACE_SECONDS`, the proposal expires.
pub const PROPOSAL_GRACE_SECONDS: u64 = 604_800;

/// Alias for [`PROPOSAL_GRACE_SECONDS`].
pub const TIMELOCK_GRACE_SECONDS: u64 = PROPOSAL_GRACE_SECONDS;

/// Threshold & bump amounts for persistent TTL — 30 / 60 days for most keys.
pub const PROPOSAL_BUMP_THRESHOLD: u32 = 17_280 * 30; // ~30 days
pub const PROPOSAL_BUMP_AMOUNT: u32 = 17_280 * 60; // ~60 days

/// Pending proposal: pause the vault.
///
/// Wall-clock fields are stored explicitly so off-chain indexers can calculate
/// the remaining wait without re-deriving from the ledger.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingPause {
    pub proposed_at: u64,
    pub execute_after: u64,
    pub expires_at: u64,
}

/// Pending proposal: upgrade the vault WASM.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingUpgrade {
    pub wasm_hash: BytesN<32>,
    pub proposed_at: u64,
    pub execute_after: u64,
    pub expires_at: u64,
}

/// Pending proposal: sweep (distribute) on-ledger surplus to a recipient.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingSweep {
    pub to: Address,
    pub amount: i128,
    pub proposed_at: u64,
    pub execute_after: u64,
    pub expires_at: u64,
}

/// Legacy representation of `PendingPause` without `expires_at`, used for backward
/// compatibility during migration of pre-existing proposals.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyPendingPause {
    pub proposed_at: u64,
    pub execute_after: u64,
}

/// Legacy representation of `PendingUpgrade` without `expires_at`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyPendingUpgrade {
    pub wasm_hash: BytesN<32>,
    pub proposed_at: u64,
    pub execute_after: u64,
}

/// Legacy representation of `PendingSweep` without `expires_at`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyPendingSweep {
    pub to: Address,
    pub amount: i128,
    pub proposed_at: u64,
    pub execute_after: u64,
}

/// Read the configured window length, defaulting to
/// [`DEFAULT_TIMELOCK_SECONDS`] if it has never been set.
pub(crate) fn get_timelock_window(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&crate::StorageKey::TimelockWindow)
        .unwrap_or(DEFAULT_TIMELOCK_SECONDS)
}

/// Persist a new window length. Caller is responsible for bounds checks and
/// error reporting via typed [`crate::VaultError`] codes.
pub(crate) fn set_timelock_window(env: &Env, window: u64) {
    env.storage()
        .instance()
        .set(&crate::StorageKey::TimelockWindow, &window);
}

/// Compute `proposed_at + window`, returning false on overflow (timestamp +
/// window exceeded `u64::MAX`). Callers should panic with
/// `VaultError::TimelockOverflow` when this returns `false`.
pub(crate) fn saturating_deadline(proposed_at: u64, window: u64) -> Option<u64> {
    if window == 0 {
        return Some(proposed_at);
    }
    let delta = u64::MAX.checked_sub(proposed_at)?;
    if window > delta {
        return None;
    }
    proposed_at.checked_add(window)
}

// --- PendingPause helpers --------------------------------------------------

pub(crate) fn get_pending_pause(env: &Env) -> Option<PendingPause> {
    let key = crate::StorageKey::PendingPause;
    let raw: Val = env.storage().persistent().get(&key)?;
    let map = Map::<Symbol, Val>::try_from_val(env, &raw).ok()?;
    if map.contains_key(Symbol::new(env, "expires_at")) {
        PendingPause::try_from_val(env, &raw).ok()
    } else {
        let legacy = LegacyPendingPause::try_from_val(env, &raw).ok()?;
        let expires_at = saturating_deadline(legacy.execute_after, PROPOSAL_GRACE_SECONDS)
            .unwrap_or(u64::MAX);
        let proposal = PendingPause {
            proposed_at: legacy.proposed_at,
            execute_after: legacy.execute_after,
            expires_at,
        };
        set_pending_pause(env, &proposal);
        Some(proposal)
    }
}

pub(crate) fn set_pending_pause(env: &Env, proposal: &PendingPause) {
    let key = crate::StorageKey::PendingPause;
    env.storage().persistent().set(&key, proposal);
    env.storage()
        .persistent()
        .extend_ttl(&key, PROPOSAL_BUMP_THRESHOLD, PROPOSAL_BUMP_AMOUNT);
}

pub(crate) fn clear_pending_pause(env: &Env) {
    env.storage()
        .persistent()
        .remove(&crate::StorageKey::PendingPause);
}

// --- PendingUpgrade helpers ------------------------------------------------

pub(crate) fn get_pending_upgrade(env: &Env) -> Option<PendingUpgrade> {
    let key = crate::StorageKey::PendingUpgrade;
    let raw: Val = env.storage().persistent().get(&key)?;
    let map = Map::<Symbol, Val>::try_from_val(env, &raw).ok()?;
    if map.contains_key(Symbol::new(env, "expires_at")) {
        PendingUpgrade::try_from_val(env, &raw).ok()
    } else {
        let legacy = LegacyPendingUpgrade::try_from_val(env, &raw).ok()?;
        let expires_at = saturating_deadline(legacy.execute_after, PROPOSAL_GRACE_SECONDS)
            .unwrap_or(u64::MAX);
        let proposal = PendingUpgrade {
            wasm_hash: legacy.wasm_hash,
            proposed_at: legacy.proposed_at,
            execute_after: legacy.execute_after,
            expires_at,
        };
        set_pending_upgrade(env, &proposal);
        Some(proposal)
    }
}

pub(crate) fn set_pending_upgrade(env: &Env, proposal: &PendingUpgrade) {
    let key = crate::StorageKey::PendingUpgrade;
    env.storage().persistent().set(&key, proposal);
    env.storage()
        .persistent()
        .extend_ttl(&key, PROPOSAL_BUMP_THRESHOLD, PROPOSAL_BUMP_AMOUNT);
}

pub(crate) fn clear_pending_upgrade(env: &Env) {
    env.storage()
        .persistent()
        .remove(&crate::StorageKey::PendingUpgrade);
}

// --- PendingSweep helpers --------------------------------------------------

pub(crate) fn get_pending_sweep(env: &Env) -> Option<PendingSweep> {
    let key = crate::StorageKey::PendingSweep;
    let raw: Val = env.storage().persistent().get(&key)?;
    let map = Map::<Symbol, Val>::try_from_val(env, &raw).ok()?;
    if map.contains_key(Symbol::new(env, "expires_at")) {
        PendingSweep::try_from_val(env, &raw).ok()
    } else {
        let legacy = LegacyPendingSweep::try_from_val(env, &raw).ok()?;
        let expires_at = saturating_deadline(legacy.execute_after, PROPOSAL_GRACE_SECONDS)
            .unwrap_or(u64::MAX);
        let proposal = PendingSweep {
            to: legacy.to,
            amount: legacy.amount,
            proposed_at: legacy.proposed_at,
            execute_after: legacy.execute_after,
            expires_at,
        };
        set_pending_sweep(env, &proposal);
        Some(proposal)
    }
}

pub(crate) fn set_pending_sweep(env: &Env, proposal: &PendingSweep) {
    let key = crate::StorageKey::PendingSweep;
    env.storage().persistent().set(&key, proposal);
    env.storage()
        .persistent()
        .extend_ttl(&key, PROPOSAL_BUMP_THRESHOLD, PROPOSAL_BUMP_AMOUNT);
}

pub(crate) fn clear_pending_sweep(env: &Env) {
    env.storage()
        .persistent()
        .remove(&crate::StorageKey::PendingSweep);
}
