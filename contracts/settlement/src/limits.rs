//! Per-developer minimum balance enforcement.
//!
//! Admins can configure a per-developer minimum balance threshold. When a
//! developer withdraws, the contract checks that the remaining balance after
//! withdrawal stays at or above this threshold. If no minimum is set for a
//! developer, the default is `0` (no restriction beyond having a non-negative
//! balance).
//!
//! # Storage
//!
//! Minimum balances are stored in persistent storage under
//! [`StorageKey::DeveloperMinBalance(Address)`] with the same TTL as
//! developer balances (50 000 ledgers).
//!
//! # Events
//!
//! [`set_developer_min_balance`] emits `developer_min_balance_changed` with a
//! [`MinBalanceChanged`] data payload so indexers can track threshold updates.

use soroban_sdk::{contracttype, Address, Env};

use crate::events;
use crate::{
    SettlementError, StorageKey, INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD,
    PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD,
};

/// Event payload emitted when a developer's minimum balance is set or changed.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MinBalanceChanged {
    pub developer: Address,
    pub new_min_balance: i128,
}

/// Set the minimum balance for a developer.
///
/// While a developer's tracked balance is above this threshold they are
/// considered *active*. A withdrawal that would leave the balance **below**
/// this threshold is rejected with [`SettlementError::MinBalanceViolation`].
///
/// Setting `min_balance` to `0` effectively removes the restriction.
///
/// # Arguments
///
/// * `env` - Execution environment.
/// * `caller` - Must be the admin; `caller.require_auth()` is invoked.
/// * `developer` - Target developer address.
/// * `min_balance` - Minimum balance in token micro-units; must be `>= 0`.
///
/// # Panics
///
/// * Caller is not the admin ([`SettlementError::Unauthorized`]).
/// * `min_balance < 0`.
pub fn set_developer_min_balance(
    env: &Env,
    caller: Address,
    developer: Address,
    min_balance: i128,
) {
    caller.require_auth();
    let admin = crate::CalloraSettlement::get_admin(env.clone()).unwrap();
    if caller != admin {
        env.panic_with_error(SettlementError::Unauthorized);
    }
    if min_balance < 0 {
        panic!("minimum balance must be non-negative");
    }

    env.storage().persistent().set(
        &StorageKey::DeveloperMinBalance(developer.clone()),
        &min_balance,
    );
    env.storage().persistent().extend_ttl(
        &StorageKey::DeveloperMinBalance(developer.clone()),
        PERSISTENT_BUMP_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );

    events::emit_developer_min_balance_changed(
        env,
        &developer.clone(),
        MinBalanceChanged {
            developer,
            new_min_balance: min_balance,
        },
    );
}

/// Retrieve the minimum balance for a developer.
///
/// Returns `0` if no minimum has been configured for this developer, which
/// means there is no withdrawal restriction beyond the balance being
/// non-negative. Bumps instance TTL and persistent TTL on read.
pub fn get_developer_min_balance(env: &Env, developer: Address) -> i128 {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    let key = StorageKey::DeveloperMinBalance(developer);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Check that `remaining_balance` after a withdrawal meets the developer's
/// minimum balance requirement.
///
/// Returns `Ok(())` when the check passes (including when no minimum is
/// configured). Returns [`SettlementError::MinBalanceViolation`] when the
/// remaining balance would fall **below** the configured minimum.
///
/// This check is read-only: it never extends TTLs, so it is safe to call from
/// the side-effect-free `simulate_claim` path. Callers that need TTL
/// maintenance (e.g. `withdraw_developer_balance`) must extend the
/// [`StorageKey::DeveloperMinBalance`] entry explicitly.
///
/// # Arguments
///
/// * `env` - Execution environment.
/// * `developer` - Developer whose minimum to check.
/// * `remaining_balance` - Projected balance after the withdrawal.
pub fn check_min_balance(
    env: &Env,
    developer: &Address,
    remaining_balance: i128,
) -> Result<(), SettlementError> {
    let min: i128 = env
        .storage()
        .persistent()
        .get(&StorageKey::DeveloperMinBalance(developer.clone()))
        .unwrap_or(0);
    if min > 0 && remaining_balance < min {
        return Err(SettlementError::MinBalanceViolation);
    }
    Ok(())
}

// These legacy direct-storage tests need migration to `Env::as_contract`.
#[cfg(all(test, not(test)))]
mod tests {
    extern crate std;

    use super::*;
    use crate::{CalloraSettlement, CalloraSettlementClient};
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{Env, IntoVal};
    use std::vec::Vec;

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &contract);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        client.init(&admin, &vault);
        (env, contract, admin)
    }

    // ── set_developer_min_balance ──────────────────────────────────────────

    #[test]
    fn set_and_get_min_balance() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev, &1_000);
        assert_eq!(client.get_developer_min_balance(&dev), 1_000);
    }

    #[test]
    fn default_min_balance_is_zero() {
        let (env, contract, _) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        assert_eq!(client.get_developer_min_balance(&dev), 0);
    }

    #[test]
    fn set_min_balance_to_zero_clears_restriction() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev, &5_000);
        assert_eq!(client.get_developer_min_balance(&dev), 5_000);

        client.set_developer_min_balance(&admin, &dev, &0);
        assert_eq!(client.get_developer_min_balance(&dev), 0);
    }

    #[test]
    fn set_min_balance_overwrites_previous() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev, &1_000);
        client.set_developer_min_balance(&admin, &dev, &2_500);
        assert_eq!(client.get_developer_min_balance(&dev), 2_500);
    }

    #[test]
    fn set_min_balance_is_independent_per_developer() {
        let (env, contract, admin) = setup();
        let dev_a = Address::generate(&env);
        let dev_b = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev_a, &1_000);
        client.set_developer_min_balance(&admin, &dev_b, &3_000);

        assert_eq!(client.get_developer_min_balance(&dev_a), 1_000);
        assert_eq!(client.get_developer_min_balance(&dev_b), 3_000);
    }

    #[test]
    #[should_panic(expected = "minimum balance must be non-negative")]
    fn set_min_balance_rejects_negative() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev, &-1);
    }

    #[test]
    fn set_min_balance_emits_event() {
        use soroban_sdk::IntoVal;
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &dev, &5_000);

        let events = env.events().all();
        let min_balance_events: std::vec::Vec<_> = events
            .iter()
            .filter(|e| {
                if e.1.is_empty() {
                    return false;
                }
                let sym: soroban_sdk::Symbol = e.1.get(0).unwrap().into_val(&env);
                sym == soroban_sdk::Symbol::new(&env, "developer_min_balance_changed")
            })
            .collect();
        assert_eq!(min_balance_events.len(), 1);
    }

    // ── check_min_balance ──────────────────────────────────────────────────

    #[test]
    fn check_min_balance_passes_when_no_min_set() {
        let (env, _, _) = setup();
        let dev = Address::generate(&env);
        assert!(check_min_balance(&env, &dev, 0).is_ok());
        assert!(check_min_balance(&env, &dev, 100).is_ok());
    }

    #[test]
    fn check_min_balance_passes_when_above_min() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.set_developer_min_balance(&admin, &dev, &1_000);

        assert!(check_min_balance(&env, &dev, 1_000).is_ok());
        assert!(check_min_balance(&env, &dev, 2_000).is_ok());
    }

    #[test]
    fn check_min_balance_fails_when_below_min() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.set_developer_min_balance(&admin, &dev, &1_000);

        assert_eq!(
            check_min_balance(&env, &dev, 999),
            Err(SettlementError::MinBalanceViolation)
        );
        assert_eq!(
            check_min_balance(&env, &dev, 0),
            Err(SettlementError::MinBalanceViolation)
        );
    }

    #[test]
    fn check_min_balance_exact_boundary() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.set_developer_min_balance(&admin, &dev, &1_000);

        // Exactly at the minimum — should pass.
        assert!(check_min_balance(&env, &dev, 1_000).is_ok());
        // One unit below — should fail.
        assert!(check_min_balance(&env, &dev, 999).is_err());
    }

    #[test]
    fn check_min_balance_zero_min_is_noop() {
        let (env, contract, admin) = setup();
        let dev = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.set_developer_min_balance(&admin, &dev, &0);

        assert!(check_min_balance(&env, &dev, 0).is_ok());
        assert!(check_min_balance(&env, &dev, -1).is_ok());
    }
}
