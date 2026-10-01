//! Event topic Symbol constructors and structured emit helpers for the
//! Callora Rescue contract.
//!
//! # Design
//!
//! This module is the **single source of truth** for every event emitted by
//! the rescue contract. It exposes two layers:
//!
//! 1. **Topic constructors** – `event_*(&env) -> Symbol` functions that return
//!    the stable topic Symbol for a given event. These guarantee byte-level
//!    identity and prevent accidental topic-name drift across call sites.
//!
//! 2. **Emit helpers** – `emit_*(&env, …)` functions that bundle the topic
//!    construction, data struct construction, and `env.events().publish()` call
//!    into one place. Call sites in `lib.rs` use these helpers exclusively; no
//!    inline `Symbol::new(…)` or `env.events().publish(…)` calls appear outside
//!    this module.
//!
//! # Adding a new event
//!
//! 1. Add `pub fn event_<name>(env: &Env) -> Symbol` with a doc-comment.
//! 2. Add a corresponding `pub fn emit_<name>(env: &Env, …)` function.
//! 3. Add a snapshot test in the `#[cfg(test)] mod tests` block asserting
//!    byte-identity of the topic string.
//! 4. Update `docs/EVENT_TOPICS.md`.
//!
//! # Event Overview
//!
//! | Topic               | Trigger                                  |
//! |---------------------|------------------------------------------|
//! | `"initialized"`     | Contract initialised with admin (`init`) |
//! | `"rescue"`          | Admin transfers tokens (`rescue`)        |
//! | `"rescue_capped"`   | Admin transfers tokens with a per-call cap (`rescue_capped`) |

use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Schema version for structured rescue lifecycle event payloads.
pub const RESCUE_EVENT_VERSION: u32 = 1;

/// Indicates whether a rescue operation was subject to a per-call cap.
///
/// `Standard` rescue has no cap; `Capped` rescue includes the cap value
/// so off-chain indexers can distinguish the two entrypoints from the
/// event payload alone.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RescueCap {
    /// No cap applied — emitted by `rescue`.
    None,
    /// Cap applied — emitted by `rescue_capped` with the limit that was enforced.
    Some(i128),
}

/// Stable, versioned payload shared by rescue lifecycle events.
///
/// Emitted alongside every rescue execution so off-chain indexers can
/// reconstruct the complete recovery audit trail without polling storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RescueEvent {
    /// Schema version of this payload. Incremented on breaking shape changes.
    pub version: u32,
    /// Address of the token contract being rescued.
    pub token: Address,
    /// Destination address receiving the rescued tokens.
    pub to: Address,
    /// Amount of tokens transferred in this rescue operation.
    pub amount: i128,
    /// Per-call cap applied, if any. `RescueCap::None` for `rescue`,
    /// `RescueCap::Some(cap)` for `rescue_capped`.
    pub cap: RescueCap,
    /// Running cumulative total of all tokens rescued after this operation.
    pub cumulative_rescued: i128,
    /// Ledger sequence number at the time of rescue, for causal ordering.
    pub ledger_sequence: u32,
    /// Ledger timestamp (seconds since Unix epoch) at the time of rescue.
    pub timestamp: u64,
}

impl RescueEvent {
    /// Construct a new rescue event payload, capturing the current ledger context.
    ///
    /// # Arguments
    /// * `env` - Soroban environment handle for ledger metadata.
    /// * `token` - Token contract address.
    /// * `to` - Recipient address.
    /// * `amount` - Amount transferred.
    /// * `cap` - Per-call cap mode (`RescueCap::None` or `RescueCap::Some(cap)`).
    /// * `cumulative_rescued` - Running total after this operation.
    pub fn new(
        env: &Env,
        token: Address,
        to: Address,
        amount: i128,
        cap: RescueCap,
        cumulative_rescued: i128,
    ) -> Self {
        Self {
            version: RESCUE_EVENT_VERSION,
            token,
            to,
            amount,
            cap,
            cumulative_rescued,
            ledger_sequence: env.ledger().sequence(),
            timestamp: env.ledger().timestamp(),
        }
    }
}

// ─── Topic constructors ──────────────────────────────────────────────────────

/// Returns the Symbol for the `"initialized"` event topic.
///
/// **What**: Returns the canonical symbol for contract initialization events.
///
/// **How**: Creates a `Symbol` from `"initialized"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_initialized(env: &Env) -> Symbol {
    Symbol::new(env, "initialized")
}

/// Returns the Symbol for the `"rescue"` event topic.
///
/// **What**: Returns the canonical symbol for uncapped rescue events.
///
/// **How**: Creates a `Symbol` from `"rescue"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_rescue(env: &Env) -> Symbol {
    Symbol::new(env, "rescue")
}

/// Returns the Symbol for the `"rescue_capped"` event topic.
///
/// **What**: Returns the canonical symbol for capped rescue events.
///
/// **How**: Creates a `Symbol` from `"rescue_capped"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_rescue_capped(env: &Env) -> Symbol {
    Symbol::new(env, "rescue_capped")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora.v1")
}

// ─── Emit helpers ────────────────────────────────────────────────────────────

/// Emit `"initialized"` once when the rescue contract is first set up.
///
/// **What**: Publishes the initialization event containing the admin address.
///
/// **How**: Calls `env.events().publish()` with topic `(initialized, admin)` and
/// payload `admin`.
///
/// **Why**: Allows off-chain indexers to discover the contract deployment and
/// initial admin identity.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Primary contract administrator address.
pub fn emit_initialized(env: &Env, admin: &Address) {
    env.events()
        .publish((event_initialized(env), admin.clone()), admin.clone());
}

/// Emit `"rescue"` when the admin executes an uncapped token rescue.
///
/// **What**: Publishes a structured rescue event recording token, recipient,
/// amount, and cumulative total.
///
/// **How**: Calls `env.events().publish()` with topic `(rescue, admin, token)`
/// and payload `RescueEvent`.
///
/// **Why**: Audit trail for off-chain indexers tracking token recovery operations.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address executing the rescue.
/// * `payload` - Structured rescue event details.
pub fn emit_rescue(env: &Env, admin: &Address, payload: &RescueEvent) {
    env.events().publish(
        (event_rescue(env), admin.clone(), payload.token.clone()),
        payload.clone(),
    );
}

/// Emit `"rescue_capped"` when the admin executes a capped token rescue.
///
/// **What**: Publishes a structured rescue event recording token, recipient,
/// amount, cap, and cumulative total.
///
/// **How**: Calls `env.events().publish()` with topic `(rescue_capped, admin, token)`
/// and payload `RescueEvent`.
///
/// **Why**: Audit trail for off-chain indexers tracking capped recovery operations.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address executing the rescue.
/// * `payload` - Structured rescue event details (includes `cap` field).
pub fn emit_rescue_capped(env: &Env, admin: &Address, payload: &RescueEvent) {
    env.events().publish(
        (
            event_rescue_capped(env),
            admin.clone(),
            payload.token.clone(),
        ),
        payload.clone(),
    );
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    // ── Topic byte-identity snapshots ─────────────────────────────────────

    /// Every constructor must map to exactly the expected byte string.
    /// Changing any of these would be a breaking change to the on-chain
    /// interface; that is why they are explicitly snapshot-tested.

    #[test]
    fn test_event_initialized_bytes() {
        let env = Env::default();
        assert_eq!(event_initialized(&env), Symbol::new(&env, "initialized"));
    }

    #[test]
    fn test_event_rescue_bytes() {
        let env = Env::default();
        assert_eq!(event_rescue(&env), Symbol::new(&env, "rescue"));
    }

    #[test]
    fn test_event_rescue_capped_bytes() {
        let env = Env::default();
        assert_eq!(
            event_rescue_capped(&env),
            Symbol::new(&env, "rescue_capped")
        );
    }

    // ── Structured payload tests ──────────────────────────────────────────

    #[test]
    fn test_rescue_event_payload_is_versioned() {
        let env = Env::default();
        let token = Address::generate(&env);
        let to = Address::generate(&env);

        let payload = RescueEvent::new(&env, token.clone(), to.clone(), 42, RescueCap::None, 42);

        assert_eq!(payload.version, RESCUE_EVENT_VERSION);
        assert_eq!(payload.token, token);
        assert_eq!(payload.to, to);
        assert_eq!(payload.amount, 42);
        assert_eq!(payload.cap, RescueCap::None);
        assert_eq!(payload.cumulative_rescued, 42);
        assert_eq!(payload.ledger_sequence, env.ledger().sequence());
        assert_eq!(payload.timestamp, env.ledger().timestamp());
    }

    #[test]
    fn test_rescue_capped_payload_includes_cap() {
        let env = Env::default();
        let token = Address::generate(&env);
        let to = Address::generate(&env);

        let payload = RescueEvent::new(
            &env,
            token.clone(),
            to.clone(),
            500,
            RescueCap::Some(1000),
            500,
        );

        assert_eq!(payload.amount, 500);
        assert_eq!(payload.cap, RescueCap::Some(1000));
        assert_eq!(payload.cumulative_rescued, 500);
    }

    #[test]
    fn test_rescue_event_captures_ledger_context() {
        let env = Env::default();
        let token = Address::generate(&env);
        let to = Address::generate(&env);

        let seq_before = env.ledger().sequence();
        let ts_before = env.ledger().timestamp();

        let payload = RescueEvent::new(&env, token, to, 100, RescueCap::None, 100);

        assert_eq!(payload.ledger_sequence, seq_before);
        assert_eq!(payload.timestamp, ts_before);
    }
}
