#![no_std]
//! Callora rescue contract — overflow-safe token recovery.
//!
//! Provides admin-gated entrypoints to recover tokens accidentally sent to
//! a Callora contract.  **All** arithmetic uses `checked_*` operations;
//! there are no raw `+`, `-`, or `*` in production paths.
//!
//! # Entrypoints
//! | Function | Auth | Description |
//! |----------|------|-------------|
//! | [`CalloraRescue::init`]           | admin | One-time initialisation |
//! | [`CalloraRescue::rescue`]         | admin | Transfer `amount` of any token to `to` |
//! | [`CalloraRescue::rescue_capped`]  | admin | Like `rescue` but enforces a per-call cap |
//! | [`CalloraRescue::total_rescued`]  | —     | View: cumulative rescued amount |
//! | [`CalloraRescue::get_admin`]      | —     | View: current admin address |
//!
//! # Security
//! - `require_auth` on every state-changing entrypoint.
//! - No `unwrap()` in production paths; all fallible ops return `Result`.
//! - Overflow-safe: `checked_add`, `checked_sub` with explicit error variants.

mod events;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, Env};

// ---------------------------------------------------------------------------
// Error catalogue
// ---------------------------------------------------------------------------

/// Errors returned by the rescue contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RescueError {
    /// Contract has not been initialised yet.
    NotInitialized = 1,
    /// `init` was already called.
    AlreadyInitialized = 2,
    /// Caller is not the stored admin.
    Unauthorized = 3,
    /// `amount` must be strictly positive.
    AmountNotPositive = 4,
    /// On-ledger balance is too low to satisfy the requested rescue.
    InsufficientBalance = 5,
    /// Arithmetic overflow detected in a checked operation.
    Overflow = 6,
    /// Requested amount exceeds the configured per-call rescue cap.
    ExceedsCap = 7,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// On-chain data keys for the rescue contract.
#[contracttype]
pub enum DataKey {
    /// Stored admin [`Address`] (instance storage).
    Admin,
    /// Running total of all tokens rescued across all calls (instance storage).
    TotalRescued,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Callora rescue contract.
///
/// Admin-gated, overflow-safe token recovery surface.
#[contract]
pub struct CalloraRescue;

#[contractimpl]
impl CalloraRescue {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialise the rescue contract with `admin` as the sole authorised
    /// rescuer.
    ///
    /// # Errors
    /// - [`RescueError::AlreadyInitialized`] if called more than once.
    pub fn init(env: Env, admin: Address) -> Result<(), RescueError> {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::Admin) {
            return Err(RescueError::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        // Initialise total rescued counter to zero (overflow-safe baseline).
        env.storage()
            .instance()
            .set(&DataKey::TotalRescued, &0_i128);

        events::emit_initialized(&env, &admin);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // State-changing entrypoints
    // -----------------------------------------------------------------------

    /// Transfer `amount` units of `token` from this contract to `to`.
    ///
    /// The on-ledger balance of `token` held by this contract must be at
    /// least `amount`.  The cumulative rescued counter is updated with
    /// overflow protection.
    ///
    /// # Auth
    /// Requires `admin.require_auth()`.
    ///
    /// # Errors
    /// - [`RescueError::NotInitialized`] if `init` was not called.
    /// - [`RescueError::Unauthorized`] if `admin` does not match stored admin.
    /// - [`RescueError::AmountNotPositive`] if `amount <= 0`.
    /// - [`RescueError::InsufficientBalance`] if on-ledger balance < amount.
    /// - [`RescueError::Overflow`] if cumulative counter would overflow i128.
    pub fn rescue(
        env: Env,
        admin: Address,
        token: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), RescueError> {
        admin.require_auth();

        Self::assert_admin(&env, &admin)?;

        if amount <= 0 {
            return Err(RescueError::AmountNotPositive);
        }

        let token_client = token::Client::new(&env, &token);
        let vault = env.current_contract_address();
        let on_ledger = token_client.balance(&vault);

        if on_ledger < amount {
            return Err(RescueError::InsufficientBalance);
        }

        // Overflow-safe transfer execution.
        token_client.transfer(&vault, &to, &amount);

        // Update running total with overflow-safe addition.
        let prev: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalRescued)
            .unwrap_or(0_i128);
        let next = prev.checked_add(amount).ok_or(RescueError::Overflow)?;
        env.storage().instance().set(&DataKey::TotalRescued, &next);

        let payload = events::RescueEvent::new(
            &env,
            token.clone(),
            to.clone(),
            amount,
            events::RescueCap::None,
            next,
        );
        events::emit_rescue(&env, &admin, &payload);

        Ok(())
    }

    /// Like [`rescue`] but also enforces that `amount <= cap`.
    ///
    /// Useful when the caller wants an extra on-chain guard against
    /// oversized individual rescue operations.
    ///
    /// # Auth
    /// Requires `admin.require_auth()`.
    ///
    /// # Errors
    /// All errors from [`rescue`], plus:
    /// - [`RescueError::ExceedsCap`] if `amount > cap`.
    pub fn rescue_capped(
        env: Env,
        admin: Address,
        token: Address,
        to: Address,
        amount: i128,
        cap: i128,
    ) -> Result<(), RescueError> {
        admin.require_auth();

        Self::assert_admin(&env, &admin)?;

        if amount <= 0 {
            return Err(RescueError::AmountNotPositive);
        }

        // cap must also be positive; use checked comparison.
        if cap <= 0 || amount > cap {
            return Err(RescueError::ExceedsCap);
        }

        let token_client = token::Client::new(&env, &token);
        let vault = env.current_contract_address();
        let on_ledger = token_client.balance(&vault);

        if on_ledger < amount {
            return Err(RescueError::InsufficientBalance);
        }

        token_client.transfer(&vault, &to, &amount);

        let prev: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalRescued)
            .unwrap_or(0_i128);
        let next = prev.checked_add(amount).ok_or(RescueError::Overflow)?;
        env.storage().instance().set(&DataKey::TotalRescued, &next);

        let payload = events::RescueEvent::new(
            &env,
            token.clone(),
            to.clone(),
            amount,
            events::RescueCap::Some(cap),
            next,
        );
        events::emit_rescue_capped(&env, &admin, &payload);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only views (no auth required)
    // -----------------------------------------------------------------------

    /// Return the cumulative amount rescued across all calls.
    ///
    /// Returns `0` before `init` is called (no storage entry yet).
    pub fn total_rescued(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalRescued)
            .unwrap_or(0_i128)
    }

    /// Return the stored admin address.
    ///
    /// # Errors
    /// - [`RescueError::NotInitialized`] if `init` was not called.
    pub fn get_admin(env: Env) -> Result<Address, RescueError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(RescueError::NotInitialized)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Assert that `caller` matches the stored admin.
    fn assert_admin(env: &Env, caller: &Address) -> Result<(), RescueError> {
        let stored: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(RescueError::NotInitialized)?;
        if caller != &stored {
            return Err(RescueError::Unauthorized);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(CalloraRescue, ());
        let admin = Address::generate(&env);
        (env, contract_id, admin)
    }

    fn client<'a>(env: &'a Env, contract_id: &'a Address) -> CalloraRescueClient<'a> {
        CalloraRescueClient::new(env, contract_id)
    }

    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    #[test]
    fn test_init_sets_admin() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        assert_eq!(c.get_admin(), admin);
    }

    #[test]
    fn test_double_init_returns_already_initialized() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        assert_eq!(
            c.try_init(&admin).unwrap_err().unwrap(),
            RescueError::AlreadyInitialized
        );
    }

    #[test]
    fn test_get_admin_before_init_returns_not_initialized() {
        let (env, cid, _admin) = setup();
        let c = client(&env, &cid);
        assert_eq!(
            c.try_get_admin().unwrap_err().unwrap(),
            RescueError::NotInitialized
        );
    }

    // -----------------------------------------------------------------------
    // total_rescued view
    // -----------------------------------------------------------------------

    #[test]
    fn test_total_rescued_starts_at_zero() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        assert_eq!(c.total_rescued(), 0);
    }

    // -----------------------------------------------------------------------
    // Validation: amount must be positive
    // -----------------------------------------------------------------------

    #[test]
    fn test_rescue_zero_amount_returns_amount_not_positive() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue(&admin, &token, &to, &0_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::AmountNotPositive
        );
    }

    #[test]
    fn test_rescue_negative_amount_returns_amount_not_positive() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue(&admin, &token, &to, &-1_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::AmountNotPositive
        );
    }

    // -----------------------------------------------------------------------
    // Validation: cap enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn test_rescue_capped_exceeds_cap_returns_error() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue_capped(&admin, &token, &to, &1001_i128, &1000_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::ExceedsCap
        );
    }

    #[test]
    fn test_rescue_capped_zero_cap_returns_error() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue_capped(&admin, &token, &to, &1_i128, &0_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::ExceedsCap
        );
    }

    // -----------------------------------------------------------------------
    // Unauthorised access
    // -----------------------------------------------------------------------

    #[test]
    fn test_rescue_wrong_admin_returns_unauthorized() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        c.init(&admin);
        let imposter = Address::generate(&env);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue(&imposter, &token, &to, &1_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::Unauthorized
        );
    }

    // -----------------------------------------------------------------------
    // Not-initialized guard
    // -----------------------------------------------------------------------

    #[test]
    fn test_rescue_before_init_returns_not_initialized() {
        let (env, cid, admin) = setup();
        let c = client(&env, &cid);
        let token = Address::generate(&env);
        let to = Address::generate(&env);
        assert_eq!(
            c.try_rescue(&admin, &token, &to, &1_i128)
                .unwrap_err()
                .unwrap(),
            RescueError::NotInitialized
        );
    }
}

// ---------------------------------------------------------------------------
// Event lifecycle tests
// ---------------------------------------------------------------------------

/// Integration tests that verify actual event emission from rescue
/// contract entrypoints. Each test drives the contract through a public
/// function and inspects `env.events().all()` to assert the correct
/// topic shape and structured payload.
#[cfg(test)]
mod test_events {
    extern crate std;

    use crate::events::{self, RescueCap};
    use crate::{CalloraRescue, CalloraRescueClient};
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

    /// Helper: register a Stellar Asset Contract token, mint `amount` to the
    /// rescue contract, and return the token address.
    fn create_token(
        env: &Env,
        admin: &Address,
        rescue_addr: &Address,
        mint_amount: i128,
    ) -> Address {
        let token_addr = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let token_admin = token::StellarAssetClient::new(env, &token_addr);
        token_admin.mint(rescue_addr, &mint_amount);
        token_addr
    }

    /// `init` must emit exactly one `initialized` event with topic[1]=admin.
    #[test]
    fn test_init_emits_initialized_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let all_events = env.events().all();
        // Find the last event emitted by this contract.
        let event = all_events
            .iter()
            .rev()
            .find(|e| e.0 == cid)
            .expect("expected at least one event from rescue contract");

        let topics = &event.1;
        assert_eq!(topics.len(), 2);
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        let topic1: Address = topics.get(1).unwrap().into_val(&env);
        assert_eq!(topic0, Symbol::new(&env, "initialized"));
        assert_eq!(topic1, admin);

        // Payload is the admin address.
        let data: Address = event.2.into_val(&env);
        assert_eq!(data, admin);
    }

    /// Failed `init` (double-init) must NOT emit an `initialized` event.
    #[test]
    fn test_double_init_emits_no_second_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);
        // Clear events from successful init.
        env.events().all();

        // Double init should fail.
        let result = c.try_init(&admin);
        assert!(result.is_err());

        let all_events = env.events().all();
        let init_events: std::vec::Vec<_> = all_events
            .iter()
            .filter(|e| {
                if e.0 != cid || e.1.is_empty() {
                    return false;
                }
                let t0: Symbol = e.1.get(0).unwrap().into_val(&env);
                t0 == Symbol::new(&env, "initialized")
            })
            .collect();
        assert_eq!(
            init_events.len(),
            0,
            "double init must not emit 'initialized'"
        );
    }

    /// `rescue` emits exactly one `rescue` event with correct topics and
    /// a versioned `RescueEvent` payload.
    #[test]
    fn test_rescue_emits_rescue_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let to = Address::generate(&env);
        let amount: i128 = 500;
        let token_addr = create_token(&env, &admin, &cid, &amount);

        // Clear init event.
        env.events().all();

        c.rescue(&admin, &token_addr, &to, &amount);

        let all_events = env.events().all();
        let event = all_events
            .iter()
            .rev()
            .find(|e| e.0 == cid)
            .expect("expected rescue event");

        let topics = &event.1;
        assert_eq!(topics.len(), 3);
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        let topic1: Address = topics.get(1).unwrap().into_val(&env);
        let topic2: Address = topics.get(2).unwrap().into_val(&env);
        assert_eq!(topic0, Symbol::new(&env, "rescue"));
        assert_eq!(topic1, admin);
        assert_eq!(topic2, token_addr);

        // Payload is a RescueEvent.
        let payload: events::RescueEvent = event.2.into_val(&env);
        assert_eq!(payload.version, events::RESCUE_EVENT_VERSION);
        assert_eq!(payload.token, token_addr);
        assert_eq!(payload.to, to);
        assert_eq!(payload.amount, amount);
        assert_eq!(payload.cap, RescueCap::None);
        assert_eq!(payload.cumulative_rescued, amount);
        assert_eq!(payload.ledger_sequence, env.ledger().sequence());
        assert_eq!(payload.timestamp, env.ledger().timestamp());
    }

    /// `rescue_capped` emits `rescue_capped` event with `cap` populated in
    /// the payload.
    #[test]
    fn test_rescue_capped_emits_rescue_capped_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let to = Address::generate(&env);
        let amount: i128 = 500;
        let cap: i128 = 1000;
        let token_addr = create_token(&env, &admin, &cid, &amount);

        env.events().all();

        c.rescue_capped(&admin, &token_addr, &to, &amount, &cap);

        let all_events = env.events().all();
        let event = all_events
            .iter()
            .rev()
            .find(|e| e.0 == cid)
            .expect("expected rescue_capped event");

        let topics = &event.1;
        assert_eq!(topics.len(), 3);
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        let topic1: Address = topics.get(1).unwrap().into_val(&env);
        let topic2: Address = topics.get(2).unwrap().into_val(&env);
        assert_eq!(topic0, Symbol::new(&env, "rescue_capped"));
        assert_eq!(topic1, admin);
        assert_eq!(topic2, token_addr);

        let payload: events::RescueEvent = event.2.into_val(&env);
        assert_eq!(payload.version, events::RESCUE_EVENT_VERSION);
        assert_eq!(payload.amount, amount);
        assert_eq!(payload.cap, RescueCap::Some(cap));
        assert_eq!(payload.cumulative_rescued, amount);
    }

    /// `cumulative_rescued` must accumulate across multiple rescue calls.
    #[test]
    fn test_multiple_rescues_accumulate_cumulative_total() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let to = Address::generate(&env);
        let token_addr = create_token(&env, &admin, &cid, &1000);

        env.events().all();

        // First rescue: 300
        c.rescue(&admin, &token_addr, &to, &300_i128);
        // Second rescue: 200
        c.rescue(&admin, &token_addr, &to, &200_i128);

        let all_events = env.events().all();
        let rescue_events: std::vec::Vec<_> = all_events
            .iter()
            .filter(|e| {
                if e.0 != cid || e.1.is_empty() {
                    return false;
                }
                let t0: Symbol = e.1.get(0).unwrap().into_val(&env);
                t0 == Symbol::new(&env, "rescue")
            })
            .collect();

        assert_eq!(rescue_events.len(), 2);

        let payload1: events::RescueEvent = rescue_events[0].2.into_val(&env);
        assert_eq!(payload1.cumulative_rescued, 300);

        let payload2: events::RescueEvent = rescue_events[1].2.into_val(&env);
        assert_eq!(payload2.cumulative_rescued, 500);
    }

    /// Failed rescue (invalid amount) must NOT emit a `rescue` event.
    #[test]
    fn test_failed_rescue_emits_no_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let token = Address::generate(&env);
        let to = Address::generate(&env);

        // Clear init event.
        env.events().all();

        let result = c.try_rescue(&admin, &token, &to, &0_i128);
        assert!(result.is_err());

        let all_events = env.events().all();
        let rescue_events: std::vec::Vec<_> = all_events
            .iter()
            .filter(|e| {
                if e.0 != cid || e.1.is_empty() {
                    return false;
                }
                let t0: Symbol = e.1.get(0).unwrap().into_val(&env);
                t0 == Symbol::new(&env, "rescue")
            })
            .collect();
        assert_eq!(
            rescue_events.len(),
            0,
            "failed rescue must not emit 'rescue'"
        );
    }

    /// Failed `rescue_capped` (amount exceeds cap) must NOT emit an event.
    #[test]
    fn test_failed_rescue_capped_emits_no_event() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let cid = env.register(CalloraRescue, ());
        let c = CalloraRescueClient::new(&env, &cid);

        c.init(&admin);

        let token_addr = create_token(&env, &admin, &cid, &500);
        let to = Address::generate(&env);

        // Clear init event.
        env.events().all();

        // Amount 500 > cap 400 should fail.
        let result = c.try_rescue_capped(&admin, &token_addr, &to, &500_i128, &400_i128);
        assert!(result.is_err());

        let all_events = env.events().all();
        let capped_events: std::vec::Vec<_> = all_events
            .iter()
            .filter(|e| {
                if e.0 != cid || e.1.is_empty() {
                    return false;
                }
                let t0: Symbol = e.1.get(0).unwrap().into_val(&env);
                t0 == Symbol::new(&env, "rescue_capped")
            })
            .collect();
        assert_eq!(
            capped_events.len(),
            0,
            "failed rescue_capped must not emit 'rescue_capped'"
        );
    }
}
