//! Emergency migration stub — data reshape + upgrade guard.
//!
//! This contract provides a minimal, security-hardened migration path for
//! emergency state.  It reshapes legacy emergency data into the current layout
//! and enforces a monotonic, version-gated upgrade authorization so that a
//! follow-on WASM deployment cannot be installed against stale state.
//!
//! ## Design principles
//!
//! * **Single-shot migration** — the reshape runs at most once per version.
//! * **Admin-gated** — every state-changing entrypoint calls
//!   `caller.require_auth()` and verifies the caller is the configured
//!   administrator.
//! * **Overflow-safe** — all arithmetic uses `checked_*` methods; no
//!   `unwrap()` on production paths.
//! * **Idempotent** — re-running a completed migration is a safe no-op.
//! * **Upgrade guard** — an authorised WASM hash must be recorded for the
//!   current migration version before the deployment tool should proceed.
//!
//! ## API overview
//!
//! | Entrypoint           | Auth  | Mutates state | Description                                 |
//! |----------------------|-------|---------------|---------------------------------------------|
//! | `init`               | Yes   | Yes           | Configure admin and starting version        |
//! | `migrate`            | Yes   | Yes           | Reshape legacy → current data               |
//! | `authorize_upgrade`  | Yes   | Yes           | Record authorised WASM hash for version     |
//! | `get_current`        | No    | No            | Return reshaped state (if migrated)         |
//! | `version`            | No    | No            | Return stored migration version             |
//! | `is_upgrade_authorised`| No  | No            | Check whether a hash is authorised          |
//!
//! Legacy data is expected to have been pre-staged under
//! [`StorageKey::Legacy`] by the previous deployment.  No public entrypoint
//! writes that key, preventing arbitrary callers from supplying migration
//! input.
//!
//! ## Events
//!
//! | Function              | Topic               | Topics          | Data                 |
//! |-----------------------|---------------------|-----------------|----------------------|
//! | `migrate`             | `emergency_migrated`| `(topic)`       | `target_version`     |
//! | `authorize_upgrade`   | `upgrade_authorised`| `(topic, hash)` | `target_version`     |

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Symbol,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

/// Storage keys used by the emergency migration contract.
#[derive(Clone)]
#[contracttype]
pub enum StorageKey {
    /// Configured administrator address.
    Admin,
    /// Monotonic migration version counter.
    Version,
    /// Legacy emergency data written by the previous deployment.
    Legacy,
    /// Reshaped current emergency state (populated after migration).
    Current,
    /// Authorised WASM hash for the current version.
    AuthorisedUpgrade,
}

// ─── Data types ───────────────────────────────────────────────────────────────

/// Data layout written by the legacy deployment.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct LegacyEmergency {
    /// Aggregate emergency balance carried forward.
    pub balance: i128,
    /// Timestamp (ledger seconds) when the legacy state was last updated.
    pub last_updated: u64,
}

/// Current emergency data layout after a successful migration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CurrentEmergency {
    /// Aggregate emergency balance preserved from the legacy layout.
    pub balance: i128,
    /// Timestamp preserved from the legacy layout.
    pub last_updated: u64,
    /// Newly introduced reserve field, initialised to zero during reshape.
    pub reserved: i128,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Stable, machine-readable error codes for the emergency migration contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[contracterror]
#[repr(u32)]
pub enum EmergencyError {
    /// The contract has not been initialised.
    NotInitialized = 1,
    /// Initialisation was attempted more than once.
    AlreadyInitialized = 2,
    /// The caller is not the configured administrator.
    Unauthorized = 3,
    /// No legacy emergency state was found in storage.
    LegacyStateMissing = 4,
    /// The legacy emergency balance is invalid (negative).
    InvalidLegacyBalance = 5,
    /// The supplied source version does not match the stored version.
    VersionMismatch = 6,
    /// The requested target version is not the expected next version.
    InvalidTargetVersion = 7,
    /// Migration has already been performed for this state.
    AlreadyMigrated = 8,
    /// No upgrade has been authorised for the requested version.
    UpgradeNotAuthorized = 9,
    /// Arithmetic overflow detected.
    Overflow = 10,
}

// ─── Event symbols ────────────────────────────────────────────────────────────

/// Returns the Symbol for the `"emergency_migrated"` event topic.
fn event_emergency_migrated(env: &Env) -> Symbol {
    Symbol::new(env, "emergency_migrated")
}

/// Returns the Symbol for the `"upgrade_authorised"` event topic.
fn event_upgrade_authorised(env: &Env) -> Symbol {
    Symbol::new(env, "upgrade_authorised")
}

// ─── Contract ─────────────────────────────────────────────────────────────────

/// Emergency data migration and upgrade guard.
///
/// Deploy this contract **before** the main upgrade to ensure stale emergency
/// state is reshaped and the new WASM hash is authorised.
#[contract]
pub struct EmergencyMigrate;

#[contractimpl]
impl EmergencyMigrate {
    /// Initialise the emergency migration guard.
    ///
    /// # Arguments
    ///
    /// * `admin` — the administrator address; must authorise this call.
    /// * `initial_version` — the version of the legacy state currently stored
    ///   by the previous deployment.
    ///
    /// # Errors
    ///
    /// Returns [`EmergencyError::AlreadyInitialized`] if already initialised.
    pub fn init(env: Env, admin: Address, initial_version: u32) -> Result<(), EmergencyError> {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(EmergencyError::AlreadyInitialized);
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&StorageKey::Version, &initial_version);
        Ok(())
    }

    /// Reshape legacy emergency state and advance the migration version.
    ///
    /// The operation is atomic from the caller's perspective: the current
    /// record and version are written only after all validation passes.  The
    /// target **must** be exactly one greater than `expected_version`,
    /// preventing skipped migrations and replay of an already-completed
    /// migration.
    ///
    /// # Arguments
    ///
    /// * `caller` — must be the configured administrator.
    /// * `expected_version` — the version the contract should currently be at.
    /// * `target_version` — the version to advance to; must equal
    ///   `expected_version + 1`.
    ///
    /// # Returns
    ///
    /// The reshaped [`CurrentEmergency`] record.
    ///
    /// # Errors
    ///
    /// | Condition                              | Error                                 |
    /// |----------------------------------------|---------------------------------------|
    /// | Contract not initialised               | [`EmergencyError::NotInitialized`]    |
    /// | Caller is not the admin                | [`EmergencyError::Unauthorized`]      |
    /// | `expected_version` ≠ stored version    | [`EmergencyError::VersionMismatch`]   |
    /// | `target_version` ≠ version + 1         | [`EmergencyError::InvalidTargetVersion`]|
    /// | Migration already completed            | [`EmergencyError::AlreadyMigrated`]   |
    /// | Legacy state not found                 | [`EmergencyError::LegacyStateMissing`]|
    /// | Legacy balance is negative             | [`EmergencyError::InvalidLegacyBalance`]|
    ///
    /// # Events
    ///
    /// Emits `emergency_migrated` with `target_version` as data.
    pub fn migrate(
        env: Env,
        caller: Address,
        expected_version: u32,
        target_version: u32,
    ) -> Result<CurrentEmergency, EmergencyError> {
        caller.require_auth();
        require_admin(&env, &caller)?;

        let version: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::Version)
            .ok_or(EmergencyError::NotInitialized)?;

        if version != expected_version {
            return Err(EmergencyError::VersionMismatch);
        }

        let next_version = expected_version
            .checked_add(1)
            .ok_or(EmergencyError::Overflow)?;

        if target_version != next_version {
            return Err(EmergencyError::InvalidTargetVersion);
        }

        if env.storage().instance().has(&StorageKey::Current) {
            return Err(EmergencyError::AlreadyMigrated);
        }

        let legacy: LegacyEmergency = env
            .storage()
            .instance()
            .get(&StorageKey::Legacy)
            .ok_or(EmergencyError::LegacyStateMissing)?;

        if legacy.balance < 0 {
            return Err(EmergencyError::InvalidLegacyBalance);
        }

        let current = CurrentEmergency {
            balance: legacy.balance,
            last_updated: legacy.last_updated,
            reserved: 0,
        };

        env.storage().instance().set(&StorageKey::Current, &current);
        env.storage()
            .instance()
            .set(&StorageKey::Version, &target_version);

        env.events()
            .publish((event_emergency_migrated(&env),), target_version);

        Ok(current)
    }

    /// Authorise a contract upgrade for the current migration version.
    ///
    /// This is a guard only — it does **not** call
    /// `update_current_contract_wasm`.  The deployment tool must consume the
    /// returned authorisation state and perform the platform upgrade in a
    /// separate transaction.
    ///
    /// # Arguments
    ///
    /// * `caller` — must be the configured administrator.
    /// * `target_version` — must match the stored migration version.
    /// * `wasm_hash` — the WASM hash to authorise for deployment.
    ///
    /// # Errors
    ///
    /// | Condition                              | Error                               |
    /// |----------------------------------------|-------------------------------------|
    /// | Contract not initialised               | [`EmergencyError::NotInitialized`]  |
    /// | Caller is not the admin                | [`EmergencyError::Unauthorized`]    |
    /// | `target_version` ≠ stored version      | [`EmergencyError::VersionMismatch`] |
    ///
    /// # Events
    ///
    /// Emits `upgrade_authorised` with `(topic, wasm_hash)` as topics and
    /// `target_version` as data.
    pub fn authorize_upgrade(
        env: Env,
        caller: Address,
        target_version: u32,
        wasm_hash: BytesN<32>,
    ) -> Result<(), EmergencyError> {
        caller.require_auth();
        require_admin(&env, &caller)?;

        let version: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::Version)
            .ok_or(EmergencyError::NotInitialized)?;

        if target_version != version {
            return Err(EmergencyError::VersionMismatch);
        }

        env.storage().instance().set(
            &StorageKey::AuthorisedUpgrade,
            &(target_version, wasm_hash.clone()),
        );

        env.events()
            .publish((event_upgrade_authorised(&env), wasm_hash), target_version);

        Ok(())
    }

    /// Return the reshaped emergency state, if migration has completed.
    ///
    /// Returns `None` if migration has not yet run.
    pub fn get_current(env: Env) -> Option<CurrentEmergency> {
        env.storage().instance().get(&StorageKey::Current)
    }

    /// Return the stored migration version, if initialised.
    ///
    /// Returns `None` if the contract has not been initialised.
    pub fn version(env: Env) -> Option<u32> {
        env.storage().instance().get(&StorageKey::Version)
    }

    /// Check whether a WASM hash is authorised for the current migration
    /// version.
    ///
    /// Returns `true` only when:
    /// - A version is stored.
    /// - An authorised (version, hash) pair is stored.
    /// - Both the stored version and the authorised version match, **and** the
    ///   supplied hash matches the authorised hash.
    pub fn is_upgrade_authorised(env: Env, wasm_hash: BytesN<32>) -> bool {
        let stored_version: Option<u32> = env.storage().instance().get(&StorageKey::Version);
        let authorisation: Option<(u32, BytesN<32>)> =
            env.storage().instance().get(&StorageKey::AuthorisedUpgrade);

        match (stored_version, authorisation) {
            (Some(version), Some((auth_version, auth_hash))) => {
                version == auth_version && wasm_hash == auth_hash
            }
            _ => false,
        }
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Verify that `caller` is the stored administrator.
///
/// Returns [`EmergencyError::NotInitialized`] if the contract has not been
/// initialised, or [`EmergencyError::Unauthorized`] if `caller` is not the
/// admin.
fn require_admin(env: &Env, caller: &Address) -> Result<(), EmergencyError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(EmergencyError::NotInitialized)?;
    if caller != &admin {
        return Err(EmergencyError::Unauthorized);
    }
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::Env;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Register the contract, initialise it with `admin` at version `v`, and
    /// pre-stage legacy emergency data.  Returns `(admin, contract_address)`.
    fn setup(env: &Env, legacy_balance: i128, legacy_timestamp: u64) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(env, &contract);
        client.init(&admin, &1);

        // Pre-stage legacy data as the previous deployment would have done.
        env.as_contract(&contract, || {
            env.storage().instance().set(
                &StorageKey::Legacy,
                &LegacyEmergency {
                    balance: legacy_balance,
                    last_updated: legacy_timestamp,
                },
            );
        });

        (admin, contract)
    }

    // ── Initialisation ────────────────────────────────────────────────────────

    #[test]
    fn init_stores_admin_and_version() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        client.init(&admin, &42);
        assert_eq!(client.version(), Some(42));
    }

    #[test]
    fn init_rejects_double_initialisation() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        client.init(&admin, &1);
        let result = client.try_init(&admin, &2);
        assert!(result.is_err());
    }

    #[test]
    fn init_requires_auth() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);
        // No mock_all_auths — caller has not authorised.
        env.set_auths(&[]);
        let result = client.try_init(&admin, &0);
        assert!(result.is_err());
    }

    // ── Migration ─────────────────────────────────────────────────────────────

    #[test]
    fn migrate_reshapes_data_and_advances_version() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 500, 42);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.migrate(&admin, &1, &2);
        assert_eq!(result.balance, 500);
        assert_eq!(result.last_updated, 42);
        assert_eq!(result.reserved, 0);
        assert_eq!(client.version(), Some(2));
        assert_eq!(client.get_current(), Some(result));
    }

    #[test]
    fn migrate_preserves_zero_balance() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 0, 100);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.migrate(&admin, &1, &2);
        assert_eq!(result.balance, 0);
        assert_eq!(result.last_updated, 100);
        assert_eq!(result.reserved, 0);
    }

    #[test]
    fn migrate_preserves_large_balance() {
        let env = Env::default();
        let large = i128::MAX;
        let (admin, contract) = setup(&env, large, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.migrate(&admin, &1, &2);
        assert_eq!(result.balance, large);
        assert_eq!(client.version(), Some(2));
    }

    #[test]
    fn migrate_cannot_be_replayed() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        client.migrate(&admin, &1, &2);
        // Attempting to migrate again at version 2 → 3 should fail because
        // Current is already populated (AlreadyMigrated).
        let result = client.try_migrate(&admin, &2, &3);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_rejects_wrong_expected_version() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.try_migrate(&admin, &0, &1);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_rejects_non_incrementing_target() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        // Target must be exactly expected + 1; skipping versions is rejected.
        let result = client.try_migrate(&admin, &1, &3);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_rejects_negative_legacy_balance() {
        let env = Env::default();
        let (admin, contract) = setup(&env, -1, 42);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.try_migrate(&admin, &1, &2);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_requires_admin_authorisation() {
        let env = Env::default();
        let (_admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let outsider = Address::generate(&env);

        // outsider is not the admin — should be rejected.
        let result = client.try_migrate(&outsider, &1, &2);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_fails_when_legacy_state_missing() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        client.init(&admin, &1);
        // No legacy data pre-staged — migrate should fail.
        let result = client.try_migrate(&admin, &1, &2);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_fails_when_not_initialised() {
        let env = Env::default();
        env.mock_all_auths();
        let caller = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.try_migrate(&caller, &0, &1);
        assert!(result.is_err());
    }

    #[test]
    fn migrate_requires_auth_on_caller() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        // Clear auth and attempt without mock_all_auths.
        env.set_auths(&[]);
        let result = client.try_migrate(&admin, &1, &2);
        assert!(result.is_err());
    }

    // ── Upgrade guard ─────────────────────────────────────────────────────────

    #[test]
    fn authorize_upgrade_stores_hash_for_version() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        client.authorize_upgrade(&admin, &1, &hash);
        assert!(client.is_upgrade_authorised(&hash));
    }

    #[test]
    fn is_upgrade_authorised_rejects_wrong_hash() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let good_hash = BytesN::from_array(&env, &[0xAA; 32]);
        let bad_hash = BytesN::from_array(&env, &[0xBB; 32]);

        client.authorize_upgrade(&admin, &1, &good_hash);
        assert!(client.is_upgrade_authorised(&good_hash));
        assert!(!client.is_upgrade_authorised(&bad_hash));
    }

    #[test]
    fn authorize_upgrade_rejects_wrong_version() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        // Stored version is 1, but we request version 2.
        let result = client.try_authorize_upgrade(&admin, &2, &hash);
        assert!(result.is_err());
    }

    #[test]
    fn authorize_upgrade_requires_admin() {
        let env = Env::default();
        let (_admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let outsider = Address::generate(&env);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        let result = client.try_authorize_upgrade(&outsider, &1, &hash);
        assert!(result.is_err());
    }

    #[test]
    fn authorize_upgrade_requires_auth_on_caller() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        env.set_auths(&[]);
        let result = client.try_authorize_upgrade(&admin, &1, &hash);
        assert!(result.is_err());
    }

    #[test]
    fn is_upgrade_authorised_false_before_authorisation() {
        let env = Env::default();
        let (_admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        assert!(!client.is_upgrade_authorised(&hash));
    }

    #[test]
    fn is_upgrade_authorised_false_when_not_initialised() {
        let env = Env::default();
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        assert!(!client.is_upgrade_authorised(&hash));
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    #[test]
    fn get_current_is_none_before_migration() {
        let env = Env::default();
        let (_admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        assert_eq!(client.get_current(), None);
    }

    #[test]
    fn version_returns_none_before_init() {
        let env = Env::default();
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        assert_eq!(client.version(), None);
    }

    // ── Full lifecycle ────────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_init_migrate_authorize() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 1_000_000, 12345);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let wasm_hash = BytesN::from_array(&env, &[0xCC; 32]);

        // 1. Initialised at version 1 (done by setup).
        assert_eq!(client.version(), Some(1));

        // 2. Migrate legacy → current.
        let current = client.migrate(&admin, &1, &2);
        assert_eq!(current.balance, 1_000_000);
        assert_eq!(current.last_updated, 12345);
        assert_eq!(current.reserved, 0);
        assert_eq!(client.version(), Some(2));
        assert_eq!(client.get_current(), Some(current));

        // 3. Authorise upgrade for post-migration version.
        client.authorize_upgrade(&admin, &2, &wasm_hash);
        assert!(client.is_upgrade_authorised(&wasm_hash));
    }

    // ── Event emission ────────────────────────────────────────────────────────

    #[test]
    fn migrate_emits_event() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 200, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);

        client.migrate(&admin, &1, &2);

        // Check that the emergency_migrated event was emitted.
        let all_events = env.events().all();
        let expected_topic = Symbol::new(&env, "emergency_migrated");
        let has_migrated_event = all_events
            .into_iter()
            .any(|(_addr, topics, _data)| topics.contains(expected_topic.to_val()));
        assert!(has_migrated_event);
    }

    #[test]
    fn authorize_upgrade_emits_event() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 200, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xDD; 32]);

        client.authorize_upgrade(&admin, &1, &hash);

        let all_events = env.events().all();
        let expected_topic = Symbol::new(&env, "upgrade_authorised");
        let has_auth_event = all_events
            .into_iter()
            .any(|(_addr, topics, _data)| topics.contains(expected_topic.to_val()));
        assert!(has_auth_event);
    }

    // ── Ledger timestamp preservation ─────────────────────────────────────────

    #[test]
    fn migrate_preserves_exact_timestamp() {
        let env = Env::default();
        let ts = 1_700_000_000u64;
        let (admin, contract) = setup(&env, 500, ts);
        let client = EmergencyMigrateClient::new(&env, &contract);

        let result = client.migrate(&admin, &1, &2);
        assert_eq!(result.last_updated, ts);
    }

    // ── Edge cases required by issue #1237 ────────────────────────────────────

    /// `migrate` must return `Overflow` when `expected_version` is `u32::MAX`
    /// because `checked_add(1)` wraps and returns `None`.
    #[test]
    fn migrate_version_overflow_returns_overflow_error() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register(EmergencyMigrate, ());
        let client = EmergencyMigrateClient::new(&env, &contract);

        // Initialise at u32::MAX so that checked_add(1) saturates.
        client.init(&admin, &u32::MAX);
        env.as_contract(&contract, || {
            env.storage().instance().set(
                &StorageKey::Legacy,
                &LegacyEmergency {
                    balance: 100,
                    last_updated: 1,
                },
            );
        });

        // expected_version = u32::MAX; target would require u32::MAX + 1 → overflow.
        let result = client.try_migrate(&admin, &u32::MAX, &0);
        assert!(
            result.is_err(),
            "migrate with u32::MAX expected_version must fail with Overflow"
        );
    }

    /// `authorize_upgrade` must accept a valid call even when the WASM hash is
    /// the all-zero byte array.  The contract imposes no lower bound on hash
    /// content; rejecting zeros would be an undocumented implicit constraint.
    #[test]
    fn authorize_upgrade_accepts_zero_wasm_hash() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let zero_hash = BytesN::from_array(&env, &[0u8; 32]);

        // Must succeed without error.
        client.authorize_upgrade(&admin, &1, &zero_hash);
        assert!(
            client.is_upgrade_authorised(&zero_hash),
            "zero wasm hash must be accepted and reported as authorised"
        );
        // A non-zero hash must still be rejected.
        let other_hash = BytesN::from_array(&env, &[0xAB; 32]);
        assert!(
            !client.is_upgrade_authorised(&other_hash),
            "non-authorised hash must not be reported as authorised"
        );
    }

    /// `authorize_upgrade` must return `VersionMismatch` when `target_version`
    /// is strictly *less* than the stored version (stale version path).
    #[test]
    fn authorize_upgrade_rejects_stale_version() {
        let env = Env::default();
        let (admin, contract) = setup(&env, 100, 1);
        let client = EmergencyMigrateClient::new(&env, &contract);
        let hash = BytesN::from_array(&env, &[0xAA; 32]);

        // Stored version is 1. Supplying version 0 is a stale (past) version.
        let result = client.try_authorize_upgrade(&admin, &0, &hash);
        assert!(
            result.is_err(),
            "authorize_upgrade with a stale (past) version must be rejected"
        );

        // Supplying a future version (2) is equally invalid.
        let result = client.try_authorize_upgrade(&admin, &2, &hash);
        assert!(
            result.is_err(),
            "authorize_upgrade with a future version must be rejected"
        );

        // Only the exact stored version (1) should be accepted.
        client.authorize_upgrade(&admin, &1, &hash);
        assert!(
            client.is_upgrade_authorised(&hash),
            "authorize_upgrade with the exact stored version must succeed"
        );
    }
}
