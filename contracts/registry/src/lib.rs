#![no_std]

#[cfg(test)]
extern crate std;

pub mod admin;
pub mod catalog;
pub mod errors;
pub mod events;

pub use errors::RegistryError;

use catalog::OfferingCatalogClient;
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, String};

/// Maximum length of an offering identifier (matches vault offering id limits).
pub const MAX_OFFERING_ID_LEN: u32 = 64;

/// Maximum length of metadata URI / payload stored in registry events.
pub const MAX_METADATA_LEN: u32 = 256;

/// Minimum TTL (closing ledgers) below which offering records are extended
/// on every write and read. Kept large enough that offerings do not archive
/// under normal operation, which would otherwise cause
/// `is_offering_registered` to return `false` and allow duplicate
/// registrations over an archived id.
pub const OFFERING_TTL_THRESHOLD: u32 = 172_800;

/// Minimum TTL (closing ledgers) below which the contract instance is
/// extended on every entrypoint so the admin/catalog/count state stays live.
pub const INSTANCE_TTL_THRESHOLD: u32 = 172_800;

/// Target TTL extension (closing ledgers) for persistent offering records.
/// Equal to `OFFERING_TTL_THRESHOLD` but named separately to make the intent
/// explicit at callsites.
pub const OFFERING_TTL_EXTENSION: u32 = OFFERING_TTL_THRESHOLD;

/// Target TTL extension (closing ledgers) for the contract instance.
pub const INSTANCE_TTL_EXTENSION: u32 = INSTANCE_TTL_THRESHOLD;

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    Admin,
    Catalog,
    RegisteredCount,
    Offering(String),
    /// Per-developer cooldown timestamp. Stored in persistent storage keyed
    /// by the developer [`Address`] so that different developers have
    /// independent cooldown windows.
    DeveloperCooldown(Address),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct OfferingRecord {
    pub offering_id: String,
    pub metadata: String,
    pub developer: Address,
}

#[contract]
pub struct CalloraRegistry;

#[contractimpl]
impl CalloraRegistry {
    /// Extend the contract instance TTL. Called at the start of every
    /// entrypoint so the admin/catalog/count state cannot archive.
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTENSION);
    }

    /// Extend the TTL of a persistent offering record. Called after every
    /// write and on every read so records do not archive.
    fn extend_offering_ttl(env: &Env, key: &StorageKey) {
        env.storage()
            .persistent()
            .extend_ttl(key, OFFERING_TTL_THRESHOLD, OFFERING_TTL_EXTENSION);
    }

    /// Initialize the registry with an admin and catalog callee address.
    ///
    /// The catalog contract receives a cross-contract `put_offering` call for
    /// every successful registration. Registry state is updated only after the
    /// catalog call completes without error.
    pub fn init(env: Env, admin: Address, catalog: Address) -> Result<(), RegistryError> {
        Self::extend_instance_ttl(&env);
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(RegistryError::AlreadyInitialized);
        }
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::Catalog, &catalog);
        inst.set(&StorageKey::RegisteredCount, &0u32);
        env.events()
            .publish((events::event_init(&env), admin.clone()), catalog);
        Ok(())
    }

    fn admin(env: &Env) -> Result<Address, RegistryError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(RegistryError::NotInitialized)
    }

    fn catalog(env: &Env) -> Result<Address, RegistryError> {
        env.storage()
            .instance()
            .get(&StorageKey::Catalog)
            .ok_or(RegistryError::NotInitialized)
    }

    fn validate_offering_id(offering_id: &String) -> Result<(), RegistryError> {
        // Offering ids are storage keys and off-chain routing labels, so they must
        // be unambiguous. We delegate to the shared validator, which enforces
        // the 64-byte cap, rejects C0/DEL controls, zero-width/bidi controls,
        // Unicode confusables, leading/trailing spaces, and restricts the
        // alphabet to `[a-z0-9_-]`. Any rejection maps to `InvalidOfferingId`
        // so callers never see a confusing `InvalidMetadata` for an id failure.
        callora_validators::normalize_offering_id(offering_id)
            .map(|_| ())
            .map_err(|_| RegistryError::InvalidOfferingId)
    }

    fn validate_metadata(metadata: &String) -> Result<(), RegistryError> {
        // Bound the metadata byte length explicitly and reject invalid
        // encodings (C0/DEL controls, zero-width / bidi controls, Unicode
        // confusables, and leading or trailing whitespace) *before* any
        // cross-contract `put_offering` call or registry storage write.
        // `callora_validators::normalize_visible_ascii` enforces the same
        // 256-byte cap as `MAX_METADATA_LEN`, so the two bounds cannot drift
        // and the rejection reason (`InvalidMetadata`) is unambiguous.
        if metadata.len() > MAX_METADATA_LEN {
            return Err(RegistryError::InvalidMetadata);
        }
        callora_validators::normalize_visible_ascii(metadata)
            .map(|_| ())
            .map_err(|_| RegistryError::InvalidMetadata)
    }

    /// Shared registration core used by both public entrypoints.
    ///
    /// Validates inputs, checks for duplicates, publishes to the catalog, then
    /// atomically writes the offering record, increments the registered count,
    /// emits the registration event, and records the developer cooldown timestamp.
    ///
    /// Callers are responsible for authentication, admin-equality checks, the
    /// cooldown gate, and any pre-conditions specific to their variant (e.g.
    /// the developer balance gate in `register_offering_with_gate`).
    fn do_register(
        env: &Env,
        developer: Address,
        offering_id: String,
        metadata: String,
    ) -> Result<(), RegistryError> {
        Self::validate_offering_id(&offering_id)?;
        Self::validate_metadata(&metadata)?;

        let key = StorageKey::Offering(offering_id.clone());
        if env.storage().persistent().has(&key) {
            return Err(RegistryError::OfferingAlreadyRegistered);
        }

        let catalog = Self::catalog(env)?;
        OfferingCatalogClient::new(env, &catalog).put_offering(
            &env.current_contract_address(),
            &offering_id,
            &metadata,
        );

        let record = OfferingRecord {
            offering_id: offering_id.clone(),
            metadata: metadata.clone(),
            developer: developer.clone(),
        };
        env.storage().persistent().set(&key, &record);
        Self::extend_offering_ttl(env, &key);

        let count: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::RegisteredCount)
            .ok_or(RegistryError::NotInitialized)?;
        env.storage().instance().set(
            &StorageKey::RegisteredCount,
            &count.checked_add(1).ok_or(RegistryError::Overflow)?,
        );

        env.events().publish(
            (events::event_offering_registered(env), offering_id),
            record,
        );
        admin::update_cooldown(env, &developer);
        Ok(())
    }

    /// Register an offering after publishing metadata to the catalog contract.
    ///
    /// Cross-contract interactions happen before any registry persistent write,
    /// so a reverting or panicking catalog leaves registry storage unchanged.
    pub fn register_offering(
        env: Env,
        caller: Address,
        developer: Address,
        offering_id: String,
        metadata: String,
    ) -> Result<(), RegistryError> {
        Self::extend_instance_ttl(&env);
        caller.require_auth();
        let admin = Self::admin(&env)?;
        if caller != admin {
            return Err(RegistryError::Unauthorized);
        }
        admin::require_cooldown(&env, &developer)?;
        Self::do_register(&env, developer, offering_id, metadata)
    }

    /// Register an offering only when the developer's on-ledger token balance
    /// meets `min_balance`, then publish via the catalog contract.
    ///
    /// The balance gate runs before the shared registration logic so that an
    /// insufficient balance is rejected before any catalog call or state write.
    pub fn register_offering_with_gate(
        env: Env,
        caller: Address,
        developer: Address,
        token: Address,
        min_balance: i128,
        offering_id: String,
        metadata: String,
    ) -> Result<(), RegistryError> {
        Self::extend_instance_ttl(&env);
        caller.require_auth();
        let admin = Self::admin(&env)?;
        if caller != admin {
            return Err(RegistryError::Unauthorized);
        }
        admin::require_cooldown(&env, &developer)?;

        // Balance gate: reject before any catalog interaction or state write.
        let token_client = token::Client::new(&env, &token);
        let balance = token_client.balance(&developer);
        if balance < min_balance {
            return Err(RegistryError::InsufficientDeveloperBalance);
        }

        Self::do_register(&env, developer, offering_id, metadata)
    }

    /// Return whether `offering_id` has been registered.
    pub fn is_offering_registered(env: Env, offering_id: String) -> Result<bool, RegistryError> {
        Self::extend_instance_ttl(&env);
        Self::validate_offering_id(&offering_id)?;
        if Self::admin(&env).is_err() {
            return Err(RegistryError::NotInitialized);
        }
        let key = StorageKey::Offering(offering_id);
        if env.storage().persistent().has(&key) {
            Self::extend_offering_ttl(&env, &key);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Total number of offerings successfully registered.
    pub fn registered_count(env: Env) -> Result<u32, RegistryError> {
        Self::extend_instance_ttl(&env);
        if !env.storage().instance().has(&StorageKey::Admin) {
            return Err(RegistryError::NotInitialized);
        }
        Ok(env
            .storage()
            .instance()
            .get(&StorageKey::RegisteredCount)
            .ok_or(RegistryError::NotInitialized)?)
    }

    /// Fetch a registered offering record.
    pub fn get_offering(env: Env, offering_id: String) -> Result<OfferingRecord, RegistryError> {
        Self::extend_instance_ttl(&env);
        Self::validate_offering_id(&offering_id)?;
        if Self::admin(&env).is_err() {
            return Err(RegistryError::NotInitialized);
        }
        let key = StorageKey::Offering(offering_id);
        let record = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(RegistryError::OfferingNotFound)?;
        Self::extend_offering_ttl(&env, &key);
        Ok(record)
    }
}
