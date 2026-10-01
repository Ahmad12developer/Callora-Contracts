//! Event topic Symbol constructors for the Callora Revenue Pool contract.
//!
//! This module centralizes all event topic strings into dedicated functions,
//! ensuring byte-identity is preserved and preventing accidental topic name drift
//! across call sites.

use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Schema version for structured distribution lifecycle event payloads.
pub const DISTRIBUTION_EVENT_VERSION: u32 = 1;

/// Identifies which distribution entry point emitted a lifecycle event.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistributionMode {
    Single,
    Batch,
}

/// Stable, versioned payload shared by distribution lifecycle events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionLifecycleEvent {
    pub version: u32,
    pub amount: i128,
    pub mode: DistributionMode,
    pub batch_index: u32,
    pub batch_size: u32,
    pub ledger_sequence: u32,
    pub timestamp: u64,
}

impl DistributionLifecycleEvent {
    pub fn new(
        env: &Env,
        amount: i128,
        mode: DistributionMode,
        batch_index: u32,
        batch_size: u32,
    ) -> Self {
        Self {
            version: DISTRIBUTION_EVENT_VERSION,
            amount,
            mode,
            batch_index,
            batch_size,
            ledger_sequence: env.ledger().sequence(),
            timestamp: env.ledger().timestamp(),
        }
    }
}

/// Returns the Symbol for the `"init"` event topic.
///
/// Emitted when the revenue pool is first initialized with an admin and USDC token address.
pub fn event_init(env: &Env) -> Symbol {
    Symbol::new(env, "init")
}

/// Returns the Symbol for the `"admin_changed"` event topic.
///
/// Emitted by `accept_admin` (step 2 of the two-step rotation) once the admin
/// slot has been updated, carrying `(previous_admin, new_admin)` so indexers
/// record the change only when it actually happens. It is deliberately not
/// published by `set_admin`, which only nominates a successor.
pub fn event_admin_changed(env: &Env) -> Symbol {
    Symbol::new(env, "admin_changed")
}

/// Returns the Symbol for the `"admin_transfer_started"` event topic.
///
/// Emitted when the current admin nominates a new admin via `set_admin`.
/// The nominated admin must call `claim_admin` to complete the transfer.
pub fn event_admin_transfer_started(env: &Env) -> Symbol {
    Symbol::new(env, "admin_transfer_started")
}

/// Returns the Symbol for the `"admin_transfer_completed"` event topic.
///
/// Emitted when the pending admin successfully claims ownership via `claim_admin`,
/// completing the two-step admin handover.
pub fn event_admin_transfer_completed(env: &Env) -> Symbol {
    Symbol::new(env, "admin_transfer_completed")
}

/// Returns the Symbol for the `"admin_cancelled"` event topic.
///
/// Emitted when the current admin cancels a pending admin transfer.
pub fn event_admin_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "admin_cancelled")
}

/// Returns the Symbol for the `"pause_guardian_set"` event topic.
///
/// Emitted when the admin sets or replaces the emergency pause guardian.
pub fn event_pause_guardian_set(env: &Env) -> Symbol {
    Symbol::new(env, "pause_guardian_set")
}

/// Returns the Symbol for the `"pause_guardian_cleared"` event topic.
///
/// Emitted when the admin clears the emergency pause guardian.
pub fn event_pause_guardian_cleared(env: &Env) -> Symbol {
    Symbol::new(env, "pause_guardian_cleared")
}

/// Returns the Symbol for the `"pause_set"` event topic.
///
/// Emitted by both `pause` (with data `true`) and `unpause` (with data `false`)
/// to signal a change in the pool's pause state.
pub fn event_pause_set(env: &Env) -> Symbol {
    Symbol::new(env, "pause_set")
}

/// Returns the Symbol for the `"emergency_pause_set"` event topic.
///
/// Emitted when recovery-only emergency mode is entered or cleared.
pub fn event_emergency_pause_set(env: &Env) -> Symbol {
    Symbol::new(env, "emergency_pause_set")
}

/// Returns the Symbol for the `"receive_payment"` event topic.
///
/// Emitted when the admin calls `receive_payment` to log an incoming payment
/// from the vault for indexer alignment.
pub fn event_receive_payment(env: &Env) -> Symbol {
    Symbol::new(env, "receive_payment")
}

/// Returns the Symbol for the `"yield_deposited"` event topic.
///
/// Emitted when the treasury deposits accumulated protocol yield into the
/// revenue pool via `deposit_yield`.
pub fn event_yield_deposited(env: &Env) -> Symbol {
    Symbol::new(env, "yield_deposited")
}

/// Returns the Symbol for the `"treasury_transfer_started"` event topic.
///
/// Emitted when the admin nominates a new treasury via [`RevenuePool::set_treasury`].
/// The nominee must call [`RevenuePool::accept_treasury`] to complete the transfer.
pub fn event_treasury_transfer_started(env: &Env) -> Symbol {
    Symbol::new(env, "treasury_transfer_started")
}

/// Returns the Symbol for the `"treasury_transfer_completed"` event topic.
///
/// Emitted when the pending treasury accepts the nomination via
/// [`RevenuePool::accept_treasury`], completing the two-step handover.
pub fn event_treasury_transfer_completed(env: &Env) -> Symbol {
    Symbol::new(env, "treasury_transfer_completed")
}

/// Returns the Symbol for the `"treasury_cancelled"` event topic.
///
/// Emitted when the admin cancels a pending treasury nomination via
/// [`RevenuePool::cancel_treasury_transfer`].
pub fn event_treasury_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "treasury_cancelled")
}

/// Returns the Symbol for the `"set_max_distribute"` event topic.
///
/// Emitted when the admin updates the per-leg maximum distribute cap.
pub fn event_set_max_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "set_max_distribute")
}

/// Returns the Symbol for the `"distribute"` event topic.
///
/// Emitted when the admin distributes USDC to a single developer wallet via `distribute`.
pub fn event_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "distribute")
}

/// Returns the Symbol for the `"batch_distribute"` event topic.
///
/// Emitted once per payment leg during a `batch_distribute` call, after all
/// validation has passed.
pub fn event_batch_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "batch_distribute")
}

/// Returns the Symbol for the `"distribute_started"` lifecycle event topic.
pub fn event_distribute_started(env: &Env) -> Symbol {
    Symbol::new(env, "distribute_started")
}

/// Returns the Symbol for the `"distribute_completed"` lifecycle event topic.
pub fn event_distribute_completed(env: &Env) -> Symbol {
    Symbol::new(env, "distribute_completed")
}

/// Emits a structured event immediately before a validated distribution transfer.
pub fn emit_distribute_started(
    env: &Env,
    caller: &Address,
    recipient: &Address,
    payload: &DistributionLifecycleEvent,
) {
    env.events().publish(
        (event_distribute_started(env), caller, recipient),
        payload.clone(),
    );
}

/// Emits a structured event after a distribution transfer succeeds.
pub fn emit_distribute_completed(
    env: &Env,
    caller: &Address,
    recipient: &Address,
    payload: &DistributionLifecycleEvent,
) {
    env.events().publish(
        (event_distribute_completed(env), caller, recipient),
        payload.clone(),
    );
}

/// Returns the Symbol for the `"upgraded"` event topic.
///
/// Emitted when the admin upgrades the contract to a new WASM hash via `upgrade`.
pub fn event_upgraded(env: &Env) -> Symbol {
    Symbol::new(env, "upgraded")
}

/// Returns the Symbol for the `"admin_broadcast"` event topic.
///
/// Emitted when the admin broadcasts an emergency message.
pub fn event_admin_broadcast(env: &Env) -> Symbol {
    Symbol::new(env, "admin_broadcast")
}

/// Returns the Symbol for the `"emergency_drain_proposed"` event topic.
///
/// Emitted when the admin proposes a timelocked emergency drain via
/// [`RevenuePool::propose_emergency_drain`].
pub fn event_emergency_drain_proposed(env: &Env) -> Symbol {
    Symbol::new(env, "emergency_drain_proposed")
}

/// Returns the Symbol for the `"emergency_drain_executed"` event topic.
///
/// Emitted when the admin executes a pending emergency drain after the
/// timelock has expired via [`RevenuePool::execute_emergency_drain`].
pub fn event_emergency_drain_executed(env: &Env) -> Symbol {
    Symbol::new(env, "emergency_drain_executed")
}

/// Returns the Symbol for the `"emergency_drain_cancelled"` event topic.
///
/// Emitted when the admin cancels a pending emergency drain via
/// [`RevenuePool::cancel_emergency_drain`].
pub fn event_emergency_drain_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "emergency_drain_cancelled")
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

    /// Snapshot: proves event_admin_changed still maps to exactly the bytes for "admin_changed".
    #[test]
    fn test_event_admin_changed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_changed(&env),
            Symbol::new(&env, "admin_changed")
        );
    }

    /// Snapshot: proves event_admin_transfer_started still maps to exactly the bytes for "admin_transfer_started".
    #[test]
    fn test_event_admin_transfer_started_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_transfer_started(&env),
            Symbol::new(&env, "admin_transfer_started")
        );
    }

    /// Snapshot: proves event_admin_transfer_completed still maps to exactly the bytes for "admin_transfer_completed".
    #[test]
    fn test_event_admin_transfer_completed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_transfer_completed(&env),
            Symbol::new(&env, "admin_transfer_completed")
        );
    }

    /// Snapshot: proves event_admin_cancelled still maps to exactly the bytes for "admin_cancelled".
    #[test]
    fn test_event_admin_cancelled_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_cancelled(&env),
            Symbol::new(&env, "admin_cancelled")
        );
    }

    /// Snapshot: proves event_pause_guardian_set still maps to exactly the bytes for "pause_guardian_set".
    #[test]
    fn test_event_pause_guardian_set_bytes() {
        let env = Env::default();
        assert_eq!(
            event_pause_guardian_set(&env),
            Symbol::new(&env, "pause_guardian_set")
        );
    }

    /// Snapshot: proves event_pause_guardian_cleared still maps to exactly the bytes for "pause_guardian_cleared".
    #[test]
    fn test_event_pause_guardian_cleared_bytes() {
        let env = Env::default();
        assert_eq!(
            event_pause_guardian_cleared(&env),
            Symbol::new(&env, "pause_guardian_cleared")
        );
    }

    /// Snapshot: proves event_pause_set still maps to exactly the bytes for "pause_set".
    #[test]
    fn test_event_pause_set_bytes() {
        let env = Env::default();
        assert_eq!(event_pause_set(&env), Symbol::new(&env, "pause_set"));
    }

    #[test]
    fn test_event_emergency_pause_set_bytes() {
        let env = Env::default();
        assert_eq!(
            event_emergency_pause_set(&env),
            Symbol::new(&env, "emergency_pause_set")
        );
    }

    /// Snapshot: proves event_receive_payment still maps to exactly the bytes for "receive_payment".
    #[test]
    fn test_event_receive_payment_bytes() {
        let env = Env::default();
        assert_eq!(
            event_receive_payment(&env),
            Symbol::new(&env, "receive_payment")
        );
    }

    #[test]
    fn test_event_treasury_transfer_started_bytes() {
        let env = Env::default();
        assert_eq!(
            event_treasury_transfer_started(&env),
            Symbol::new(&env, "treasury_transfer_started")
        );
    }

    #[test]
    fn test_event_treasury_transfer_completed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_treasury_transfer_completed(&env),
            Symbol::new(&env, "treasury_transfer_completed")
        );
    }

    #[test]
    fn test_event_treasury_cancelled_bytes() {
        let env = Env::default();
        assert_eq!(
            event_treasury_cancelled(&env),
            Symbol::new(&env, "treasury_cancelled")
        );
    }

    /// Snapshot: proves event_set_max_distribute still maps to exactly the bytes for "set_max_distribute".
    #[test]
    fn test_event_set_max_distribute_bytes() {
        let env = Env::default();
        assert_eq!(
            event_set_max_distribute(&env),
            Symbol::new(&env, "set_max_distribute")
        );
    }

    /// Snapshot: proves event_yield_deposited still maps to exactly the bytes for "yield_deposited".
    #[test]
    fn test_event_yield_deposited_bytes() {
        let env = Env::default();
        assert_eq!(
            event_yield_deposited(&env),
            Symbol::new(&env, "yield_deposited")
        );
    }

    /// Snapshot: proves event_distribute still maps to exactly the bytes for "distribute".
    #[test]
    fn test_event_distribute_bytes() {
        let env = Env::default();
        assert_eq!(event_distribute(&env), Symbol::new(&env, "distribute"));
    }

    /// Snapshot: proves event_batch_distribute still maps to exactly the bytes for "batch_distribute".
    #[test]
    fn test_event_batch_distribute_bytes() {
        let env = Env::default();
        assert_eq!(
            event_batch_distribute(&env),
            Symbol::new(&env, "batch_distribute")
        );
    }

    #[test]
    fn test_distribution_lifecycle_event_bytes() {
        let env = Env::default();
        assert_eq!(
            event_distribute_started(&env),
            Symbol::new(&env, "distribute_started")
        );
        assert_eq!(
            event_distribute_completed(&env),
            Symbol::new(&env, "distribute_completed")
        );
    }

    #[test]
    fn test_distribution_lifecycle_payload_is_versioned() {
        let env = Env::default();
        let payload = DistributionLifecycleEvent::new(&env, 42, DistributionMode::Batch, 1, 3);

        assert_eq!(payload.version, DISTRIBUTION_EVENT_VERSION);
        assert_eq!(payload.amount, 42);
        assert_eq!(payload.mode, DistributionMode::Batch);
        assert_eq!(payload.batch_index, 1);
        assert_eq!(payload.batch_size, 3);
        assert_eq!(payload.ledger_sequence, env.ledger().sequence());
        assert_eq!(payload.timestamp, env.ledger().timestamp());
    }

    /// Snapshot: proves event_upgraded still maps to exactly the bytes for "upgraded".
    #[test]
    fn test_event_upgraded_bytes() {
        let env = Env::default();
        assert_eq!(event_upgraded(&env), Symbol::new(&env, "upgraded"));
    }

    /// Snapshot: proves event_admin_broadcast still maps to exactly the bytes for "admin_broadcast".
    #[test]
    fn test_event_admin_broadcast_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_broadcast(&env),
            Symbol::new(&env, "admin_broadcast")
        );
    }

    /// Snapshot: proves event_emergency_drain_proposed still maps to exactly the bytes for "emergency_drain_proposed".
    #[test]
    fn test_event_emergency_drain_proposed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_emergency_drain_proposed(&env),
            Symbol::new(&env, "emergency_drain_proposed")
        );
    }

    /// Snapshot: proves event_emergency_drain_executed still maps to exactly the bytes for "emergency_drain_executed".
    #[test]
    fn test_event_emergency_drain_executed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_emergency_drain_executed(&env),
            Symbol::new(&env, "emergency_drain_executed")
        );
    }

    /// Snapshot: proves event_emergency_drain_cancelled still maps to exactly the bytes for "emergency_drain_cancelled".
    #[test]
    fn test_event_emergency_drain_cancelled_bytes() {
        let env = Env::default();
        assert_eq!(
            event_emergency_drain_cancelled(&env),
            Symbol::new(&env, "emergency_drain_cancelled")
        );
    }
}
