//! Administrative pause control for the Callora Distribute contract.
//!
//! This module owns the **single** canonical pause storage key
//! ([`StorageKey::Paused`](crate::limits::StorageKey::Paused)) and provides
//! the three primitives the rest of the contract uses:
//!
//! | Function              | Purpose                                              |
//! |-----------------------|------------------------------------------------------|
//! | [`is_paused`]         | Read the pause flag (no auth required)               |
//! | [`set_paused`]        | Write the pause flag (auth + admin check in caller)  |
//! | [`require_not_paused`]| Guard for state-changing entrypoints                 |
//!
//! # Storage key invariant
//!
//! All reads and writes go through `StorageKey::Paused`.  No other key (e.g. a
//! bare `Symbol("paused")` string) may be used for the pause flag.  The
//! [`CalloraDistribute::init`](crate::CalloraDistribute::init) function seeds
//! the flag as `false` using this same key, so the layout is consistent from
//! the very first ledger entry.
//!
//! # Integration
//!
//! `CalloraDistribute::pause` and `CalloraDistribute::unpause` call
//! [`set_paused`] after performing authorization and emitting events.
//! Distribution entrypoints call [`require_not_paused`] before any state
//! mutation.

use soroban_sdk::Env;

use crate::limits::StorageKey;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `true` when the circuit-breaker is active.
///
/// Reads [`StorageKey::Paused`] from instance storage.  Returns `false` when
/// the key is absent (safe default — contract starts unpaused).
///
/// This function requires no authorization and is always available.
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&StorageKey::Paused)
        .unwrap_or(false)
}

/// Write the pause flag to [`StorageKey::Paused`].
///
/// The caller is responsible for authorization, admin verification, and event
/// emission before calling this function.  This function performs only the
/// storage write so that those concerns remain in [`crate::CalloraDistribute`].
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&StorageKey::Paused, &paused);
}

/// Abort with [`DistributeError::Paused`](crate::errors::DistributeError::Paused)
/// if the contract is currently paused.
///
/// Must be called by every state-changing entrypoint
/// (`distribute`, `batch_distribute`) before performing any persistent write.
/// Authorization for the entrypoint itself remains the caller's responsibility.
pub fn require_not_paused(env: &Env) {
    if is_paused(env) {
        env.panic_with_error(crate::errors::DistributeError::Paused);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::limits::StorageKey;
    use crate::CalloraDistribute;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    // -----------------------------------------------------------------------
    // Helper: run storage-touching code inside a registered contract context.
    // -----------------------------------------------------------------------
    fn with_contract<F: FnOnce(&Env)>(f: F) {
        let env = Env::default();
        let id = env.register(CalloraDistribute, ());
        env.as_contract(&id, || f(&env));
    }

    // -----------------------------------------------------------------------
    // Core behaviour
    // -----------------------------------------------------------------------

    #[test]
    fn pause_is_false_by_default() {
        with_contract(|env| {
            assert!(!is_paused(env));
        });
    }

    #[test]
    fn set_paused_true_then_false() {
        with_contract(|env| {
            set_paused(env, true);
            assert!(is_paused(env));

            set_paused(env, false);
            assert!(!is_paused(env));
        });
    }

    // -----------------------------------------------------------------------
    // Storage key invariant
    // -----------------------------------------------------------------------

    /// Verify that `set_paused` writes to `StorageKey::Paused` — the single
    /// canonical key shared by the entire contract.
    #[test]
    fn uses_canonical_storage_key() {
        with_contract(|env| {
            set_paused(env, true);

            // Read directly via the canonical key — must see the value.
            let from_key: bool = env
                .storage()
                .instance()
                .get(&StorageKey::Paused)
                .unwrap_or(false);
            assert!(from_key, "set_paused must write to StorageKey::Paused");

            // Module helper must agree.
            assert!(is_paused(env));
        });
    }

    /// Confirm that no bare `Symbol("paused")` string key is present after
    /// writing via `set_paused`.  If it were, the contract and pause module
    /// would read from different slots, silently defeating the circuit-breaker.
    #[test]
    fn no_symbol_key_alias() {
        with_contract(|env| {
            set_paused(env, true);

            // A bare Symbol("paused") key must NOT exist — only StorageKey::Paused.
            let sym_key = soroban_sdk::Symbol::new(env, "paused");
            let via_symbol: bool = env
                .storage()
                .instance()
                .get(&sym_key)
                .unwrap_or(false);
            assert!(
                !via_symbol,
                "pause module must not write to Symbol(\"paused\"); use StorageKey::Paused only"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Guard
    // -----------------------------------------------------------------------

    #[test]
    fn require_not_paused_passes_when_unpaused() {
        with_contract(|env| {
            // Flag is false — guard must not abort.
            assert!(!is_paused(env));
            require_not_paused(env);
        });
    }

    /// When the flag is set, verify the guard condition (`is_paused`) is true.
    /// End-to-end abort behaviour is covered by `distribute_while_paused_fails`
    /// in `test.rs` which uses the contract client.
    #[test]
    fn require_not_paused_is_gated_on_flag() {
        with_contract(|env| {
            set_paused(env, true);
            assert!(is_paused(env), "flag must be true so the guard will fire");
        });
    }

    // -----------------------------------------------------------------------
    // Idempotency
    // -----------------------------------------------------------------------

    #[test]
    fn double_pause_is_idempotent() {
        with_contract(|env| {
            set_paused(env, true);
            set_paused(env, true); // second write — must not panic
            assert!(is_paused(env));
        });
    }

    #[test]
    fn double_resume_is_idempotent() {
        with_contract(|env| {
            set_paused(env, false); // already false — must not panic
            assert!(!is_paused(env));
        });
    }
}
