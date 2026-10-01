//! Ledger-sequence high-water-mark replay protection.
//!
//! Stores the last-applied ledger sequence per developer (and a global-pool
//! HWM) and rejects any settlement claim at a lower or equal sequence.
//!
//! # Storage
//!
//! * `StorageKey::HighWaterMark(Address)` — persistent storage; TTL bumped on
//!   each write to match the developer-balance lifecycle.
//! * `StorageKey::PoolHighWaterMark` — instance storage (lives as long as the
//!   contract instance).
//!
//! # Reorg threat model
//!
//! If a chain reorg replays a settlement transaction the vault-provided
//! `ledger_seq` will be the same (or lower) than the stored HWM.  The guard
//! rejects the call, preventing a double credit.

use soroban_sdk::{Address, Env};

use crate::{SettlementError, StorageKey};

/// Persistent TTL parameters – kept in lockstep with the developer-balance
/// entry TTL (#1131). Aliases of the shared constants so a HWM can never
/// archive before the balance it protects; losing a HWM would reopen replay
/// of old ledger sequences. Kept as `pub` names for API compatibility.
pub const HWM_LIVE: u32 = crate::types::PERSISTENT_BUMP_AMOUNT;
pub const HWM_THRESHOLD: u32 = crate::types::PERSISTENT_BUMP_THRESHOLD;

/// Validate a settlement claim for `developer`.
///
/// Returns [`SettlementError::ReplayDetected`] when `ledger_seq <= stored`
/// high-water mark.  On success the stored mark is raised to `ledger_seq` and
/// the persistent TTL is extended.
pub fn check_developer(
    env: &Env,
    developer: &Address,
    ledger_seq: u32,
) -> Result<(), SettlementError> {
    let key = StorageKey::HighWaterMark(developer.clone());
    let stored: u32 = env.storage().persistent().get(&key).unwrap_or(0);

    if ledger_seq <= stored {
        return Err(SettlementError::ReplayDetected);
    }

    env.storage().persistent().set(&key, &ledger_seq);
    env.storage()
        .persistent()
        .extend_ttl(&key, HWM_THRESHOLD, HWM_LIVE);

    Ok(())
}

/// Validate a settlement claim for the global pool.
///
/// Behaviour is identical to [`check_developer`] but the HWM is stored in
/// instance storage (no per-developer key).
pub fn check_pool(env: &Env, ledger_seq: u32) -> Result<(), SettlementError> {
    let key = StorageKey::PoolHighWaterMark;
    let stored: u32 = env.storage().instance().get(&key).unwrap_or(0);

    if ledger_seq <= stored {
        return Err(SettlementError::ReplayDetected);
    }

    env.storage().instance().set(&key, &ledger_seq);

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{CalloraSettlement, CalloraSettlementClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        (env, addr, vault, admin)
    }

    fn dev(env: &Env) -> Address {
        Address::generate(env)
    }

    fn token(env: &Env) -> Address {
        Address::generate(env)
    }

    /// Normal progression – strictly increasing sequences pass.
    #[test]
    fn test_hwm_accepts_strictly_increasing() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &100i128, &false, &Some(d.clone()), &t, &10u32);
        assert_eq!(client.get_developer_balance(&d, &t), 100);

        client.receive_payment(&vault, &200i128, &false, &Some(d.clone()), &t, &20u32);
        assert_eq!(client.get_developer_balance(&d, &t), 300);
    }

    /// Equal ledger_seq is rejected.
    #[test]
    fn test_hwm_rejects_equal_seq() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &100i128, &false, &Some(d.clone()), &t, &10u32);

        let result =
            client.try_receive_payment(&vault, &50i128, &false, &Some(d.clone()), &t, &10u32);
        assert!(result.is_err(), "equal ledger_seq should be rejected");
    }

    /// Lower ledger_seq is rejected.
    #[test]
    fn test_hwm_rejects_lower_seq() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &100i128, &false, &Some(d.clone()), &t, &20u32);

        let result =
            client.try_receive_payment(&vault, &50i128, &false, &Some(d.clone()), &t, &5u32);
        assert!(result.is_err(), "lower ledger_seq should be rejected");
    }

    /// Different developers have independent HWMs.
    #[test]
    fn test_hwm_independent_per_developer() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d1 = dev(&env);
        let d2 = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &100i128, &false, &Some(d1.clone()), &t, &10u32);
        client.receive_payment(&vault, &200i128, &false, &Some(d2.clone()), &t, &10u32);

        assert_eq!(client.get_developer_balance(&d1, &t), 100);
        assert_eq!(client.get_developer_balance(&d2, &t), 200);
    }

    /// Pool payments use their own HWM.
    #[test]
    fn test_hwm_pool_independent() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        // Pool seq advances to 50
        client.receive_payment(&vault, &1000i128, &true, &None, &t, &50u32);

        // Developer can still use lower seq 20
        client.receive_payment(&vault, &100i128, &false, &Some(d.clone()), &t, &20u32);

        // Developer seq advances to 100
        client.receive_payment(&vault, &100i128, &false, &Some(d.clone()), &t, &100u32);

        // Pool can still use seq 70 (which is lower than developer's 100, but higher than pool's 50)
        client.receive_payment(&vault, &500i128, &true, &None, &t, &70u32);

        assert_eq!(client.get_global_pool().total_balance, 1500);
        assert_eq!(client.get_developer_balance(&d, &t), 200);
    }

    /// Pool payments reject equal and lower sequences.
    #[test]
    fn test_hwm_pool_rejects_stale_sequences() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &1000i128, &true, &None, &t, &20u32);
        assert_eq!(client.get_global_pool().total_balance, 1000);

        // Equal sequence fails
        let result_equal = client.try_receive_payment(&vault, &500i128, &true, &None, &t, &20u32);
        assert!(result_equal.is_err(), "equal pool ledger_seq should be rejected");
        assert_eq!(client.get_global_pool().total_balance, 1000, "pool balance unchanged on rejected replay");

        // Lower sequence fails
        let result_lower = client.try_receive_payment(&vault, &500i128, &true, &None, &t, &10u32);
        assert!(result_lower.is_err(), "lower pool ledger_seq should be rejected");
        assert_eq!(client.get_global_pool().total_balance, 1000, "pool balance unchanged on rejected replay");
    }

    /// Reorg scenario: same transaction replayed after a reorg that returns to
    /// the same ledger sequence is caught by the guard.
    #[test]
    fn test_hwm_reorg_replay_rejected() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        client.receive_payment(&vault, &500i128, &false, &Some(d.clone()), &t, &42u32);
        assert_eq!(client.get_developer_balance(&d, &t), 500);

        // Reorg replay: same payload at same ledger_seq
        let result =
            client.try_receive_payment(&vault, &500i128, &false, &Some(d.clone()), &t, &42u32);
        assert!(
            result.is_err(),
            "reorg replay with same ledger_seq must be rejected"
        );

        assert_eq!(client.get_developer_balance(&d, &t), 500);
    }

    /// Batch payments: all developers in the batch must have their HWM
    /// checked independently.
    #[test]
    fn test_hwm_batch_payment() {
        let (env, addr, vault, _admin) = setup();
        let client = CalloraSettlementClient::new(&env, &addr);
        let d1 = dev(&env);
        let d2 = dev(&env);
        let t = token(&env);
        client.add_supported_token(&_admin, &t);

        let items = soroban_sdk::vec![&env, (d1.clone(), 100i128), (d2.clone(), 200i128)];

        client.batch_receive_payment(&vault, &items, &t, &10u32);
        assert_eq!(client.get_developer_balance(&d1, &t), 100);
        assert_eq!(client.get_developer_balance(&d2, &t), 200);

        let result = client.try_batch_receive_payment(&vault, &items, &t, &10u32);
        assert!(
            result.is_err(),
            "batch replay with same seq must be rejected"
        );
    }
}
