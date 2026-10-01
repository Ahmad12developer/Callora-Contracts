//! Event topic `Symbol` constructors for the Callora Whitelist contract.
//!
//! Every access-list mutation exposed by [`crate::CalloraWhitelist`] emits
//! exactly one structured event so off-chain indexers can reconstruct *who*
//! changed the whitelist, *when*, and *which* address was affected. Access-list
//! changes are prime audit targets, so the events are first-class parts of the
//! contract interface rather than incidental log lines.
//!
//! # Topic shape
//!
//! Each mutation publishes `(action, version, subject[, affected])`:
//!
//! | Position | Value                                            |
//! |----------|--------------------------------------------------|
//! | topic 0  | action symbol from this module                   |
//! | topic 1  | [`event_version_v1`] — schema version marker     |
//! | topic 2  | the caller (or, for `accept_admin`, the nominee) |
//! | topic 3  | the affected address when there is exactly one   |
//!
//! The remaining structured fields (counts, cooldown seconds) travel in the
//! event **data** payload.
//!
//! # Snapshot tests
//!
//! The tests at the bottom of this file pin every topic to its raw byte
//! representation so an accidental rename fails loudly in `cargo test` before
//! a pull request is opened.

#![allow(dead_code)]

use soroban_sdk::{Env, Symbol};

/// Returns the Symbol for the canonical Callora event version marker.
///
/// Published as topic 1 of every whitelist event so indexers can distinguish
/// the current schema from any future revision. Uses the underscore form
/// (`callora_v1`) because Soroban `Symbol`s reject the `.` character at
/// runtime (same convention as the vault contract).
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora_v1")
}

/// Returns the Symbol for the `"init"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::init`] when the contract is first
/// initialized. The owner/admin address travels in the data payload.
pub fn event_init(env: &Env) -> Symbol {
    Symbol::new(env, "init")
}

/// Returns the Symbol for the `"address_added"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::add_address`] after an address is
/// appended to the whitelist. Duplicate adds return an error and emit nothing.
pub fn event_address_added(env: &Env) -> Symbol {
    Symbol::new(env, "address_added")
}

/// Returns the Symbol for the `"address_removed"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::remove_address`] after an address is
/// removed from the whitelist.
pub fn event_address_removed(env: &Env) -> Symbol {
    Symbol::new(env, "address_removed")
}

/// Returns the Symbol for the `"whitelist_cleared"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::clear_all`] after every address is
/// removed. The number of entries that were cleared travels in the data
/// payload, since `clear_all` has no single affected address.
pub fn event_whitelist_cleared(env: &Env) -> Symbol {
    Symbol::new(env, "whitelist_cleared")
}

/// Returns the Symbol for the `"admin_nominated"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::set_admin`] when the current admin
/// nominates a successor. The transfer only completes on `accept_admin`.
pub fn event_admin_nominated(env: &Env) -> Symbol {
    Symbol::new(env, "admin_nominated")
}

/// Returns the Symbol for the `"admin_accepted"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::accept_admin`] when the nominated
/// admin accepts the role.
pub fn event_admin_accepted(env: &Env) -> Symbol {
    Symbol::new(env, "admin_accepted")
}

/// Returns the Symbol for the `"admin_cooldown_set"` event topic.
///
/// Emitted by [`crate::CalloraWhitelist::set_admin_cooldown`] when the admin
/// updates the global cool-off window. The new window (seconds) travels in the
/// data payload.
pub fn event_admin_cooldown_set(env: &Env) -> Symbol {
    Symbol::new(env, "admin_cooldown_set")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snapshot: proves `event_version_v1` maps to exactly the bytes for `"callora_v1"`.
    #[test]
    fn test_event_version_v1_bytes() {
        let env = Env::default();
        assert_eq!(event_version_v1(&env), Symbol::new(&env, "callora_v1"));
    }

    /// Snapshot: proves `event_init` maps to exactly the bytes for `"init"`.
    #[test]
    fn test_event_init_bytes() {
        let env = Env::default();
        assert_eq!(event_init(&env), Symbol::new(&env, "init"));
    }

    /// Snapshot: proves `event_address_added` maps to exactly the bytes for `"address_added"`.
    #[test]
    fn test_event_address_added_bytes() {
        let env = Env::default();
        assert_eq!(
            event_address_added(&env),
            Symbol::new(&env, "address_added")
        );
    }

    /// Snapshot: proves `event_address_removed` maps to exactly the bytes for `"address_removed"`.
    #[test]
    fn test_event_address_removed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_address_removed(&env),
            Symbol::new(&env, "address_removed")
        );
    }

    /// Snapshot: proves `event_whitelist_cleared` maps to exactly the bytes for `"whitelist_cleared"`.
    #[test]
    fn test_event_whitelist_cleared_bytes() {
        let env = Env::default();
        assert_eq!(
            event_whitelist_cleared(&env),
            Symbol::new(&env, "whitelist_cleared")
        );
    }

    /// Snapshot: proves `event_admin_nominated` maps to exactly the bytes for `"admin_nominated"`.
    #[test]
    fn test_event_admin_nominated_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_nominated(&env),
            Symbol::new(&env, "admin_nominated")
        );
    }

    /// Snapshot: proves `event_admin_accepted` maps to exactly the bytes for `"admin_accepted"`.
    #[test]
    fn test_event_admin_accepted_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_accepted(&env),
            Symbol::new(&env, "admin_accepted")
        );
    }

    /// Snapshot: proves `event_admin_cooldown_set` maps to exactly the bytes for `"admin_cooldown_set"`.
    #[test]
    fn test_event_admin_cooldown_set_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_cooldown_set(&env),
            Symbol::new(&env, "admin_cooldown_set")
        );
    }
}
