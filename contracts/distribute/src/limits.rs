//! # Per-Account State Caps — `limits` module
//!
//! This module enforces per-account state limits for the Callora Distribute
//! contract.  It tracks how many **active state entries** (bets, positions,
//! subscriptions, etc.) each account holds and rejects operations that would
//! exceed a configurable global cap.
//!
//! ## Why caps?
//!
//! Without per-account caps a malicious or buggy client could open an
//! unbounded number of state entries, bloating persistent storage and
//! degrading read performance for every other participant.  Caps keep
//! storage usage predictable and proportional to legitimate usage.
//!
//! ## Storage layout
//!
//! Two counters are maintained for every account:
//!
//! | Key                              | Type | Description                             |
//! |----------------------------------|------|-----------------------------------------|
//! | `AccountCount(Address)`          | u32  | Total active entries across all categories |
//! | `AccountCategoryCount(Addr,Sym)` | u32  | Active entries for a specific category   |
//!
//! Both counters live in **instance storage** so they are cheap to read and
//! write within a single transaction but persist across ledger closes.
//!
//! ## Semantics
//!
//! * **open** — increments both counters; rejects if the global count would
//!   equal or exceed the cap.
//! * **close** — decrements both counters; rejects if the global count is
//!   already zero (underflow guard).
//!
//! All arithmetic is overflow-checked.  No `unwrap()` is used in any
//! production path.

use soroban_sdk::{contracttype, Address, Env, Symbol};

use crate::errors::DistributeError;

/// Default global per-account cap.
///
/// Applied when the admin has not explicitly called `set_global_cap`.
/// Set to a conservative value that still allows legitimate use while
/// capping worst-case storage growth.
pub const DEFAULT_GLOBAL_CAP: u32 = 100;

/// Maximum number of items allowed in a single batch operation.
pub const MAX_BATCH_SIZE: u32 = 50;

/// TTL bump constants for instance storage archival risk mitigation.
/// Soroban archives ledger entries after ~7 days (631 ledgers) of inactivity.
/// Bumping TTL ensures state remains accessible for critical operations.
pub const BUMP_AMOUNT: u32 = 10_000;
pub const LIFETIME_THRESHOLD: u32 = 1_000;

/// Canonical storage keys for the entire Distribute contract.
///
/// All reads and writes to instance storage MUST go through this enum so that
/// the layout remains consistent.  In particular, the pause flag **must** use
/// `StorageKey::Paused` — using a bare `Symbol("paused")` string key would
/// create a second, incompatible entry and break the circuit-breaker.
#[contracttype]
pub enum StorageKey {
    /// Contract admin address.
    Admin,
    /// Pending admin address during a two-step admin transfer.
    PendingAdmin,
    /// Circuit-breaker flag (`true` = paused).
    ///
    /// This is the **single** canonical key for the pause flag.  No other
    /// key (e.g. a bare `Symbol("paused")`) may be used for this purpose.
    Paused,
    /// Contract version marker (WASM hash) set by `upgrade`.
    ContractVersion,
    /// Global per-account cap.
    GlobalCap,
    /// USDC token address configured during `init`.
    Usdc,
    /// Maximum distributable amount per leg.
    MaxDistribute,
    /// Total active entries for an account (all categories combined).
    AccountCount(Address),
    /// Active entries for a specific `(account, category)` pair.
    AccountCategoryCount(Address, Symbol),
}

/// Per-account state record returned by view functions.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    /// Total active entries across all categories.
    pub count: u32,
}

/// Check whether `account` is currently under the global per-account cap.
///
/// Returns `Ok(true)` when `count < cap`, `Ok(false)` when at or above cap,
/// and `Err(AccountLimitExceeded)` if incrementing would violate the cap.
///
/// This is a pure read-only check — no state is mutated.
pub fn check_under_cap(env: &Env, account: &Address, cap: u32) -> Result<bool, DistributeError> {
    let count = get_account_count(env, account);
    if count < cap {
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Increment the active state count for `account` and return the new total.
///
/// # Errors
/// Returns `DistributeError::AccountLimitExceeded` if the current count is
/// already at or above the cap (i.e. `count >= cap`).
///
/// # Overflow safety
/// Uses `checked_add` internally.  Since the cap is a `u32` and counts are
/// `u32`, overflow is impossible when the cap is enforced, but the check is
/// explicit for defense-in-depth.
pub fn increment_state(env: &Env, account: &Address, cap: u32) -> Result<u32, DistributeError> {
    let current = get_account_count(env, account);
    if current >= cap {
        return Err(DistributeError::AccountLimitExceeded);
    }
    let new_count = current.checked_add(1).ok_or(DistributeError::Overflow)?;
    write_account_count(env, account, new_count);
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    Ok(new_count)
}

/// Decrement the active state count for `account` and return the new total.
///
/// # Errors
/// Returns `DistributeError::AccountStateEmpty` if the current count is zero.
///
/// # Overflow safety
/// Uses `checked_sub` to guard against underflow.
pub fn decrement_state(env: &Env, account: &Address) -> Result<u32, DistributeError> {
    let current = get_account_count(env, account);
    if current == 0 {
        return Err(DistributeError::AccountStateEmpty);
    }
    let new_count = current.checked_sub(1).ok_or(DistributeError::Overflow)?;
    write_account_count(env, account, new_count);
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    Ok(new_count)
}

/// Increment the per-category counter for `(account, category)`.
///
/// Returns the new per-category count.  This function does **not** enforce
/// the global cap — the caller (`open`) must do that separately.
///
/// # Overflow safety
/// Uses `checked_add`.  Returns `DistributeError::Overflow` on `u32` overflow.
pub fn increment_category(
    env: &Env,
    account: &Address,
    category: &Symbol,
) -> Result<u32, DistributeError> {
    let current = get_account_category_count(env, account, category);
    let new_count = current.checked_add(1).ok_or(DistributeError::Overflow)?;
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().set(&key, &new_count);
    Ok(new_count)
}

/// Decrement the per-category counter for `(account, category)`.
///
/// Returns the new per-category count.  Does **not** check the global cap.
///
/// # Overflow safety
/// Uses `checked_sub`.  Returns `DistributeError::AccountStateEmpty` on
/// underflow (category count already zero).
pub fn decrement_category(
    env: &Env,
    account: &Address,
    category: &Symbol,
) -> Result<u32, DistributeError> {
    let current = get_account_category_count(env, account, category);
    if current == 0 {
        return Err(DistributeError::AccountStateEmpty);
    }
    let new_count = current.checked_sub(1).ok_or(DistributeError::Overflow)?;
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().set(&key, &new_count);
    Ok(new_count)
}

/// Read the total active entry count for `account` across all categories.
///
/// Returns `0` if the account has no active entries (safe default).
pub fn get_account_count(env: &Env, account: &Address) -> u32 {
    let key = StorageKey::AccountCount(account.clone());
    env.storage().instance().get(&key).unwrap_or(0u32)
}

/// Read the per-category active entry count for `(account, category)`.
///
/// Returns `0` if the account has no active entries in this category.
pub fn get_account_category_count(env: &Env, account: &Address, category: &Symbol) -> u32 {
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().get(&key).unwrap_or(0u32)
}

/// Read the global per-account cap.
///
/// Returns `DEFAULT_GLOBAL_CAP` if the admin has not explicitly set a cap.
pub fn get_global_cap(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::GlobalCap)
        .unwrap_or(DEFAULT_GLOBAL_CAP)
}

/// Write the global per-account cap to instance storage.
pub fn write_global_cap(env: &Env, cap: u32) {
    env.storage().instance().set(&StorageKey::GlobalCap, &cap);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Write the total active entry count for `account` to instance storage.
fn write_account_count(env: &Env, account: &Address, count: u32) {
    let key = StorageKey::AccountCount(account.clone());
    env.storage().instance().set(&key, &count);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_global_cap_is_reasonable() {
        const { assert!(DEFAULT_GLOBAL_CAP > 0) };
    }

    #[test]
    fn max_batch_size_is_reasonable() {
        const {
            assert!(MAX_BATCH_SIZE > 0);
            assert!(MAX_BATCH_SIZE <= 100);
        }
    }

    #[test]
    fn bump_constants_are_sensible() {
        const { assert!(BUMP_AMOUNT > LIFETIME_THRESHOLD) };
    }
}
