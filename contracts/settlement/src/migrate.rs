//! Settlement storage migration: V1 (single-token) -> V2 (per-token) layout.
//!
//! ## Background
//!
//! The original settlement contract stored developer balances as a flat,
//! single-token mapping keyed by `StorageKey::DeveloperBalanceV1(Address)`.
//! V2 introduces explicit per-token accounting via
//! `StorageKey::DeveloperBalance(Address, Address)` where the second address
//! is the token contract (typically USDC).
//!
//! ## Storage layout changes
//!
//! | Key | V1 | V2 |
//! |-----|----|----|
//! | `DeveloperBalanceV1(addr)` | `i128` | read, merged, removed |
//! | `DeveloperBalance(addr, usdc_token)` | - | written during migration |
//! | `StorageVersion` | absent | `2u32` on completion |
//!
//! Existing V2 credits written after WASM upgrade but before migration runs are
//! **preserved and merged** with the corresponding V1 balance.
//!
//! ## Usage
//!
//! ```text
//! // One-shot (<=50 developers)
//! client.migrate_v1_to_v2(&admin);
//!
//! // Paginated (large deployments)
//! let mut offset = 0u32;
//! loop {
//!     let (next, done) = client.migrate_v1_to_v2_page(&admin, &offset, &50u32);
//!     if done { break; }
//!     offset = next;
//! }
//! assert_eq!(client.migration_storage_version(), 2u32);
//! ```
//!
//! ## Security
//!
//! - All entry points call `caller.require_auth()` and verify the admin address.
//! - Re-running after `StorageVersion == 2` is a safe no-op (idempotent).
//! - Balance merging uses `checked_add`; no silent overflow.
//! - No `unwrap()` in production paths.

use soroban_sdk::{Address, Env, Symbol, Vec};

use crate::{
    SettlementError, StorageKey, INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD, MAX_BATCH_SIZE,
};

/// Storage-layout version that predates version tracking.
pub const STORAGE_VERSION_V1: u32 = 1;
/// Storage-layout version set after the V1 -> V2 migration completes.
pub const STORAGE_VERSION_V2: u32 = 2;

// ─── Public query ─────────────────────────────────────────────────────────────

/// Return the current storage-layout version.
///
/// Returns [`STORAGE_VERSION_V1`] when the `StorageVersion` key is absent
/// (contract initialised before version tracking was introduced).
/// Returns [`STORAGE_VERSION_V2`] once [`migrate_v1_to_v2`] has completed.
pub fn storage_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    env.storage()
        .instance()
        .get(&StorageKey::StorageVersion)
        .unwrap_or(STORAGE_VERSION_V1)
}

// ─── Public entry points ──────────────────────────────────────────────────────

/// One-shot V1 -> V2 storage migration (admin only).
///
/// Iterates over every address in [`StorageKey::DeveloperIndex`], reads the
/// legacy `DeveloperBalanceV1(addr)` slot, merges it into
/// `DeveloperBalance(addr, usdc_token)`, and removes the V1 slot.
///
/// Suitable for deployments with **<= [`MAX_BATCH_SIZE`] registered
/// developers**. For larger deployments use [`migrate_v1_to_v2_page`].
///
/// # Arguments
///
/// * `caller` - Must be the current admin; `caller.require_auth()` is invoked.
///
/// # Panics
///
/// | Condition | Error |
/// |-----------|-------|
/// | Contract not initialised | [`SettlementError::NotInitialized`] |
/// | Caller is not the admin | [`SettlementError::Unauthorized`] |
/// | USDC token not configured | [`SettlementError::UsdcTokenNotConfigured`] |
/// | Balance merge overflows `i128` | [`SettlementError::DeveloperOverflow`] |
///
/// # Idempotency
///
/// Returns immediately without state changes when `StorageVersion == 2`.
pub fn migrate_v1_to_v2(env: &Env, caller: &Address) {
    caller.require_auth();
    require_admin(env, caller);
    register_configured_usdc(env, caller);

    if storage_version(env) >= STORAGE_VERSION_V2 {
        return;
    }

    let inst = env.storage().instance();
    let usdc_token: Address = inst
        .get(&StorageKey::Usdc)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::UsdcTokenNotConfigured));

    let index: Vec<Address> = inst
        .get(&StorageKey::DeveloperIndex)
        .unwrap_or_else(|| Vec::new(env));

    for addr in index.iter() {
        migrate_developer_slot(env, &addr, &usdc_token);
    }

    inst.set(&StorageKey::StorageVersion, &STORAGE_VERSION_V2);
    env.events()
        .publish((Symbol::new(env, "mig_v1_v2_done"),), STORAGE_VERSION_V2);
}

/// Paginated V1 -> V2 storage migration (admin only).
///
/// Processes up to `batch_size` (capped at [`MAX_BATCH_SIZE`]) developer
/// accounts per call, starting at index position `offset`. Call repeatedly,
/// passing the returned `next_offset` as `offset`, until `is_complete == true`.
///
/// # Snap-point API
///
/// ```text
/// let mut offset = 0u32;
/// loop {
///     let (next, done) = client.migrate_v1_to_v2_page(&admin, &offset, &50u32);
///     if done { break; }
///     offset = next;
/// }
/// ```
///
/// # Arguments
///
/// * `caller` - Must be the current admin.
/// * `offset` - Index of the first developer to process. Pass `0` on first call.
/// * `batch_size` - Max developers per call; capped at [`MAX_BATCH_SIZE`].
///   A value of `0` is treated as `1`.
///
/// # Returns
///
/// `(next_offset, is_complete)`:
/// * `next_offset` - First unprocessed position for the next call.
/// * `is_complete` - `true` when all slots are migrated and `StorageVersion == 2`.
///
/// # Panics
///
/// Same conditions as [`migrate_v1_to_v2`].
///
/// # Idempotency
///
/// Returns `(0, true)` immediately when migration is already at V2.
pub fn migrate_v1_to_v2_page(
    env: &Env,
    caller: &Address,
    offset: u32,
    batch_size: u32,
) -> (u32, bool) {
    caller.require_auth();
    require_admin(env, caller);
    register_configured_usdc(env, caller);

    if storage_version(env) >= STORAGE_VERSION_V2 {
        return (0, true);
    }

    let inst = env.storage().instance();
    let usdc_token: Address = inst
        .get(&StorageKey::Usdc)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::UsdcTokenNotConfigured));

    let index: Vec<Address> = inst
        .get(&StorageKey::DeveloperIndex)
        .unwrap_or_else(|| Vec::new(env));

    let total = index.len();
    let effective = if batch_size == 0 {
        1
    } else {
        batch_size.min(MAX_BATCH_SIZE)
    };
    let end = offset.saturating_add(effective).min(total);

    let mut i = 0u32;
    for addr in index.iter() {
        if i >= end {
            break;
        }
        if i >= offset {
            migrate_developer_slot(env, &addr, &usdc_token);
        }
        i = i.saturating_add(1);
    }

    let done = end >= total;
    if done {
        inst.set(&StorageKey::StorageVersion, &STORAGE_VERSION_V2);
        env.events()
            .publish((Symbol::new(env, "mig_v1_v2_done"),), STORAGE_VERSION_V2);
    }

    (end, done)
}

// ─── Internal helpers ─────────────────────────────────────────────────────────
fn register_configured_usdc(env: &Env, caller: &Address) {
    let instance = env.storage().instance();
    if instance
        .get::<_, bool>(&StorageKey::SupportedTokensMigrated)
        .unwrap_or(false)
    {
        return;
    }
    let Some(usdc_token) = instance.get::<_, Address>(&StorageKey::Usdc) else {
        return;
    };
    let key = StorageKey::SupportedToken(usdc_token.clone());
    if !env.storage().persistent().has(&key) {
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(
            &key,
            crate::PERSISTENT_BUMP_THRESHOLD,
            crate::PERSISTENT_BUMP_AMOUNT,
        );
        crate::events::emit_supported_token_added(env, caller, &usdc_token);
    }
    instance.set(&StorageKey::SupportedTokensMigrated, &true);
}

/// Abort with `NotInitialized` if the contract has not been initialised, or
/// `Unauthorized` if `caller` is not the stored admin.
fn require_admin(env: &Env, caller: &Address) {
    let inst = env.storage().instance();
    if !inst.has(&StorageKey::Admin) {
        env.panic_with_error(SettlementError::NotInitialized);
    }
    let admin: Address = inst
        .get(&StorageKey::Admin)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::NotInitialized));
    if caller != &admin {
        env.panic_with_error(SettlementError::Unauthorized);
    }
}

/// Read the V1 balance for `addr`, merge it into the V2 per-token slot, and
/// remove the V1 key. Already-migrated addresses (no V1 slot) are skipped.
fn migrate_developer_slot(env: &Env, addr: &Address, usdc_token: &Address) {
    let v1_key = StorageKey::DeveloperBalanceV1(addr.clone());
    let v1_balance: Option<i128> = env.storage().persistent().get(&v1_key);
    if let Some(v1) = v1_balance {
        let v2_key = StorageKey::DeveloperBalance(addr.clone(), usdc_token.clone());
        let existing_v2: i128 = env.storage().persistent().get(&v2_key).unwrap_or(0i128);
        let merged = v1
            .checked_add(existing_v2)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));
        env.storage().persistent().set(&v2_key, &merged);
        env.storage().persistent().extend_ttl(
            &v2_key,
            crate::types::PERSISTENT_BUMP_THRESHOLD,
            crate::types::PERSISTENT_BUMP_AMOUNT,
        );
        env.storage().persistent().remove(&v1_key);
    }
}

/// Migrate a single developer's V1 balance to V2 (admin only).
pub fn migrate_single_developer(
    env: &Env,
    caller: &Address,
    developer: &Address,
) -> Result<(), crate::SettlementError> {
    caller.require_auth();
    require_admin(env, caller);
    let inst = env.storage().instance();
    let usdc_token: Address = inst
        .get(&crate::StorageKey::Usdc)
        .ok_or(crate::SettlementError::UsdcTokenNotConfigured)?;
    migrate_developer_slot(env, developer, &usdc_token);
    Ok(())
}

// ─── Index-to-pages migration ─────────────────────────────────────────────────

/// Migrate the legacy flat `DeveloperIndex` (instance storage `Vec<Address>`)
/// to the new paged persistent index (`IndexPage(u32)` + `DeveloperMember`).
///
/// ## What this does
///
/// 1. Reads `StorageKey::DeveloperIndex` from instance storage (the old
///    unbounded `Vec<Address>`).
/// 2. For each address, calls [`CalloraSettlement::index_insert`] which:
///    - Sets `StorageKey::DeveloperMember(addr)` in persistent storage.
///    - Appends the address to the appropriate `StorageKey::IndexPage(n)`.
///    - Updates `StorageKey::IndexPageCount` in instance storage.
/// 3. Removes `StorageKey::DeveloperIndex` from instance storage.
///
/// ## Resumability
///
/// The function uses `DeveloperMember` as a per-address idempotency guard.
/// If the migration is interrupted mid-way and re-run, already-migrated
/// addresses are skipped in O(1).  Partial progress is therefore safe.
///
/// ## Idempotency
///
/// Once `StorageKey::DeveloperIndex` is absent (removed on completion),
/// subsequent calls are a cheap no-op: no pages exist to iterate, and
/// `index_insert` guards against double-registration.
///
/// ## Batch API
///
/// For large deployments process `batch_size` addresses per call and pass
/// the returned `next_offset` back in until `is_complete == true`.
///
/// ```text
/// let mut offset = 0u32;
/// loop {
///     let (next, done) = client.migrate_index_to_pages(&admin, &offset, &50u32);
///     if done { break; }
///     offset = next;
/// }
/// ```
///
/// ## Auth
///
/// Admin signature required.
pub fn migrate_index_to_pages(
    env: &Env,
    caller: &Address,
    offset: u32,
    batch_size: u32,
) -> (u32, bool) {
    caller.require_auth();
    require_admin(env, caller);

    let inst = env.storage().instance();

    // If the old key is gone the migration is already complete.
    if !inst.has(&StorageKey::DeveloperIndex) {
        return (0, true);
    }

    let index: Vec<Address> = inst
        .get(&StorageKey::DeveloperIndex)
        .unwrap_or_else(|| Vec::new(env));

    let total = index.len();

    // Empty old index — remove and mark done immediately.
    if total == 0 {
        inst.remove(&StorageKey::DeveloperIndex);
        env.events().publish(
            (Symbol::new(env, "mig_idx_pages_done"),),
            0u32,
        );
        return (0, true);
    }

    let effective = if batch_size == 0 {
        1u32
    } else {
        batch_size.min(MAX_BATCH_SIZE)
    };
    let end = offset.saturating_add(effective).min(total);

    let mut i = 0u32;
    for addr in index.iter() {
        if i >= end {
            break;
        }
        if i >= offset {
            crate::CalloraSettlement::index_insert(env, addr);
        }
        i = i.saturating_add(1);
    }

    let done = end >= total;
    if done {
        inst.remove(&StorageKey::DeveloperIndex);
        env.events().publish(
            (Symbol::new(env, "mig_idx_pages_done"),),
            total,
        );
    }

    (end, done)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{CalloraSettlement, CalloraSettlementClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Register a fresh contract and configure admin, vault, and USDC.
    /// Returns `(contract_address, admin, usdc_token)`.
    fn setup(env: &Env) -> (Address, Address, Address) {
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(env, &contract);
        let admin = Address::generate(env);
        let vault = Address::generate(env);
        let usdc = Address::generate(env);
        env.mock_all_auths();
        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc);
        (contract, admin, usdc)
    }

    // ── storage_version ───────────────────────────────────────────────────────

    #[test]
    fn storage_version_is_one_before_migration() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, ..) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V1);
    }

    // ── one-shot migration ────────────────────────────────────────────────────

    #[test]
    fn one_shot_empty_contract_marks_v2() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.migrate_v1_to_v2(&admin);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V2);
    }

    #[test]
    fn one_shot_migration_is_idempotent() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.migrate_v1_to_v2(&admin);
        client.migrate_v1_to_v2(&admin);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V2);
    }

    #[test]
    fn one_shot_migrates_v1_developer_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, usdc) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let dev = Address::generate(&env);

        env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .set(&StorageKey::DeveloperBalanceV1(dev.clone()), &5_000i128);
            let mut idx: soroban_sdk::Vec<Address> = env
                .storage()
                .instance()
                .get(&StorageKey::DeveloperIndex)
                .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
            idx.push_back(dev.clone());
            env.storage()
                .instance()
                .set(&StorageKey::DeveloperIndex, &idx);
        });

        client.migrate_v1_to_v2(&admin);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V2);

        let v2_balance: i128 = env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .get(&StorageKey::DeveloperBalance(dev.clone(), usdc.clone()))
                .unwrap_or(0)
        });
        assert_eq!(v2_balance, 5_000i128);

        let v1_gone: bool = env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .get::<_, i128>(&StorageKey::DeveloperBalanceV1(dev.clone()))
                .is_none()
        });
        assert!(v1_gone);
    }

    #[test]
    fn one_shot_merges_v1_and_existing_v2_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, usdc) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let dev = Address::generate(&env);

        env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .set(&StorageKey::DeveloperBalanceV1(dev.clone()), &3_000i128);
            env.storage().persistent().set(
                &StorageKey::DeveloperBalance(dev.clone(), usdc.clone()),
                &1_500i128,
            );
            let mut idx: soroban_sdk::Vec<Address> = env
                .storage()
                .instance()
                .get(&StorageKey::DeveloperIndex)
                .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
            idx.push_back(dev.clone());
            env.storage()
                .instance()
                .set(&StorageKey::DeveloperIndex, &idx);
        });

        client.migrate_v1_to_v2(&admin);

        let merged: i128 = env.as_contract(&contract, || {
            env.storage()
                .persistent()
                .get(&StorageKey::DeveloperBalance(dev.clone(), usdc.clone()))
                .unwrap_or(0)
        });
        assert_eq!(merged, 4_500i128);
    }

    #[test]
    #[should_panic]
    fn one_shot_panics_when_not_initialized() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &contract);
        let caller = Address::generate(&env);
        client.migrate_v1_to_v2(&caller);
    }

    #[test]
    #[should_panic]
    fn one_shot_panics_when_caller_not_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, _, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let non_admin = Address::generate(&env);
        client.migrate_v1_to_v2(&non_admin);
    }

    #[test]
    #[should_panic]
    fn one_shot_panics_when_usdc_not_configured() {
        let env = Env::default();
        env.mock_all_auths();
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &contract);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        client.init(&admin, &vault);
        client.migrate_v1_to_v2(&admin);
    }

    // ── paginated migration ───────────────────────────────────────────────────

    #[test]
    fn paginated_empty_contract_completes_in_one_call() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let (_, done) = client.migrate_v1_to_v2_page(&admin, &0u32, &50u32);
        assert!(done);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V2);
    }

    #[test]
    fn paginated_idempotent_after_completion() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.migrate_v1_to_v2_page(&admin, &0u32, &50u32);
        let (next, done) = client.migrate_v1_to_v2_page(&admin, &0u32, &50u32);
        assert!(done);
        assert_eq!(next, 0u32);
    }

    #[test]
    fn paginated_multi_page_processes_all_developers() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, usdc) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        let dev_a = Address::generate(&env);
        let dev_b = Address::generate(&env);
        let dev_c = Address::generate(&env);
        let dev_d = Address::generate(&env);
        let dev_e = Address::generate(&env);

        let devs = [
            dev_a.clone(),
            dev_b.clone(),
            dev_c.clone(),
            dev_d.clone(),
            dev_e.clone(),
        ];
        env.as_contract(&contract, || {
            for dev in devs.iter() {
                env.storage()
                    .persistent()
                    .set(&StorageKey::DeveloperBalanceV1(dev.clone()), &100i128);
                let mut idx: soroban_sdk::Vec<Address> = env
                    .storage()
                    .instance()
                    .get(&StorageKey::DeveloperIndex)
                    .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
                idx.push_back(dev.clone());
                env.storage()
                    .instance()
                    .set(&StorageKey::DeveloperIndex, &idx);
            }
        });

        let (o1, d1) = client.migrate_v1_to_v2_page(&admin, &0u32, &2u32);
        assert!(!d1);
        let (o2, d2) = client.migrate_v1_to_v2_page(&admin, &o1, &2u32);
        assert!(!d2);
        let (_, d3) = client.migrate_v1_to_v2_page(&admin, &o2, &2u32);
        assert!(d3);
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V2);

        for dev in devs.iter() {
            let bal: i128 = env.as_contract(&contract, || {
                env.storage()
                    .persistent()
                    .get(&StorageKey::DeveloperBalance(dev.clone(), usdc.clone()))
                    .unwrap_or(0)
            });
            assert_eq!(bal, 100i128);
        }
    }

    #[test]
    fn paginated_migration_matches_one_shot_across_varied_pages() {
        let env = Env::default();
        env.mock_all_auths();
        let (one_shot_contract, one_shot_admin, one_shot_usdc) = setup(&env);
        let (paged_contract, paged_admin, paged_usdc) = setup(&env);
        let one_shot = CalloraSettlementClient::new(&env, &one_shot_contract);
        let paged = CalloraSettlementClient::new(&env, &paged_contract);
        let devs: [Address; 9] = core::array::from_fn(|_| Address::generate(&env));

        for contract in [&one_shot_contract, &paged_contract] {
            env.as_contract(contract, || {
                let mut index = soroban_sdk::Vec::new(&env);
                for (position, dev) in devs.iter().enumerate() {
                    let v1_balance = 1_000 + position as i128;
                    let existing_v2 = 100 + position as i128;
                    env.storage()
                        .persistent()
                        .set(&StorageKey::DeveloperBalanceV1(dev.clone()), &v1_balance);
                    let usdc = if contract == &one_shot_contract {
                        one_shot_usdc.clone()
                    } else {
                        paged_usdc.clone()
                    };
                    env.storage().persistent().set(
                        &StorageKey::DeveloperBalance(dev.clone(), usdc),
                        &existing_v2,
                    );
                    index.push_back(dev.clone());
                }
                env.storage()
                    .instance()
                    .set(&StorageKey::DeveloperIndex, &index);
            });
        }

        one_shot.migrate_v1_to_v2(&one_shot_admin);
        assert_eq!(one_shot.migration_storage_version(), STORAGE_VERSION_V2);
        assert_eq!(paged.migration_storage_version(), STORAGE_VERSION_V1);

        let total = devs.len() as u32;
        let mut offset = 0u32;
        let mut random_state = 0xC011_AA_u32;
        while offset < total {
            random_state = random_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let batch_size = random_state % 4 + 1;
            let (next_offset, done) =
                paged.migrate_v1_to_v2_page(&paged_admin, &offset, &batch_size);
            assert_eq!(next_offset, offset.saturating_add(batch_size).min(total));
            assert_eq!(done, next_offset == total);
            assert_eq!(
                paged.migration_storage_version(),
                if done {
                    STORAGE_VERSION_V2
                } else {
                    STORAGE_VERSION_V1
                }
            );
            offset = next_offset;
        }

        let (next_offset, done) = paged.migrate_v1_to_v2_page(&paged_admin, &0, &3);
        assert_eq!((next_offset, done), (0, true));
        assert_eq!(paged.migration_storage_version(), STORAGE_VERSION_V2);

        for (position, dev) in devs.iter().enumerate() {
            let expected = 1_100 + 2 * position as i128;
            let one_shot_balance: i128 = env.as_contract(&one_shot_contract, || {
                env.storage()
                    .persistent()
                    .get(&StorageKey::DeveloperBalance(
                        dev.clone(),
                        one_shot_usdc.clone(),
                    ))
                    .unwrap_or(0)
            });
            let paged_balance: i128 = env.as_contract(&paged_contract, || {
                env.storage()
                    .persistent()
                    .get(&StorageKey::DeveloperBalance(
                        dev.clone(),
                        paged_usdc.clone(),
                    ))
                    .unwrap_or(0)
            });
            assert_eq!(one_shot_balance, expected);
            assert_eq!(paged_balance, one_shot_balance);
            for contract in [&one_shot_contract, &paged_contract] {
                let v1_exists = env.as_contract(contract, || {
                    env.storage()
                        .persistent()
                        .has(&StorageKey::DeveloperBalanceV1(dev.clone()))
                });
                assert!(!v1_exists);
            }
        }
    }

    #[test]
    fn paginated_migration_rejects_non_admin() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, _, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let non_admin = Address::generate(&env);

        assert!(client
            .try_migrate_v1_to_v2_page(&non_admin, &0u32, &1u32)
            .is_err());
        assert_eq!(client.migration_storage_version(), STORAGE_VERSION_V1);
    }

    #[test]
    fn paginated_batch_size_capped_at_max_batch_size() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let (_, done) = client.migrate_v1_to_v2_page(&admin, &0u32, &u32::MAX);
        assert!(done);
    }

    #[test]
    fn paginated_zero_batch_treated_as_one() {
        let env = Env::default();
        env.mock_all_auths();
        let (contract, admin, _) = setup(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let (_, done) = client.migrate_v1_to_v2_page(&admin, &0u32, &0u32);
        assert!(done);
    }
}
