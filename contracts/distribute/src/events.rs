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
/// Emitted when the distribute contract is first initialized with an admin
/// and USDC token address. Guaranteed to fire exactly once per contract lifetime.
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
/// The nominee must call `claim_admin` to complete the two-step transfer.
pub fn event_admin_transfer_started(env: &Env) -> Symbol {
    Symbol::new(env, "admin_transfer_started")
}

/// Returns the Symbol for the `"admin_transfer_completed"` event topic.
///
/// Emitted when the pending admin successfully claims ownership via
/// `claim_admin`, completing the two-step admin handover.
pub fn event_admin_transfer_completed(env: &Env) -> Symbol {
    Symbol::new(env, "admin_transfer_completed")
}

/// Returns the Symbol for the `"admin_cancelled"` event topic.
///
/// Emitted when the current admin cancels a pending admin transfer.
pub fn event_admin_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "admin_cancelled")
}

/// Returns the Symbol for the `"pause_set"` event topic.
///
/// Emitted by both `pause` (with data `true`) and `unpause` (with data `false`)
/// to signal a change in the contract's pause state.
pub fn event_pause_set(env: &Env) -> Symbol {
    Symbol::new(env, "pause_set")
}

/// Returns the Symbol for the `"set_max_distribute"` event topic.
///
/// Emitted when the admin updates the per-leg maximum distribute cap.
pub fn event_set_max_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "set_max_distribute")
}

/// Returns the Symbol for the `"distribute"` event topic.
///
/// Emitted when the admin distributes USDC to a single recipient via `distribute`
/// or per payment leg in `batch_distribute`.
/// New indexers should subscribe to the structured `distribute_started` / `distribute_completed` pair.
pub fn event_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "distribute")
}

/// Returns the Symbol for the `"distribute_started"` event topic.
///
/// Emitted before the USDC transfer begins in the `distribute` entrypoint or per-leg
/// in `batch_distribute`. Captures the intent to distribute, allowing indexers to track in-flight operations.
/// Pair with `distribute_completed` to confirm atomic success.
pub fn event_distribute_started(env: &Env) -> Symbol {
    Symbol::new(env, "distribute_started")
}

/// Returns the Symbol for the `"distribute_completed"` event topic.
///
/// Emitted after the USDC transfer succeeds in the `distribute` entrypoint or per-leg
/// in `batch_distribute`. Confirms the distribution completed atomically. Receipt of `distribute_started`
/// without a matching `distribute_completed` at the same ledger indicates failure.
pub fn event_distribute_completed(env: &Env) -> Symbol {
    Symbol::new(env, "distribute_completed")
}

/// Returns the Symbol for the `"upgraded"` event topic.
///
/// Emitted when the admin upgrades the contract to a new WASM hash via `upgrade`.
pub fn event_upgraded(env: &Env) -> Symbol {
    Symbol::new(env, "upgraded")
}

/// Returns the Symbol for the `"batch_distribute_started"` event topic.
///
/// Emitted before batch distribution starts. Carries the total amount and
/// count of legs as data for indexers to validate against the completed event.
pub fn event_batch_distribute_started(env: &Env) -> Symbol {
    Symbol::new(env, "batch_distribute_started")
}

/// Returns the Symbol for the `"batch_distribute_completed"` event topic.
///
/// Emitted after a successful batch distribution. Carries the total amount and
/// count of legs as data, matching the started event for atomicity verification.
pub fn event_batch_distribute_completed(env: &Env) -> Symbol {
    Symbol::new(env, "batch_distribute_completed")
}

/// Returns the Symbol for the `"batch_distribute"` event topic.
pub fn event_batch_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "batch_distribute")
}

/// Returns the Symbol for the `"batch_leg"` event topic.
pub fn event_batch_leg(env: &Env) -> Symbol {
    Symbol::new(env, "batch_leg")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora_v1")
}

/// Emits a structured lifecycle event immediately before a validated distribution transfer.
pub fn emit_distribute_started(
    env: &Env,
    recipient: &Address,
    payload: &DistributionLifecycleEvent,
) {
    env.events().publish(
        (
            event_distribute_started(env),
            event_version_v1(env),
            recipient.clone(),
        ),
        payload.clone(),
    );
}

/// Emits a structured lifecycle event after a distribution transfer succeeds.
pub fn emit_distribute_completed(
    env: &Env,
    recipient: &Address,
    payload: &DistributionLifecycleEvent,
) {
    env.events().publish(
        (
            event_distribute_completed(env),
            event_version_v1(env),
            recipient.clone(),
        ),
        payload.clone(),
    );
}

/// Emits a per-leg distribution transfer event with recipient topic and amount data.
pub fn emit_distribute(env: &Env, recipient: &Address, amount: i128) {
    env.events().publish(
        (
            event_distribute(env),
            event_version_v1(env),
            recipient.clone(),
        ),
        amount,
    );
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

    /// Snapshot: proves event_pause_set still maps to exactly the bytes for "pause_set".
    #[test]
    fn test_event_pause_set_bytes() {
        let env = Env::default();
        assert_eq!(event_pause_set(&env), Symbol::new(&env, "pause_set"));
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

    /// Snapshot: proves event_distribute still maps to exactly the bytes for "distribute".
    #[test]
    fn test_event_distribute_bytes() {
        let env = Env::default();
        assert_eq!(event_distribute(&env), Symbol::new(&env, "distribute"));
    }

    /// Snapshot: proves event_upgraded still maps to exactly the bytes for "upgraded".
    #[test]
    fn test_event_upgraded_bytes() {
        let env = Env::default();
        assert_eq!(event_upgraded(&env), Symbol::new(&env, "upgraded"));
    }

    /// Snapshot: proves event_distribute_started still maps to exactly the bytes for "distribute_started".
    #[test]
    fn test_event_distribute_started_bytes() {
        let env = Env::default();
        assert_eq!(
            event_distribute_started(&env),
            Symbol::new(&env, "distribute_started")
        );
    }

    /// Snapshot: proves event_distribute_completed still maps to exactly the bytes for "distribute_completed".
    #[test]
    fn test_event_distribute_completed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_distribute_completed(&env),
            Symbol::new(&env, "distribute_completed")
        );
    }

    /// Snapshot: proves event_batch_distribute_started still maps to exactly the bytes for "batch_distribute_started".
    #[test]
    fn test_event_batch_distribute_started_bytes() {
        let env = Env::default();
        assert_eq!(
            event_batch_distribute_started(&env),
            Symbol::new(&env, "batch_distribute_started")
        );
    }

    /// Snapshot: proves event_batch_distribute_completed still maps to exactly the bytes for "batch_distribute_completed".
    #[test]
    fn test_event_batch_distribute_completed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_batch_distribute_completed(&env),
            Symbol::new(&env, "batch_distribute_completed")
        );
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

    /// Snapshot: proves event_batch_leg still maps to exactly the bytes for "batch_leg".
    #[test]
    fn test_event_batch_leg_bytes() {
        let env = Env::default();
        assert_eq!(event_batch_leg(&env), Symbol::new(&env, "batch_leg"));
    }

    /// Snapshot: proves event_version_v1 still maps to exactly the bytes for "callora_v1".
    #[test]
    fn test_event_version_v1_bytes() {
        let env = Env::default();
        assert_eq!(event_version_v1(&env), Symbol::new(&env, "callora_v1"));
    }

    /// Verifies DistributionLifecycleEvent constructor populates all fields.
    #[test]
    fn test_distribution_lifecycle_event_new() {
        let env = Env::default();
        let payload = DistributionLifecycleEvent::new(&env, 500_000, DistributionMode::Batch, 2, 5);
        assert_eq!(payload.version, DISTRIBUTION_EVENT_VERSION);
        assert_eq!(payload.amount, 500_000);
        assert_eq!(payload.mode, DistributionMode::Batch);
        assert_eq!(payload.batch_index, 2);
        assert_eq!(payload.batch_size, 5);
        assert_eq!(payload.ledger_sequence, env.ledger().sequence());
        assert_eq!(payload.timestamp, env.ledger().timestamp());
    }
}
