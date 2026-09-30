use soroban_sdk::{Env, Symbol};

/// Returns the Symbol for the `"init"` event topic.
///
/// Emitted when the distribute contract is first initialized with an admin
/// and USDC token address. Guaranteed to fire exactly once per contract lifetime.
pub fn event_init(env: &Env) -> Symbol {
    Symbol::new(env, "init")
}

/// Returns the Symbol for the `"admin_changed"` event topic.
///
/// Emitted during `set_admin` alongside `admin_transfer_started` to record the
/// before/after admin intent for indexers and audit trails.
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
/// Emitted when the admin distributes USDC to a single recipient via `distribute`.
/// This is the legacy single-event shape retained for backwards compatibility with
/// off-chain subscribers written against the pre-lifecycle schema.
/// New indexers should subscribe to the structured `distribute_started` / `distribute_completed` pair.
pub fn event_distribute(env: &Env) -> Symbol {
    Symbol::new(env, "distribute")
}

/// Returns the Symbol for the `"distribute_started"` event topic.
///
/// Emitted before the USDC transfer begins in the `distribute` entrypoint.
/// Captures the intent to distribute, allowing indexers to track in-flight operations.
/// Pair with `distribute_completed` to confirm atomic success.
pub fn event_distribute_started(env: &Env) -> Symbol {
    Symbol::new(env, "distribute_started")
}

/// Returns the Symbol for the `"distribute_completed"` event topic.
///
/// Emitted after the USDC transfer succeeds in the `distribute` entrypoint.
/// Confirms the distribution completed atomically. Receipt of `distribute_started`
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

/// Returns the Symbol for the `"broadcast"` event topic.
///
/// Emitted when the admin publishes a severity-tagged broadcast message.
pub fn event_broadcast(env: &Env) -> Symbol {
    Symbol::new(env, "broadcast")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora_v1")
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
}
