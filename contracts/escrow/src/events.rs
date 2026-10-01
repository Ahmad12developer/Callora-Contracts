//! Event topic Symbol constructors for the Callora Escrow contract.
//!
//! This module centralises all event topic strings into dedicated functions,
//! ensuring byte-identity is preserved and preventing accidental topic name
//! drift across call sites.

use soroban_sdk::{Env, Symbol};

/// Returns the Symbol for the `"init"` event topic.
///
/// Emitted when the escrow contract is first initialized with an admin address
/// and a cooldown window.
pub fn event_init(env: &Env) -> Symbol {
    Symbol::new(env, "init")
}

/// Returns the Symbol for the `"cooldown_set"` event topic.
///
/// Emitted when the admin updates the global cool-off window via
/// [`crate::CalloraEscrow::set_cooldown`].
pub fn event_cooldown_set(env: &Env) -> Symbol {
    Symbol::new(env, "cooldown_set")
}

/// Returns the Symbol for the `"action"` event topic.
///
/// Emitted every time a guarded critical action is successfully executed,
/// recording the action tag so indexers can reconstruct the cool-off timeline.
pub fn event_action(env: &Env) -> Symbol {
    Symbol::new(env, "action")
}

/// Returns the Symbol for the `"signer_rotated"` event topic.
///
/// Emitted when the admin rotates the active signer via
/// [`crate::CalloraEscrow::rotate_signer`]. The event data carries the
/// previous and new signer addresses so monitors can alert on unpredictable
/// signer values without reading contract storage.
pub fn event_signer_rotated(env: &Env) -> Symbol {
    Symbol::new(env, "signer_rotated")
}

/// Returns the Symbol for the `"admin_nominated"` event topic.
///
/// Emitted when the current admin nominates a new admin via
/// [`crate::CalloraEscrow::set_admin`]. The nominated admin must call
/// [`crate::CalloraEscrow::accept_admin`] to complete the transfer.
pub fn event_admin_nominated(env: &Env) -> Symbol {
    Symbol::new(env, "admin_nominated")
}

/// Returns the Symbol for the `"admin_accepted"` event topic.
///
/// Emitted when the pending admin accepts the role via
/// [`crate::CalloraEscrow::accept_admin`], completing the two-step handover.
pub fn event_admin_accepted(env: &Env) -> Symbol {
    Symbol::new(env, "admin_accepted")
}

/// Returns the Symbol for the `"admin_cancelled"` event topic.
///
/// Emitted when the current admin cancels a pending admin nomination via
/// [`crate::CalloraEscrow::cancel_admin_transfer`]. The data contains the
/// address of the pending admin that was cancelled.
pub fn event_admin_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "admin_cancelled")
}

/// Returns the Symbol for the `"asset_approved"` event topic.
///
/// Emitted when the admin marks a payment asset as approved for escrow
/// creation via [`crate::CalloraEscrow::add_approved_asset`].
pub fn event_asset_approved(env: &Env) -> Symbol {
    Symbol::new(env, "asset_approved")
}

/// Returns the Symbol for the `"asset_removed"` event topic.
///
/// Emitted when the admin revokes a payment asset approval via
/// [`crate::CalloraEscrow::remove_approved_asset`].
pub fn event_asset_removed(env: &Env) -> Symbol {
    Symbol::new(env, "asset_removed")
}

/// Returns the Symbol for the `"escrow_created"` event topic.
///
/// Emitted when a new escrow is successfully created against an approved
/// payment asset via [`crate::CalloraEscrow::create_escrow`].
pub fn event_escrow_created(env: &Env) -> Symbol {
    Symbol::new(env, "escrow_created")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora.v1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    /// Snapshot: proves event_init still maps to exactly the bytes for "init".
    #[test]
    fn test_event_init_bytes() {
        let env = Env::default();
        assert_eq!(event_init(&env), Symbol::new(&env, "init"));
    }

    /// Snapshot: proves event_cooldown_set still maps to exactly the bytes for "cooldown_set".
    #[test]
    fn test_event_cooldown_set_bytes() {
        let env = Env::default();
        assert_eq!(event_cooldown_set(&env), Symbol::new(&env, "cooldown_set"));
    }

    /// Snapshot: proves event_action still maps to exactly the bytes for "action".
    #[test]
    fn test_event_action_bytes() {
        let env = Env::default();
        assert_eq!(event_action(&env), Symbol::new(&env, "action"));
    }

    /// Snapshot: proves event_signer_rotated maps to exactly the bytes for "signer_rotated".
    #[test]
    fn test_event_signer_rotated_bytes() {
        let env = Env::default();
        assert_eq!(event_signer_rotated(&env), Symbol::new(&env, "signer_rotated"));
    }

    /// Snapshot: proves event_admin_nominated still maps to exactly the bytes for "admin_nominated".
    #[test]
    fn test_event_admin_nominated_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_nominated(&env),
            Symbol::new(&env, "admin_nominated")
        );
    }

    /// Snapshot: proves event_admin_accepted still maps to exactly the bytes for "admin_accepted".
    #[test]
    fn test_event_admin_accepted_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_accepted(&env),
            Symbol::new(&env, "admin_accepted")
        );
    }

    /// Snapshot: proves event_asset_approved still maps to exactly the bytes for "asset_approved".
    #[test]
    fn test_event_asset_approved_bytes() {
        let env = Env::default();
        assert_eq!(
            event_asset_approved(&env),
            Symbol::new(&env, "asset_approved")
        );
    }

    /// Snapshot: proves event_asset_removed still maps to exactly the bytes for "asset_removed".
    #[test]
    fn test_event_asset_removed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_asset_removed(&env),
            Symbol::new(&env, "asset_removed")
        );
    }

    /// Snapshot: proves event_escrow_created still maps to exactly the bytes for "escrow_created".
    #[test]
    fn test_event_escrow_created_bytes() {
        let env = Env::default();
        assert_eq!(
            event_escrow_created(&env),
            Symbol::new(&env, "escrow_created")
        );
    }
}
