//! Event topic Symbol constructors and emit helpers for the Errors contract.
//!
//! # Design
//!
//! This module is the **single source of truth** for every event emitted by
//! the errors contract. It exposes two layers:
//!
//! 1. **Topic constructors** – `event_*(&env) -> Symbol` functions that return
//!    the stable topic Symbol for a given event. These guarantee byte-level
//!    identity and prevent accidental topic-name drift across call sites.
//!
//! 2. **Emit helpers** – `emit_*(&env, …)` functions that bundle the topic
//!    construction and `env.events().publish()` call into one place. Call
//!    sites in `lib.rs` use these helpers exclusively; no inline
//!    `Symbol::new(…)` or `env.events().publish(…)` calls appear outside this
//!    module.
//!
//! # Adding a new event
//!
//! 1. Add `pub fn event_<name>(env: &Env) -> Symbol` with a doc-comment.
//! 2. Add a corresponding `pub fn emit_<name>(env: &Env, …)` function.
//! 3. Add a byte-identity snapshot test in the `#[cfg(test)] mod tests` block.
//!
//! # Event Overview
//!
//! | Topic                | Trigger                                                     |
//! |----------------------|-------------------------------------------------------------|
//! | `"error_registered"` | Admin registers a new error code (`register_error`)         |
//! | `"error_updated"`    | Admin replaces an existing error description (`update_error`) |

use soroban_sdk::{Address, Env, String, Symbol};

// ─── Topic constructors ──────────────────────────────────────────────────────

/// Returns the Symbol for the canonical `"error_registered"` event topic.
///
/// Emitted once per successful [`crate::ErrorsContract::register_error`].
pub fn event_error_registered(env: &Env) -> Symbol {
    Symbol::new(env, "error_registered")
}

/// Returns the Symbol for the canonical `"error_updated"` event topic.
///
/// Emitted once per successful [`crate::ErrorsContract::update_error`].
pub fn event_error_updated(env: &Env) -> Symbol {
    Symbol::new(env, "error_updated")
}

// ─── Emit helpers ────────────────────────────────────────────────────────────

/// Emit `"error_registered"` when the admin registers a new error code.
///
/// **What**: Publishes the registration event containing the registered code
/// and its description.
///
/// **How**: Calls `env.events().publish()` with topic
/// `(error_registered, admin)` and payload `(code, desc)`.
///
/// **Why**: Error descriptions are part of the public interface; indexers
/// need an observable, immutable record of when each code was introduced.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address that performed the registration.
/// * `code` - The registered `u32` error code.
/// * `desc` - The registered description (at most
///   [`crate::MAX_DESC_LEN`] bytes).
pub fn emit_error_registered(env: &Env, admin: &Address, code: u32, desc: &String) {
    env.events().publish(
        (event_error_registered(env), admin.clone()),
        (code, desc.clone()),
    );
}

/// Emit `"error_updated"` when the admin replaces an existing description.
///
/// **What**: Publishes the update event containing the updated code and its
/// new description.
///
/// **How**: Calls `env.events().publish()` with topic
/// `(error_updated, admin)` and payload `(code, desc)`.
///
/// **Why**: The only path that may change a stored description is
/// `update_error`; indexers can observe the new value here rather than
/// discovering a silent overwrite after the fact.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address that performed the update.
/// * `code` - The updated `u32` error code.
/// * `desc` - The new description (at most [`crate::MAX_DESC_LEN`] bytes).
pub fn emit_error_updated(env: &Env, admin: &Address, code: u32, desc: &String) {
    env.events().publish(
        (event_error_updated(env), admin.clone()),
        (code, desc.clone()),
    );
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    // ── Topic byte-identity snapshots ─────────────────────────────────────

    /// Every constructor must map to exactly the expected byte string.
    /// Changing any of these would be a breaking change to the on-chain
    /// interface; that is why they are explicitly snapshot-tested.

    #[test]
    fn test_event_error_registered_bytes() {
        let env = Env::default();
        assert_eq!(
            event_error_registered(&env),
            Symbol::new(&env, "error_registered")
        );
    }

    #[test]
    fn test_event_error_updated_bytes() {
        let env = Env::default();
        assert_eq!(
            event_error_updated(&env),
            Symbol::new(&env, "error_updated")
        );
    }
}
