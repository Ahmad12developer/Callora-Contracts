//! Batch limits for the Callora Distribute contract.
//!
//! [`Distribute::batch_distribute`](crate::Distribute::batch_distribute) rejects
//! batches larger than [`MAX_BATCH_SIZE`] before validating or transferring
//! any payment. [`Distribute::get_max_batch_size`](crate::Distribute::get_max_batch_size)
//! exposes the same limit to clients.
//!
//! Distributions transfer tokens immediately and do not create per-account
//! state entries or pending payouts. This module does not enforce a lifetime
//! payment count or a per-account storage cap.

/// Maximum number of payment legs allowed in a single batch operation.
pub const MAX_BATCH_SIZE: u32 = 50;

/// TTL bump constants for instance storage archival risk mitigation.
/// Soroban archives ledger entries after ~7 days (631 ledgers) of inactivity.
/// Bumping TTL ensures state remains accessible for critical operations.
pub const BUMP_AMOUNT: u32 = 10_000;
pub const LIFETIME_THRESHOLD: u32 = 1_000;

/// Canonical storage keys for the entire Distribute contract.
///
/// All reads and writes to instance storage MUST go through this enum so that
/// the layout remains consistent.  In particular, the pause flag **must** use
/// `StorageKey::Paused` — using a bare `Symbol("paused")` string key would
/// create a second, incompatible entry and break the circuit-breaker.
#[contracttype]
pub enum StorageKey {
    /// Contract admin address.
    Admin,
    /// Pending admin address during a two-step admin transfer.
    PendingAdmin,
    /// Circuit-breaker flag (`true` = paused).
    ///
    /// This is the **single** canonical key for the pause flag.  No other
    /// key (e.g. a bare `Symbol("paused")`) may be used for this purpose.
    Paused,
    /// Contract version marker (WASM hash) set by `upgrade`.
    ContractVersion,
    /// Global per-account cap.
    GlobalCap,
    /// USDC token address configured during `init`.
    Usdc,
    /// Maximum distributable amount per leg.
    MaxDistribute,
    /// Total active entries for an account (all categories combined).
    AccountCount(Address),
    /// Active entries for a specific `(account, category)` pair.
    AccountCategoryCount(Address, Symbol),
}

/// Per-account state record returned by view functions.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    /// Total active entries across all categories.
    pub count: u32,
}

/// Check whether `account` is currently under the global per-account cap.
///
/// Returns `Ok(true)` when `count < cap`, `Ok(false)` when at or above cap,
/// and `Err(AccountLimitExceeded)` if incrementing would violate the cap.
///
/// This is a pure read-only check — no state is mutated.
pub fn check_under_cap(env: &Env, account: &Address, cap: u32) -> Result<bool, DistributeError> {
    let count = get_account_count(env, account);
    if count < cap {
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Increment the active state count for `account` and return the new total.
///
/// # Errors
/// Returns `DistributeError::AccountLimitExceeded` if the current count is
/// already at or above the cap (i.e. `count >= cap`).
///
/// # Overflow safety
/// Uses `checked_add` internally.  Since the cap is a `u32` and counts are
/// `u32`, overflow is impossible when the cap is enforced, but the check is
/// explicit for defense-in-depth.
pub fn increment_state(env: &Env, account: &Address, cap: u32) -> Result<u32, DistributeError> {
    let current = get_account_count(env, account);
    if current >= cap {
        return Err(DistributeError::AccountLimitExceeded);
    }
    let new_count = current.checked_add(1).ok_or(DistributeError::Overflow)?;
    write_account_count(env, account, new_count);
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    Ok(new_count)
}

/// Decrement the active state count for `account` and return the new total.
///
/// # Errors
/// Returns `DistributeError::AccountStateEmpty` if the current count is zero.
///
/// # Overflow safety
/// Uses `checked_sub` to guard against underflow.
pub fn decrement_state(env: &Env, account: &Address) -> Result<u32, DistributeError> {
    let current = get_account_count(env, account);
    if current == 0 {
        return Err(DistributeError::AccountStateEmpty);
    }
    let new_count = current.checked_sub(1).ok_or(DistributeError::Overflow)?;
    write_account_count(env, account, new_count);
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    Ok(new_count)
}

/// Increment the per-category counter for `(account, category)`.
///
/// Returns the new per-category count.  This function does **not** enforce
/// the global cap — the caller (`open`) must do that separately.
///
/// # Overflow safety
/// Uses `checked_add`.  Returns `DistributeError::Overflow` on `u32` overflow.
pub fn increment_category(
    env: &Env,
    account: &Address,
    category: &Symbol,
) -> Result<u32, DistributeError> {
    let current = get_account_category_count(env, account, category);
    let new_count = current.checked_add(1).ok_or(DistributeError::Overflow)?;
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().set(&key, &new_count);
    Ok(new_count)
}

/// Decrement the per-category counter for `(account, category)`.
///
/// Returns the new per-category count.  Does **not** check the global cap.
///
/// # Overflow safety
/// Uses `checked_sub`.  Returns `DistributeError::AccountStateEmpty` on
/// underflow (category count already zero).
pub fn decrement_category(
    env: &Env,
    account: &Address,
    category: &Symbol,
) -> Result<u32, DistributeError> {
    let current = get_account_category_count(env, account, category);
    if current == 0 {
        return Err(DistributeError::AccountStateEmpty);
    }
    let new_count = current.checked_sub(1).ok_or(DistributeError::Overflow)?;
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().set(&key, &new_count);
    Ok(new_count)
}

/// Read the total active entry count for `account` across all categories.
///
/// Returns `0` if the account has no active entries (safe default).
pub fn get_account_count(env: &Env, account: &Address) -> u32 {
    let key = StorageKey::AccountCount(account.clone());
    env.storage().instance().get(&key).unwrap_or(0u32)
}

/// Read the per-category active entry count for `(account, category)`.
///
/// Returns `0` if the account has no active entries in this category.
pub fn get_account_category_count(env: &Env, account: &Address, category: &Symbol) -> u32 {
    let key = StorageKey::AccountCategoryCount(account.clone(), category.clone());
    env.storage().instance().get(&key).unwrap_or(0u32)
}

/// Read the global per-account cap.
///
/// Returns `DEFAULT_GLOBAL_CAP` if the admin has not explicitly set a cap.
pub fn get_global_cap(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&StorageKey::GlobalCap)
        .unwrap_or(DEFAULT_GLOBAL_CAP)
}

/// Write the global per-account cap to instance storage.
pub fn write_global_cap(env: &Env, cap: u32) {
    env.storage().instance().set(&StorageKey::GlobalCap, &cap);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Write the total active entry count for `account` to instance storage.
fn write_account_count(env: &Env, account: &Address, count: u32) {
    let key = StorageKey::AccountCount(account.clone());
    env.storage().instance().set(&key, &count);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
/// Backwards-compatible aliases for callers that used the limits module.
/// The canonical TTL policy is defined once at the crate root.
pub use crate::{
    INSTANCE_BUMP_AMOUNT as BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD as LIFETIME_THRESHOLD,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors::DistributeError, Distribute, DistributeClient};
    use soroban_sdk::testutils::{storage::Instance as _, Address as _, Events as _};
    use soroban_sdk::{token, Address, Env, Vec};

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let usdc = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let contract = env.register(Distribute, ());
        DistributeClient::new(&env, &contract).init(&admin, &usdc);
        token::StellarAssetClient::new(&env, &usdc).mint(&contract, &1_000);
        (env, admin, recipient, usdc, contract)
    }

    fn payments(env: &Env, recipient: &Address, count: u32) -> Vec<(Address, i128)> {
        let mut payments = Vec::new(env);
        for _ in 0..count {
            payments.push_back((recipient.clone(), 1));
        }
        payments
    }

    #[test]
    fn batch_limits_accept_below_and_at_maximum() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let token = token::Client::new(&env, &usdc);
        assert_eq!(client.get_max_batch_size(), MAX_BATCH_SIZE);

        for count in [MAX_BATCH_SIZE - 1, MAX_BATCH_SIZE] {
            let before = token.balance(&recipient);
            client.batch_distribute(&admin, &payments(&env, &recipient, count));
            assert_eq!(token.balance(&recipient), before + i128::from(count));
        }
        assert_eq!(client.balance(), 1_000 - i128::from(2 * MAX_BATCH_SIZE - 1));
    }

    #[test]
    fn batch_limits_reject_empty_and_oversized_without_transfers() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let token = token::Client::new(&env, &usdc);

        for (count, error) in [
            (0, DistributeError::BatchEmpty),
            (MAX_BATCH_SIZE + 1, DistributeError::BatchTooLarge),
        ] {
            assert_eq!(
                client.try_batch_distribute(&admin, &payments(&env, &recipient, count)),
                Err(Ok(soroban_sdk::Error::from_contract_error(error as u32)))
            );
            // Inspect the rejected invocation before balance queries replace
            // the SDK's per-invocation event buffer.
            assert!(env.events().all().is_empty());
            assert_eq!(client.balance(), 1_000);
            assert_eq!(token.balance(&recipient), 0);
        }

        // Rejection must not prevent a subsequent valid batch.
        client.batch_distribute(&admin, &payments(&env, &recipient, MAX_BATCH_SIZE));
        assert_eq!(token.balance(&recipient), i128::from(MAX_BATCH_SIZE));
    }

    #[test]
    fn batch_limits_apply_per_call_without_accumulating_account_state() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let batch = payments(&env, &recipient, MAX_BATCH_SIZE);
        let state_before = env.as_contract(&contract, || env.storage().instance().all());

        // Exceed the removed default of 100 payments to the same account.
        // Completed payments are not active state and must not consume a cap.
        for _ in 0..3 {
            client.batch_distribute(&admin, &batch);
        }
        client.distribute(&admin, &recipient, &1);

        let total = i128::from(3 * MAX_BATCH_SIZE) + 1;
        assert_eq!(token::Client::new(&env, &usdc).balance(&recipient), total);
        assert_eq!(client.balance(), 1_000 - total);
        env.as_contract(&contract, || {
            assert_eq!(env.storage().instance().all(), state_before);
        });
    }
}
