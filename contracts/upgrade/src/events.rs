//! Event topic Symbol constructors for the Callora Upgrade contract.
//!
//! This module centralizes all event topic strings into dedicated functions,
//! ensuring byte-identity is preserved and preventing accidental topic name
//! drift across call sites.
//!
//! ## Event Overview
//!
//! | Topic               | Trigger                                            |
//! |---------------------|----------------------------------------------------|
//! | `"upgrade_started"` | Cooldown check passed; upgrade is authorized       |
//! | `"upgrade_recorded"`| Last-upgrade timestamp persisted in storage        |
//! | `"cooldown_set"`    | Cooldown window updated via `set_cooldown`         |
//!
//! ## Topic Shape
//!
//! All events follow the canonical 2-topic shape used across Callora contracts:
//!
//! ```text
//! topics: (action: Symbol, caller: Address)
//! data:   <event-specific payload>
//! ```
//!
//! Off-chain indexers should filter on `topic[0]` (action) and scope queries
//! to the emitting contract address to avoid cross-contract topic collisions.

use soroban_sdk::{Env, Symbol};

/// Returns the Symbol for the `"upgrade_started"` event topic.
///
/// Emitted at the beginning of a successful [`crate::check_and_record_upgrade`]
/// call, immediately after the cooldown check passes and auth is verified.
/// An indexer that receives this topic knows the caller was authorized and the
/// cooldown constraint was satisfied.
///
/// Topics: `(upgrade_started, caller: Address)`
/// Data:   `(current_timestamp: u64, cooldown: u64)`
pub fn event_upgrade_started(env: &Env) -> Symbol {
    Symbol::new(env, "upgrade_started")
}

/// Returns the Symbol for the `"upgrade_recorded"` event topic.
///
/// Emitted after the last-upgrade timestamp has been written to instance
/// storage, confirming that the new baseline for future cooldown checks is
/// persisted. Pairing `upgrade_started` → `upgrade_recorded` lets indexers
/// confirm a complete, non-interrupted upgrade lifecycle.
///
/// Topics: `(upgrade_recorded, caller: Address)`
/// Data:   `(recorded_timestamp: u64)`
pub fn event_upgrade_recorded(env: &Env) -> Symbol {
    Symbol::new(env, "upgrade_recorded")
}

/// Returns the Symbol for the `"cooldown_set"` event topic.
///
/// Emitted when the admin updates the global cooldown window via
/// [`crate::set_cooldown`].
///
/// Topics: `(cooldown_set, caller: Address)`
/// Data:   `(old_cooldown_secs: u64, new_cooldown_secs: u64)`
pub fn event_cooldown_set(env: &Env) -> Symbol {
    Symbol::new(env, "cooldown_set")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora.v1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    /// Snapshot: proves event_upgrade_started still maps to exactly the bytes
    /// for `"upgrade_started"`. If this test fails, the topic was accidentally
    /// renamed — all off-chain indexers subscribed to that topic would silently
    /// stop receiving events.
    #[test]
    fn test_event_upgrade_started_bytes() {
        let env = Env::default();
        assert_eq!(
            event_upgrade_started(&env),
            Symbol::new(&env, "upgrade_started")
        );
    }

    /// Snapshot: proves event_upgrade_recorded still maps to exactly the bytes
    /// for `"upgrade_recorded"`.
    #[test]
    fn test_event_upgrade_recorded_bytes() {
        let env = Env::default();
        assert_eq!(
            event_upgrade_recorded(&env),
            Symbol::new(&env, "upgrade_recorded")
        );
    }

    /// Snapshot: proves event_cooldown_set still maps to exactly the bytes for
    /// `"cooldown_set"`.
    #[test]
    fn test_event_cooldown_set_bytes() {
        let env = Env::default();
        assert_eq!(event_cooldown_set(&env), Symbol::new(&env, "cooldown_set"));
    }
}
