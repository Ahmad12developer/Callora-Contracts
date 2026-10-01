//! Admin cooldown enforcement for the Callora registry.
//!
//! Implements a per-developer cool-off window between registrations to prevent
//! rapid abuse by the same developer while allowing different developers to
//! register concurrently. Every admin-gated entrypoint in
//! [`crate::CalloraRegistry`] must call [`require_cooldown`] before
//! mutating state and [`update_cooldown`] after a successful mutation.
//!
//! # Storage
//!
//! The last-registration timestamp for each developer is stored in persistent
//! storage under [`crate::StorageKey::DeveloperCooldown`]`(developer)`.
//! There is exactly one authoritative key per developer; the former global
//! `Symbol`-based key has been removed.

use soroban_sdk::{Address, Env};

use crate::{RegistryError, StorageKey};

/// Cooldown window in seconds between registrations by the same developer.
///
/// Set to 3 600 (1 hour): the same developer must wait at least this long
/// after one registration before performing another one. Different developers
/// are unaffected by each other's cooldown windows.
pub const COOLDOWN_SECONDS: u64 = 3_600;

/// Return the ledger timestamp of the last registration by `developer`, or
/// `None` if this developer has not yet registered anything.
pub fn last_developer_action(env: &Env, developer: &Address) -> Option<u64> {
    env.storage()
        .persistent()
        .get(&StorageKey::DeveloperCooldown(developer.clone()))
}

/// Assert that the cooldown window has elapsed since `developer`'s last
/// registration.
///
/// Returns `Err(RegistryError::AdminCooldownActive)` if the cooldown has not
/// expired for this developer. This is a no-op when the developer has no prior
/// registration, so every developer's first action always succeeds.
pub fn require_cooldown(env: &Env, developer: &Address) -> Result<(), RegistryError> {
    if let Some(last) = last_developer_action(env, developer) {
        let now = env.ledger().timestamp();
        let elapsed = now.saturating_sub(last);
        if elapsed < COOLDOWN_SECONDS {
            return Err(RegistryError::AdminCooldownActive);
        }
    }
    Ok(())
}

/// Record the current ledger timestamp as `developer`'s last registration.
///
/// Must be called after every successful registration so that subsequent
/// registrations by the same developer are subject to the cooldown window.
/// Registrations by other developers are not affected.
pub fn update_cooldown(env: &Env, developer: &Address) {
    let now = env.ledger().timestamp();
    env.storage()
        .persistent()
        .set(&StorageKey::DeveloperCooldown(developer.clone()), &now);
}
