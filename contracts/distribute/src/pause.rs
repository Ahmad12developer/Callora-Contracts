//! Administrative pause control for the Callora Distribute contract.
//!
//! This module owns the canonical pause storage key ([`StorageKey::Paused`])
//! and provides pause inspection and gating primitives for the contract.
//!
//! # Storage key invariant & migration
//!
//! All reads and writes target [`StorageKey::Paused`]. To support contracts
//! upgraded from earlier versions where the flag was stored under
//! `Symbol("paused")`, reads transparently migrate any legacy symbol key
//! into [`StorageKey::Paused`] and delete the obsolete entry.
//!
//! # Integration
//!
//! [`crate::Distribute::pause`] and [`crate::Distribute::unpause`] call
//! [`set_paused`] after performing authorization and emitting events.
//! Distribution entrypoints call [`require_not_paused`] before any state
//! mutation.

use soroban_sdk::{Env, Symbol};

use crate::limits::StorageKey;

/// Legacy instance storage key string used prior to issue #1175.
pub const LEGACY_PAUSED_SYMBOL: &str = "paused";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `true` when the circuit-breaker is active.
///
/// Reads [`StorageKey::Paused`] from instance storage. If absent, checks
/// for the legacy `Symbol("paused")` key, migrates it to [`StorageKey::Paused`],
/// removes the legacy symbol key, and returns the value. Returns `false`
/// if neither key is found.
///
/// This function requires no authorization and is always available.
pub fn is_paused(env: &Env) -> bool {
    let inst = env.storage().instance();
    if let Some(paused) = inst.get(&StorageKey::Paused) {
        paused
    } else {
        let legacy_sym = Symbol::new(env, LEGACY_PAUSED_SYMBOL);
        if let Some(paused) = inst.get::<_, bool>(&legacy_sym) {
            inst.set(&StorageKey::Paused, &paused);
            inst.remove(&legacy_sym);
            paused
        } else {
            false
        }
    }
}

/// Sets the pause state in instance storage under [`StorageKey::Paused`].
///
/// Ensures any legacy `Symbol("paused")` key is removed so that only
/// one canonical pause key exists in storage.
pub fn set_paused(env: &Env, paused: bool) {
    let inst = env.storage().instance();
    inst.set(&StorageKey::Paused, &paused);
    let legacy_sym = Symbol::new(env, LEGACY_PAUSED_SYMBOL);
    if inst.has(&legacy_sym) {
        inst.remove(&legacy_sym);
    }
}

/// Rejects execution with [`DistributeError::Paused`](crate::errors::DistributeError::Paused)
/// if the contract is currently paused.
///
/// Must be called by every state-changing entrypoint before performing any
/// persistent write.
pub fn require_not_paused(env: &Env) {
    if is_paused(env) {
        env.panic_with_error(crate::errors::DistributeError::Paused);
    }
}

/// Programmatic helper to pause the contract without auth checks.
///
/// Note: When called from contract entrypoints, use [`crate::Distribute::pause`]
/// which enforces administrator authentication and emits audit events.
pub fn pause(env: &Env) {
    set_paused(env, true);
}

/// Programmatic helper to resume the contract without auth checks.
///
/// Note: When called from contract entrypoints, use [`crate::Distribute::unpause`]
/// which enforces administrator authentication and emits audit events.
pub fn resume(env: &Env) {
    set_paused(env, false);
}

/// Alias for [`resume`].
pub fn unpause(env: &Env) {
    resume(env);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::limits::StorageKey;
    use crate::Distribute;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn with_contract<F: FnOnce(&Env, Address)>(f: F) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let usdc = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let id = env.register(Distribute, ());
        env.as_contract(&id, || {
            crate::Distribute::init(env.clone(), admin.clone(), usdc);
            f(&env, admin);
        });
    }

    #[test]
    fn pause_is_false_by_default() {
        with_contract(|env, _admin| {
            assert!(!is_paused(env));
        });
    }

    #[test]
    fn set_paused_true_then_false() {
        with_contract(|env, _admin| {
            set_paused(env, true);
            assert!(is_paused(env));

            set_paused(env, false);
            assert!(!is_paused(env));
        });
    }

    #[test]
    fn helper_pause_and_resume() {
        with_contract(|env, _admin| {
            pause(env);
            assert!(is_paused(env));

            resume(env);
            assert!(!is_paused(env));

            unpause(env);
            assert!(!is_paused(env));
        });
    }

    #[test]
    fn uses_canonical_storage_key() {
        with_contract(|env, _admin| {
            set_paused(env, true);

            let from_key: bool = env
                .storage()
                .instance()
                .get(&StorageKey::Paused)
                .unwrap_or(false);
            assert!(from_key, "set_paused must write to StorageKey::Paused");
            assert!(is_paused(env));
        });
    }

    #[test]
    fn no_symbol_key_alias() {
        with_contract(|env, _admin| {
            set_paused(env, true);

            let sym_key = Symbol::new(env, LEGACY_PAUSED_SYMBOL);
            assert!(
                !env.storage().instance().has(&sym_key),
                "pause module must not retain Symbol(\"paused\")"
            );
        });
    }

    #[test]
    fn migrates_legacy_symbol_key_on_read() {
        with_contract(|env, _admin| {
            let inst = env.storage().instance();
            // Simulate legacy contract state: remove enum key, set symbol key
            inst.remove(&StorageKey::Paused);
            let legacy_sym = Symbol::new(env, LEGACY_PAUSED_SYMBOL);
            inst.set(&legacy_sym, &true);

            // Read triggers on-the-fly migration
            assert!(is_paused(env));

            // Enum key is populated, legacy symbol key is pruned
            assert_eq!(inst.get(&StorageKey::Paused), Some(true));
            assert!(!inst.has(&legacy_sym));
        });
    }

    #[test]
    fn set_paused_cleans_up_legacy_symbol_key() {
        with_contract(|env, _admin| {
            let inst = env.storage().instance();
            let legacy_sym = Symbol::new(env, LEGACY_PAUSED_SYMBOL);
            inst.set(&legacy_sym, &true);

            set_paused(env, false);

            assert_eq!(inst.get(&StorageKey::Paused), Some(false));
            assert!(!inst.has(&legacy_sym));
            assert!(!is_paused(env));
        });
    }

    #[test]
    fn require_not_paused_passes_when_unpaused() {
        with_contract(|env, _admin| {
            assert!(!is_paused(env));
            require_not_paused(env);
        });
    }

    #[test]
    fn require_not_paused_is_gated_on_flag() {
        with_contract(|env, _admin| {
            set_paused(env, true);
            assert!(is_paused(env));
        });
    }

    #[test]
    fn double_pause_is_idempotent() {
        with_contract(|env, _admin| {
            set_paused(env, true);
            set_paused(env, true);
            assert!(is_paused(env));
        });
    }

    #[test]
    fn double_resume_is_idempotent() {
        with_contract(|env, _admin| {
            set_paused(env, false);
            assert!(!is_paused(env));
        });
    }
}
