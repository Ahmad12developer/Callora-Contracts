//! Timelock state and storage helpers for developer balance migrations and
//! contract upgrades.

use soroban_sdk::{contracttype, Address, BytesN, Env};

use crate::{StorageKey, PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD};

/// Mandatory delay between proposing and executing a balance migration.
pub const DEVELOPER_MIGRATION_TIMELOCK_SECONDS: u64 = 86_400;

/// Mandatory delay between proposing and executing a contract upgrade (48 h).
pub const UPGRADE_TIMELOCK_SECONDS: u64 = 172_800;

/// Proposal bump constants reused from the developer migration pattern.
const PROPOSAL_BUMP_THRESHOLD: u32 = 17_280 * 30; // ~30 days
const PROPOSAL_BUMP_AMOUNT: u32 = 17_280 * 60; // ~60 days

/// Immutable snapshot stored for a pending timelocked WASM upgrade.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingUpgrade {
    /// 32-byte WASM hash proposed for installation.
    pub wasm_hash: BytesN<32>,
    /// Ledger timestamp at which the proposal was recorded.
    pub proposed_at: u64,
    /// Earliest ledger timestamp at which `execute_upgrade` may run.
    pub execute_after: u64,
}

/// Immutable approval snapshot stored for a pending developer migration.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PendingDeveloperMigration {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
    pub proposed_at: u64,
    pub execute_after: u64,
}

/// Read a pending migration and refresh its storage lifetime.
pub(crate) fn get_pending_migration(
    env: &Env,
    from: &Address,
) -> Option<PendingDeveloperMigration> {
    let key = StorageKey::PendingDeveloperMigration(from.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    env.storage().persistent().get(&key)
}

/// Persist a pending migration and refresh its storage lifetime.
pub(crate) fn set_pending_migration(env: &Env, migration: &PendingDeveloperMigration) {
    let key = StorageKey::PendingDeveloperMigration(migration.from.clone());
    env.storage().persistent().set(&key, migration);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
}

/// Consume a successfully executed proposal to make replay impossible.
pub(crate) fn remove_pending_migration(env: &Env, from: &Address) {
    env.storage()
        .persistent()
        .remove(&StorageKey::PendingDeveloperMigration(from.clone()));
}

// ─── PendingUpgrade helpers ──────────────────────────────────────────────────

/// Read the pending upgrade proposal, refreshing its storage lifetime.
pub(crate) fn get_pending_upgrade(env: &Env) -> Option<PendingUpgrade> {
    let key = StorageKey::PendingUpgrade;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, PROPOSAL_BUMP_THRESHOLD, PROPOSAL_BUMP_AMOUNT);
    }
    env.storage().persistent().get(&key)
}

/// Persist a new upgrade proposal and refresh its storage lifetime.
pub(crate) fn set_pending_upgrade(env: &Env, proposal: &PendingUpgrade) {
    let key = StorageKey::PendingUpgrade;
    env.storage().persistent().set(&key, proposal);
    env.storage()
        .persistent()
        .extend_ttl(&key, PROPOSAL_BUMP_THRESHOLD, PROPOSAL_BUMP_AMOUNT);
}

/// Remove a pending upgrade proposal (after execution or cancellation).
pub(crate) fn clear_pending_upgrade(env: &Env) {
    env.storage()
        .persistent()
        .remove(&StorageKey::PendingUpgrade);
}
