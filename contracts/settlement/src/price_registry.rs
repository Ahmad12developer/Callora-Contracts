//! Per-admin write rate-limited price registry for the settlement contract.
//!
//! This module provides a simple on-chain price registry where the admin can
//! set and remove prices for offering identifiers. Every write operation
//! (`set_price`, `remove_price`) is subject to a per-admin rate limit that
//! prevents any single admin from updating prices more frequently than
//! [`MIN_WRITE_INTERVAL`] ledgers.
//!
//! # Rate Limit
//!
//! The rate limit is enforced by tracking the last ledger sequence in which
//! each admin wrote to the registry. The storage key is scoped per admin
//! address (`StorageKey::PriceRegistryLastWrite(Address)`), so one admin's
//! rate limit does not affect another admin.
//!
//! | Constant | Value | Meaning |
//! |----------|-------|---------|
//! | `MIN_WRITE_INTERVAL` | 10 ledgers | Minimum ledgers between consecutive writes by the same admin |
//!
//! # Errors
//!
//! Returns [`crate::SettlementError::WriteRateLimitExceeded`] when an admin
//! attempts to write before the interval has elapsed.

use soroban_sdk::{Address, Env, String};

use crate::events::{emit_price_removed, emit_price_set};
use crate::{CalloraSettlement, SettlementError, StorageKey};

/// Minimum number of ledgers that must pass between consecutive price writes
/// by the same admin.
///
/// On Stellar, one ledger corresponds to approximately 5 seconds, so
/// `MIN_WRITE_INTERVAL = 10` represents a ~50-second minimum interval.
pub const MIN_WRITE_INTERVAL: u32 = 10;

/// Set the price for an offering.
///
/// # Access Control
/// The caller must be the current admin and must authorize the call via
/// `require_auth`.
///
/// # Rate Limiting
/// If the admin's previous write occurred fewer than [`MIN_WRITE_INTERVAL`]
/// ledgers ago, the function returns [`SettlementError::WriteRateLimitExceeded`].
///
/// # Arguments
/// * `env` - Execution environment.
/// * `caller` - Must be the admin; `caller.require_auth()` is invoked.
/// * `offering_id` - Identifier for the offering whose price is being set.
/// * `price` - Price value as a string.
///
/// # Panics
/// * [`SettlementError::Unauthorized`] — caller is not the admin.
/// * [`SettlementError::WriteRateLimitExceeded`] — write interval not elapsed.
pub fn set_price(env: &Env, caller: Address, offering_id: String, price: String) {
    caller.require_auth();
    let admin =
        CalloraSettlement::get_admin(env.clone()).unwrap_or_else(|e| env.panic_with_error(e));
    if caller != admin {
        env.panic_with_error(SettlementError::Unauthorized);
    }
    enforce_write_rate_limit(env, &caller);
    let key = StorageKey::Price(offering_id.clone());
    let old = env.storage().persistent().get::<_, String>(&key);
    env.storage().persistent().set(&key, &price);
    emit_price_set(env, &offering_id, old, &price);
    update_last_write_ledger(env, &caller);
}

/// Remove the price for an offering.
///
/// # Access Control
/// The caller must be the current admin and must authorize the call via
/// `require_auth`.
///
/// # Rate Limiting
/// If the admin's previous write occurred fewer than [`MIN_WRITE_INTERVAL`]
/// ledgers ago, the function returns [`SettlementError::WriteRateLimitExceeded`].
///
/// # Arguments
/// * `env` - Execution environment.
/// * `caller` - Must be the admin; `caller.require_auth()` is invoked.
/// * `offering_id` - Identifier for the offering whose price is being removed.
///
/// # Panics
/// * [`SettlementError::Unauthorized`] — caller is not the admin.
/// * [`SettlementError::WriteRateLimitExceeded`] — write interval not elapsed.
pub fn remove_price(env: &Env, caller: Address, offering_id: String) {
    caller.require_auth();
    let admin =
        CalloraSettlement::get_admin(env.clone()).unwrap_or_else(|e| env.panic_with_error(e));
    if caller != admin {
        env.panic_with_error(SettlementError::Unauthorized);
    }
    enforce_write_rate_limit(env, &caller);
    let key = StorageKey::Price(offering_id.clone());
    if let Some(old) = env.storage().persistent().get::<_, String>(&key) {
        env.storage().persistent().remove(&key);
        emit_price_removed(env, &offering_id, &old);
    }
    update_last_write_ledger(env, &caller);
}

/// Get the price for an offering.
///
/// Returns `None` if no price has been set for the given offering ID.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `offering_id` - Identifier for the offering.
pub fn get_price(env: &Env, offering_id: String) -> Option<String> {
    let key = StorageKey::Price(offering_id);
    env.storage().persistent().get(&key)
}

/// Enforce the per-admin write rate limit.
///
/// Reads the admin's last write ledger from persistent storage and compares
/// it against the current ledger sequence. If the difference is less than
/// [`MIN_WRITE_INTERVAL`], panics with [`SettlementError::WriteRateLimitExceeded`].
///
/// # Arguments
/// * `env` - Execution environment.
/// * `admin` - Admin address whose rate limit is being checked.
fn enforce_write_rate_limit(env: &Env, admin: &Address) {
    let current_ledger = env.ledger().sequence();
    let last_write_key = StorageKey::PriceRegistryLastWrite(admin.clone());
    let last_write_ledger: u32 = env.storage().persistent().get(&last_write_key).unwrap_or(0);
    if last_write_ledger == 0 {
        return;
    }
    let elapsed = current_ledger.saturating_sub(last_write_ledger);
    if elapsed < MIN_WRITE_INTERVAL {
        env.panic_with_error(SettlementError::WriteRateLimitExceeded);
    }
}

/// Update the admin's last write ledger to the current ledger sequence.
///
/// Writes the current ledger sequence to persistent storage under
/// [`StorageKey::PriceRegistryLastWrite(admin)`](StorageKey::PriceRegistryLastWrite)
/// and extends the TTL.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `admin` - Admin address whose last write ledger is being updated.
fn update_last_write_ledger(env: &Env, admin: &Address) {
    let current_ledger = env.ledger().sequence();
    let key = StorageKey::PriceRegistryLastWrite(admin.clone());
    env.storage().persistent().set(&key, &current_ledger);
    env.storage().persistent().extend_ttl(
        &key,
        crate::types::PERSISTENT_BUMP_THRESHOLD,
        crate::types::PERSISTENT_BUMP_AMOUNT,
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError};
    use soroban_sdk::Env;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_sequence_number(100);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        (env, addr, admin)
    }

    #[test]
    fn set_price_succeeds_on_first_call() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        let price = client.try_get_price(&String::from_str(&env, "offer1"));
        match price {
            Ok(Ok(Some(p))) => assert_eq!(p, String::from_str(&env, "100")),
            _ => panic!("expected price to be set"),
        }
    }

    #[test]
    fn set_price_succeeds_after_interval() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        // Advance ledger by exactly MIN_WRITE_INTERVAL
        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer2"),
            &String::from_str(&env, "200"),
        );

        let price1 = client.try_get_price(&String::from_str(&env, "offer1"));
        let price2 = client.try_get_price(&String::from_str(&env, "offer2"));
        match (price1, price2) {
            (Ok(Ok(Some(p1))), Ok(Ok(Some(p2)))) => {
                assert_eq!(p1, String::from_str(&env, "100"));
                assert_eq!(p2, String::from_str(&env, "200"));
            }
            _ => panic!("expected both prices to be set"),
        }
    }

    #[test]
    fn set_price_fails_when_rate_limit_exceeded() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        // Try again before the interval has passed
        let result = client.try_set_price(
            &admin,
            &String::from_str(&env, "offer2"),
            &String::from_str(&env, "200"),
        );
        assert!(is_write_rate_limit_error(result));
    }

    #[test]
    fn rate_limit_is_per_admin() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer2"),
            &String::from_str(&env, "200"),
        );

        let price1 = client.try_get_price(&String::from_str(&env, "offer1"));
        let price2 = client.try_get_price(&String::from_str(&env, "offer2"));
        match (price1, price2) {
            (Ok(Ok(Some(p1))), Ok(Ok(Some(p2)))) => {
                assert_eq!(p1, String::from_str(&env, "100"));
                assert_eq!(p2, String::from_str(&env, "200"));
            }
            _ => panic!("expected both prices to be set"),
        }
    }

    #[test]
    fn set_price_requires_auth() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let unauthorized = Address::generate(&env);

        env.set_auths(&[]);
        let result = client.try_set_price(
            &unauthorized,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn remove_price_succeeds() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );
        let before = client.try_get_price(&String::from_str(&env, "offer1"));
        match before {
            Ok(Ok(Some(p))) => assert_eq!(p, String::from_str(&env, "100")),
            _ => panic!("expected price to be set before removal"),
        }

        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32);
        client.remove_price(&admin, &String::from_str(&env, "offer1"));
        let after = client.try_get_price(&String::from_str(&env, "offer1"));
        match after {
            Ok(Ok(None)) => {}
            _ => panic!("expected price to be None after removal"),
        }
    }

    #[test]
    fn remove_price_fails_when_rate_limit_exceeded() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        let result = client.try_remove_price(&admin, &String::from_str(&env, "offer1"));
        assert!(is_write_rate_limit_error(result));
    }

    #[test]
    fn write_at_exact_interval_boundary_succeeds() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        // Advance ledger by exactly MIN_WRITE_INTERVAL — should succeed
        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32);

        let result = client.try_set_price(
            &admin,
            &String::from_str(&env, "offer2"),
            &String::from_str(&env, "200"),
        );
        assert!(
            result.is_ok(),
            "write at exact interval boundary should succeed"
        );
    }

    #[test]
    fn write_one_ledger_before_interval_fails() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );

        // Advance by MIN_WRITE_INTERVAL - 1 — should fail
        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32 - 1);

        let result = client.try_set_price(
            &admin,
            &String::from_str(&env, "offer2"),
            &String::from_str(&env, "200"),
        );
        assert!(is_write_rate_limit_error(result));
    }

    #[test]
    fn remove_price_requires_auth() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let unauthorized = Address::generate(&env);

        env.set_auths(&[]);
        let result = client.try_remove_price(&unauthorized, &String::from_str(&env, "offer1"));
        assert!(result.is_err());
    }

    #[test]
    fn get_price_returns_none_for_unknown_offering() {
        let (env, contract, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        let result = client.try_get_price(&String::from_str(&env, "nonexistent"));
        match result {
            Ok(Ok(None)) => {}
            _ => panic!("expected Ok(Ok(None)), got {:?}", result),
        }
    }

    #[test]
    fn set_price_overwrites_previous_price() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );
        env.ledger()
            .set_sequence_number(env.ledger().sequence() + MIN_WRITE_INTERVAL as u32);
        client.set_price(
            &admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "200"),
        );

        let price = client.try_get_price(&String::from_str(&env, "offer1"));
        match price {
            Ok(Ok(Some(p))) => assert_eq!(p, String::from_str(&env, "200")),
            _ => panic!("expected overwritten price"),
        }
    }

    #[test]
    fn set_price_unauthorized_non_admin() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let non_admin = Address::generate(&env);

        let result = client.try_set_price(
            &non_admin,
            &String::from_str(&env, "offer1"),
            &String::from_str(&env, "100"),
        );
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn remove_price_unauthorized_non_admin() {
        let (env, contract, admin) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let non_admin = Address::generate(&env);

        let result = client.try_remove_price(&non_admin, &String::from_str(&env, "offer1"));
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    fn is_write_rate_limit_error<V, CE: Into<soroban_sdk::Error>, E: Into<soroban_sdk::Error>>(
        result: Result<Result<V, CE>, Result<E, soroban_sdk::InvokeError>>,
    ) -> bool {
        match result {
            Err(Ok(e)) => e.into().get_code() == SettlementError::WriteRateLimitExceeded as u32,
            _ => false,
        }
    }

    fn is_error<V, CE: Into<soroban_sdk::Error>, E: Into<soroban_sdk::Error>>(
        result: Result<Result<V, CE>, Result<E, soroban_sdk::InvokeError>>,
        expected: SettlementError,
    ) -> bool {
        let expected_code = expected as u32;
        match result {
            Err(Ok(e)) => e.into().get_code() == expected_code,
            _ => false,
        }
    }
}
