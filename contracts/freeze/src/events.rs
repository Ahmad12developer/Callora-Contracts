//! Event topic Symbol constructors for the Callora Freeze contract.
//!
//! This module centralises all event topic strings into dedicated functions,
//! ensuring byte-identity is preserved and preventing accidental topic name
//! drift across call sites.
//!
//! # Event catalogue
//!
//! | Event topic            | Emitting entrypoint      | Topics                                    | Data                                |
//! |------------------------|--------------------------|-------------------------------------------|-------------------------------------|
//! | `freeze_initialized`   | `init`                   | `(topic, callora_v1, admin)`              | `()`                                |
//! | `freeze_set`           | `freeze`                 | `(topic, callora_v1, caller)`             | `FreezeSetEvent { reason, frozen_at }` |
//! | `freeze_cleared`       | `unfreeze`               | `(topic, callora_v1, caller)`             | `()`                                |
//! | `freeze_operator_set`  | `set_freeze_operator`    | `(topic, callora_v1, caller)`             | `FreezeOperatorSetEvent { old_operator, new_operator }` |

use soroban_sdk::{contracttype, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Versioned event payload types
// ---------------------------------------------------------------------------

/// Payload for the `freeze_set` event.
///
/// Persisted fields are duplicated here so that an indexer that only stores
/// the data field has full context without a separate view call.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeSetEvent {
    /// The reason label supplied by the caller.
    pub reason: Symbol,
    /// Ledger timestamp at the moment of freeze (`env.ledger().timestamp()`).
    pub frozen_at: u64,
}

/// Payload for the `freeze_operator_set` event.
///
/// `None` in either field represents "no operator" (role cleared / unset).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeOperatorSetEvent {
    /// Operator address before this call, or `None` if none was set.
    pub old_operator: Option<Address>,
    /// New operator address after this call, or `None` if the role was cleared.
    pub new_operator: Option<Address>,
}

// ---------------------------------------------------------------------------
// Topic Symbol constructors
// ---------------------------------------------------------------------------

/// Returns the Symbol for the `"freeze_initialized"` event topic.
///
/// Emitted once when `init()` stores the admin address.
pub fn event_freeze_initialized(env: &Env) -> Symbol {
    Symbol::new(env, "freeze_initialized")
}

/// Returns the Symbol for the `"freeze_set"` event topic.
///
/// Emitted when `freeze()` activates the circuit-breaker.
pub fn event_freeze_set(env: &Env) -> Symbol {
    Symbol::new(env, "freeze_set")
}

/// Returns the Symbol for the `"freeze_cleared"` event topic.
///
/// Emitted when `unfreeze()` deactivates the circuit-breaker.
pub fn event_freeze_cleared(env: &Env) -> Symbol {
    Symbol::new(env, "freeze_cleared")
}

/// Returns the Symbol for the `"freeze_operator_set"` event topic.
///
/// Emitted by `set_freeze_operator()` for both set and clear operations.
pub fn event_freeze_operator_set(env: &Env) -> Symbol {
    Symbol::new(env, "freeze_operator_set")
}

/// Returns the canonical Callora version marker Symbol (`"callora_v1"`).
///
/// Placed at topic[1] for every event so indexers can filter on the version.
/// Must use `_` (underscore) — Soroban Symbol only allows `a-zA-Z0-9_`.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora_v1")
}

// ---------------------------------------------------------------------------
// Emit helpers
// ---------------------------------------------------------------------------

/// Emits the `freeze_initialized` event.
///
/// Topic tuple: `(freeze_initialized, callora_v1, admin)`.
/// Data: `()`.
pub fn emit_freeze_initialized(env: &Env, admin: &Address) {
    env.events().publish(
        (
            event_freeze_initialized(env),
            event_version_v1(env),
            admin.clone(),
        ),
        (),
    );
}

/// Emits the `freeze_set` event.
///
/// Topic tuple: `(freeze_set, callora_v1, caller)`.
/// Data: [`FreezeSetEvent`] with `reason` and `frozen_at`.
pub fn emit_freeze_set(env: &Env, caller: &Address, reason: Symbol, frozen_at: u64) {
    env.events().publish(
        (
            event_freeze_set(env),
            event_version_v1(env),
            caller.clone(),
        ),
        FreezeSetEvent { reason, frozen_at },
    );
}

/// Emits the `freeze_cleared` event.
///
/// Topic tuple: `(freeze_cleared, callora_v1, caller)`.
/// Data: `()`.
pub fn emit_freeze_cleared(env: &Env, caller: &Address) {
    env.events().publish(
        (
            event_freeze_cleared(env),
            event_version_v1(env),
            caller.clone(),
        ),
        (),
    );
}

/// Emits the `freeze_operator_set` event.
///
/// Topic tuple: `(freeze_operator_set, callora_v1, caller)`.
/// Data: [`FreezeOperatorSetEvent`] with `old_operator` and `new_operator`.
pub fn emit_freeze_operator_set(
    env: &Env,
    caller: &Address,
    old_operator: Option<Address>,
    new_operator: Option<Address>,
) {
    env.events().publish(
        (
            event_freeze_operator_set(env),
            event_version_v1(env),
            caller.clone(),
        ),
        FreezeOperatorSetEvent {
            old_operator,
            new_operator,
        },
    );
}

// ---------------------------------------------------------------------------
// Tests — snapshot-style byte-identity assertions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_event_freeze_initialized_bytes() {
        let env = Env::default();
        assert_eq!(
            event_freeze_initialized(&env),
            Symbol::new(&env, "freeze_initialized")
        );
    }

    #[test]
    fn test_event_freeze_set_bytes() {
        let env = Env::default();
        assert_eq!(event_freeze_set(&env), Symbol::new(&env, "freeze_set"));
    }

    #[test]
    fn test_event_freeze_cleared_bytes() {
        let env = Env::default();
        assert_eq!(
            event_freeze_cleared(&env),
            Symbol::new(&env, "freeze_cleared")
        );
    }

    #[test]
    fn test_event_freeze_operator_set_bytes() {
        let env = Env::default();
        assert_eq!(
            event_freeze_operator_set(&env),
            Symbol::new(&env, "freeze_operator_set")
        );
    }

    #[test]
    fn test_event_version_v1_bytes() {
        let env = Env::default();
        assert_eq!(event_version_v1(&env), Symbol::new(&env, "callora_v1"));
    }
}
