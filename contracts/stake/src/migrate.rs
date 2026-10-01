//! Stake migration stub — data reshape + upgrade guard.
//!
//! This module provides a minimal, security-hardened migration path for
//! stake state.  It reshapes legacy stake data into the current layout
//! and enforces a monotonic, version-gated upgrade authorisation so that a
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
//! | Entrypoint             | Auth  | Mutates state | Description                               |
//! |------------------------|-------|---------------|-------------------------------------------|
//! | `init`                 | Yes   | Yes           | Configure admin and starting version      |
//! | `migrate`              | Yes   | Yes           | Reshape legacy → current stake data       |
//! | `authorize_upgrade`    | Yes   | Yes           | Record authorised WASM hash for version   |
//! | `get_current`          | No    | No            | Return reshaped state (if migrated)       |
//! | `version`              | No    | No            | Return stored migration version           |
//! | `is_upgrade_authorised`| No    | No            | Check whether a hash is authorised        |
//!
//! Legacy data is expected to have been pre-staged under
//! [`StorageKey::Legacy`] by the previous deployment.  No public entrypoint
//! writes that key, preventing arbitrary callers from supplying migration input.
//!
//! ## Data layout changes
//!
//! | Key           | Legacy                           | Current                           |
//! |---------------|----------------------------------|-----------------------------------|
//! | `LegacyStake` | `LegacyStake { total_staked }`   | read, reshaped, removed           |
//! | `CurrentStake`| —                                | written with added `reserve` field|
//! | `Version`     | absent                           | set to `target_version`           |
//!
//! The `reserve` field defaults to zero during the reshape and is available
//! for use by future stake accounting.
//!
//! ## Events
//!
//! | Function             | Topic           | Topics           | Data             |
//! |----------------------|-----------------|------------------|------------------|
//! | `migrate`            | `stake_migrated`| `(topic)`        | `target_version` |
//! | `authorize_upgrade`  | `upg_authorised`| `(topic, hash)`  | `target_version` |

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Symbol,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

/// Storage keys used by the stake migration contract.
#[derive(Clone)]
#[contracttype]
pub enum StorageKey {
    /// Configured administrator address.
    Admin,
    /// Monotonic migration version counter.
    Version,
    /// Legacy stake data written by the previous deployment.
    Legacy,
    /// Reshaped current stake state (populated after migration).
    Current,
    /// Authorised WASM hash for the current version; stores `(version, hash)`.
    AuthorisedUpgrade,
}

// ─── Data types ───────────────────────────────────────────────────────────────

/// Stake data written by the legacy deployment.
///
/// The legacy layout stored only aggregate `total_staked` with a
/// `last_checkpoint` ledger-sequence field.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct LegacyStake {
    /// Aggregate total stake carried forward from the previous deployment.
    pub total_staked: i128,
    /// Ledger sequence when the legacy state was last written.
    pub last_checkpoint: u32,
}

/// Current stake data layout after a successful migration.
///
/// The `reserve` field is newly introduced and initialised to zero.  Future
/// versions may use this slot for protocol-level reserve accounting.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CurrentStake {
    /// Aggregate total stake preserved from the legacy layout.
    pub total_staked: i128,
    /// Ledger sequence preserved from the legacy layout.
    pub last_checkpoint: u32,
    /// Newly introduced protocol reserve, initialised to zero during reshape.
    pub reserve: i128,
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Stable, machine-readable error codes for the stake migration contract.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[contracterror]
#[repr(u32)]
pub enum StakeMigrateError {
    /// The contract has not been initialised.
    NotInitialized = 1,
    /// Initialisation was attempted more than once.
    AlreadyInitialized = 2,
    /// The caller is not the configured administrator.
    Unauthorized = 3,
    /// No legacy stake state was found in storage.
    LegacyStateMissing = 4,
    /// The legacy `total_staked` value is invalid (negative).
    InvalidLegacyBalance = 5,
    /// The supplied source version does not match the stored version.
    VersionMismatch = 6,
    /// The requested target version is not exactly `expected_version + 1`.
    InvalidTargetVersion = 7,
    /// Migration has already been performed for this version.
    AlreadyMigrated = 8,
    /// No upgrade has been authorised for the current version.
    UpgradeNotAuthorized = 9,
    /// Arithmetic overflow detected in a checked operation.
    Overflow = 10,
    /// The supplied WASM hash is all-zero bytes (almost certainly a mistake).
    WasmHashZero = 11,
}

// ─── Event symbol helpers ─────────────────────────────────────────────────────

/// Returns the `Symbol` for the `"stake_migrated"` event topic.
#[inline]
fn event_stake_migrated(env: &Env) -> Symbol {
    Symbol::new(env, "stake_migrated")
}

/// Returns the `Symbol` for the `"upg_authorised"` event topic.
#[inline]
fn event_upgrade_authorised(env: &Env) -> Symbol {
    Symbol::new(env, "upg_authorised")
}

// ─── Contract ─────────────────────────────────────────────────────────────────

/// Stake data migration and upgrade guard.
///
/// Deploy this contract **before** the main upgrade to ensure stale stake
/// state is reshaped and the new WASM hash is authorised.
#[contract]
pub struct CalloraStakeMigrate;

#[contractimpl]
impl CalloraStakeMigrate {
    /// Initialise the stake migration guard.
    ///
    /// # Arguments
    ///
    /// * `admin` — the administrator address; must authorise this call.
    /// * `initial_version` — the version of the legacy state currently stored
    ///   by the previous deployment.
    ///
    /// # Errors
    ///
    /// Returns [`StakeMigrateError::AlreadyInitialized`] if already initialised.
    pub fn init(env: Env, admin: Address, initial_version: u32) -> Result<(), StakeMigrateError> {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(StakeMigrateError::AlreadyInitialized);
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&StorageKey::Version, &initial_version);
        Ok(())
    }

    /// Reshape legacy stake state and advance the migration version.
    ///
    /// The operation is atomic from the caller's perspective: the current
    /// record and version are written only after all validation passes.
    /// `target_version` **must** be exactly `expected_version + 1`, which
    /// prevents skipped migrations and replayed migrations.
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
    /// The reshaped [`CurrentStake`] record on success.
    ///
    /// # Errors
    ///
    /// | Condition                            | Error                                      |
    /// |--------------------------------------|--------------------------------------------|
    /// | Contract not initialised             | [`StakeMigrateError::NotInitialized`]      |
    /// | Caller is not the admin              | [`StakeMigrateError::Unauthorized`]        |
    /// | `expected_version` ≠ stored version  | [`StakeMigrateError::VersionMismatch`]     |
    /// | `target_version` ≠ version + 1       | [`StakeMigrateError::InvalidTargetVersion`]|
    /// | Migration already completed          | [`StakeMigrateError::AlreadyMigrated`]     |
    /// | Legacy state not found               | [`StakeMigrateError::LegacyStateMissing`]  |
    /// | Legacy `total_staked` is negative    | [`StakeMigrateError::InvalidLegacyBalance`]|
    /// | Version arithmetic overflow          | [`StakeMigrateError::Overflow`]            |
    ///
    /// # Events
    ///
    /// Emits `stake_migrated` with `target_version` as data.
    pub fn migrate(
        env: Env,
        caller: Address,
        expected_version: u32,
        target_version: u32,
    ) -> Result<CurrentStake, StakeMigrateError> {
        caller.require_auth();
        require_admin(&env, &caller)?;

        let version: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::Version)
            .ok_or(StakeMigrateError::NotInitialized)?;

        if version != expected_version {
            return Err(StakeMigrateError::VersionMismatch);
        }

        let next_version = expected_version
            .checked_add(1)
            .ok_or(StakeMigrateError::Overflow)?;

        if target_version != next_version {
            return Err(StakeMigrateError::InvalidTargetVersion);
        }

        if env.storage().instance().has(&StorageKey::Current) {
            return Err(StakeMigrateError::AlreadyMigrated);
        }

        let legacy: LegacyStake = env
            .storage()
            .instance()
            .get(&StorageKey::Legacy)
            .ok_or(StakeMigrateError::LegacyStateMissing)?;

        if legacy.total_staked < 0 {
            return Err(StakeMigrateError::InvalidLegacyBalance);
        }

        let current = CurrentStake {
            total_staked: legacy.total_staked,
            last_checkpoint: legacy.last_checkpoint,
            reserve: 0,
        };

        env.storage().instance().set(&StorageKey::Current, &current);
        env.storage()
            .instance()
            .set(&StorageKey::Version, &target_version);

        env.events()
            .publish((event_stake_migrated(&env),), target_version);

        Ok(current)
    }

    /// Authorise a contract upgrade for the current migration version.
    ///
    /// This is a guard only — it does **not** call
    /// `update_current_contract_wasm`. The deployment tool must consume the
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
    /// | Condition                          | Error                                  |
    /// |------------------------------------|----------------------------------------|
    /// | Contract not initialised           | [`StakeMigrateError::NotInitialized`]  |
    /// | Caller is not the admin            | [`StakeMigrateError::Unauthorized`]    |
    /// | `target_version` ≠ stored version  | [`StakeMigrateError::VersionMismatch`] |
    /// | `wasm_hash` is all-zero bytes      | [`StakeMigrateError::WasmHashZero`]    |
    ///
    /// # Events
    ///
    /// Emits `upg_authorised` with `(topic, wasm_hash)` as topics and
    /// `target_version` as data.
    pub fn authorize_upgrade(
        env: Env,
        caller: Address,
        target_version: u32,
        wasm_hash: BytesN<32>,
    ) -> Result<(), StakeMigrateError> {
        caller.require_auth();
        require_admin(&env, &caller)?;

        let version: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::Version)
            .ok_or(StakeMigrateError::NotInitialized)?;

        if target_version != version {
            return Err(StakeMigrateError::VersionMismatch);
        }

        // Reject an all-zero hash — almost certainly a programming mistake
        // (e.g. a zero-initialised buffer passed by a deployment script).
        if wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(StakeMigrateError::WasmHashZero);
        }

        env.storage().instance().set(
            &StorageKey::AuthorisedUpgrade,
            &(target_version, wasm_hash.clone()),
        );

        env.events()
            .publish((event_upgrade_authorised(&env), wasm_hash), target_version);

        Ok(())
    }

    /// Return the reshaped stake state, if migration has completed.
    ///
    /// Returns `None` if migration has not yet run.
    pub fn get_current(env: Env) -> Option<CurrentStake> {
        env.storage().instance().get(&StorageKey::Current)
    }

    /// Return the stored migration version, if initialised.
    ///
    /// Returns `None` if the contract has not been initialised.
    pub fn version(env: Env) -> Option<u32> {
        env.storage().instance().get(&StorageKey::Version)
    }

    /// Check whether a WASM hash is authorised for the current migration version.
    ///
    /// Returns `true` only when:
    /// - A version is stored.
    /// - An authorised `(version, hash)` pair is stored.
    /// - Both the stored version and the authorised version match **and** the
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
/// Returns [`StakeMigrateError::NotInitialized`] if the contract has not
/// been initialised, or [`StakeMigrateError::Unauthorized`] if `caller` is
/// not the admin.
fn require_admin(env: &Env, caller: &Address) -> Result<(), StakeMigrateError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&StorageKey::Admin)
        .ok_or(StakeMigrateError::NotInitialized)?;
    if caller != &admin {
        return Err(StakeMigrateError::Unauthorized);
    }
    Ok(())
}
