#![no_std]
//! Callora Topics contract.
//!
//! Manages named event topics for the Callora API marketplace.  Each topic
//! represents a billable API surface that operators can register, resolve, and
//! deactivate.  All topic keys are short Soroban `Symbol` values so they fit
//! within a single ledger entry key.
//!
//! ## Storage layout
//!
//! | Scope      | Key                           | Value                    |
//! |------------|-------------------------------|--------------------------|
//! | Instance   | `StorageKey::Admin`           | `Address`                |
//! | Persistent | `StorageKey::Topic(symbol)`   | `TopicRecord`            |
//! | Instance   | `StorageKey::TopicCount`      | `u32`                    |
//!
//! ## TTL management
//!
//! Instance storage is bumped on every mutating call to
//! `BUMP_AMOUNT` (≈16 days) when it falls below `LIFETIME_THRESHOLD` (≈1.5 days).
//! Hot persistent topic records receive the same bump on every read so they are
//! never unexpectedly archived.
//!
//! ## Events
//!
//! | Entrypoint        | Topic                  | Data                               |
//! |-------------------|------------------------|------------------------------------|
//! | `init`            | `"topics_init"`        | `admin: Address`                   |
//! | `register_topic`  | `"topic_registered"`   | `TopicRecord`                      |
//! | `deactivate`      | `"topic_deactivated"`  | `topic_name: Symbol`               |
//! | `reactivate`      | `"topic_reactivated"`  | `topic_name: Symbol`               |
//! | `set_topic_owner` | `"topic_owner_chg"`    | `(old_owner, new_owner, name)`     |

pub mod errors;
pub mod events;

pub use errors::TopicsError;

#[cfg(test)]
mod test_lifecycle;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, String, Symbol};
use callora_validators::normalize_visible_ascii;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Extend instance storage TTL by 10 000 ledgers (≈16 days) on every mutating
/// call so the contract is never archived under normal operating load.
pub const BUMP_AMOUNT: u32 = 10_000;

/// Minimum remaining TTL before triggering a bump (≈1.5 days).
pub const LIFETIME_THRESHOLD: u32 = 1_000;

/// Bump for persistent per-topic records on hot-read paths.
pub const PERSISTENT_BUMP: u32 = 10_000;
pub const PERSISTENT_THRESHOLD: u32 = 1_000;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// Typed storage key enum.  Using `contracttype` keeps the XDR encoding
/// deterministic across upgrades.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    /// Stores the contract admin `Address`.
    Admin,
    /// Counter of how many topics have been registered (monotonically increasing).
    TopicCount,
    /// Per-topic record keyed by its name.
    Topic(Symbol),
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A registered API topic record.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TopicRecord {
    /// Short topic identifier (≤ 32 chars, Soroban `Symbol`-compatible).
    pub name: Symbol,
    /// Human-readable description stored as an on-chain `String`.
    pub description: String,
    /// Address of the developer / operator that owns this topic.
    pub owner: Address,
    /// Whether the topic is currently accepting metered calls.
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraTopics;

#[contractimpl]
impl CalloraTopics {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Initialize the topics contract.
    ///
    /// Can only be called once. The `admin` address must authorize the call.
    ///
    /// # Errors
    /// Returns [`TopicsError::AlreadyInitialized`] if called more than once.
    pub fn init(env: Env, admin: Address) -> Result<(), TopicsError> {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(TopicsError::AlreadyInitialized);
        }
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::TopicCount, &0u32);
        Self::bump_instance(&env);
        env.events()
            .publish((events::event_init(&env),), admin.clone());
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Mutating entrypoints
    // -----------------------------------------------------------------------

    /// Register a new topic on-chain.
    ///
    /// Only the admin may register topics. The `name` must be unique.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::Unauthorized`] if `caller` is not the admin.
    /// - [`TopicsError::TopicAlreadyExists`] if `name` is already registered.
    /// - [`TopicsError::InvalidDescription`] if `description` is empty, exceeds
    ///   256 bytes, contains control characters, or has leading/trailing whitespace.
    pub fn register_topic(
        env: Env,
        caller: Address,
        name: Symbol,
        description: String,
        owner: Address,
    ) -> Result<(), TopicsError> {
        caller.require_auth();
        let admin = Self::require_admin(&env)?;
        if caller != admin {
            return Err(TopicsError::Unauthorized);
        }

        let key = StorageKey::Topic(name.clone());
        if env.storage().persistent().has(&key) {
            return Err(TopicsError::TopicAlreadyExists);
        }

        // Reject descriptions that are empty, exceed 256 bytes, contain C0/DEL
        // control characters, or have leading / trailing whitespace.  This
        // prevents oversized payloads from inflating storage and event costs
        // and makes the on-chain description byte-stable visible ASCII.
        normalize_visible_ascii(&description)
            .map_err(|_| TopicsError::InvalidDescription)?;

        let record = TopicRecord {
            name: name.clone(),
            description,
            owner,
            active: true,
        };

        env.storage().persistent().set(&key, &record);

        let count: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::TopicCount)
            .unwrap_or(0);
        env.storage().instance().set(
            &StorageKey::TopicCount,
            &count.checked_add(1).ok_or(TopicsError::Overflow)?,
        );

        Self::bump_instance(&env);
        env.events()
            .publish((events::event_topic_registered(&env), name), record);
        Ok(())
    }

    /// Deactivate a registered topic, preventing new metered calls.
    ///
    /// Only the admin may deactivate topics.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::Unauthorized`] if `caller` is not the admin.
    /// - [`TopicsError::TopicNotFound`] if `name` was never registered.
    /// - [`TopicsError::TopicAlreadyInactive`] if the topic is already inactive.
    pub fn deactivate(env: Env, caller: Address, name: Symbol) -> Result<(), TopicsError> {
        caller.require_auth();
        let admin = Self::require_admin(&env)?;
        if caller != admin {
            return Err(TopicsError::Unauthorized);
        }

        let key = StorageKey::Topic(name.clone());
        let mut record: TopicRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(TopicsError::TopicNotFound)?;

        if !record.active {
            return Err(TopicsError::TopicAlreadyInactive);
        }

        record.active = false;
        env.storage().persistent().set(&key, &record);
        Self::bump_instance(&env);
        env.events()
            .publish((events::event_topic_deactivated(&env),), name);
        Ok(())
    }

    /// Reactivate a previously deactivated topic, allowing metered calls again.
    ///
    /// Only the admin may reactivate topics.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::Unauthorized`] if `caller` is not the admin.
    /// - [`TopicsError::TopicNotFound`] if `name` was never registered.
    pub fn reactivate(env: Env, caller: Address, name: Symbol) -> Result<(), TopicsError> {
        caller.require_auth();
        let admin = Self::require_admin(&env)?;
        if caller != admin {
            return Err(TopicsError::Unauthorized);
        }

        let key = StorageKey::Topic(name.clone());
        let mut record: TopicRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(TopicsError::TopicNotFound)?;

        record.active = true;
        env.storage().persistent().set(&key, &record);
        Self::bump_instance(&env);
        env.events()
            .publish((events::event_topic_reactivated(&env),), name);
        Ok(())
    }

    /// Transfer ownership of a registered topic to a new address.
    ///
    /// Only the admin may change topic ownership.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::Unauthorized`] if `caller` is not the admin.
    /// - [`TopicsError::TopicNotFound`] if `name` was never registered.
    /// - [`TopicsError::SameOwner`] if `new_owner` equals the current owner.
    pub fn set_topic_owner(
        env: Env,
        caller: Address,
        name: Symbol,
        new_owner: Address,
    ) -> Result<(), TopicsError> {
        caller.require_auth();
        let admin = Self::require_admin(&env)?;
        if caller != admin {
            return Err(TopicsError::Unauthorized);
        }

        let key = StorageKey::Topic(name.clone());
        let mut record: TopicRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(TopicsError::TopicNotFound)?;

        if record.owner == new_owner {
            return Err(TopicsError::SameOwner);
        }

        let old_owner = record.owner.clone();
        record.owner = new_owner.clone();
        env.storage().persistent().set(&key, &record);
        Self::bump_instance(&env);
        env.events().publish(
            (events::event_topic_owner_changed(&env),),
            (old_owner, new_owner, name),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Read-only views
    // -----------------------------------------------------------------------

    /// Retrieve a registered topic record.
    ///
    /// Bumps the persistent entry TTL on every read to avoid archival pressure.
    /// No auth required.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::TopicNotFound`] if `name` was never registered.
    pub fn get_topic(env: Env, name: Symbol) -> Result<TopicRecord, TopicsError> {
        Self::require_admin(&env)?;
        let key = StorageKey::Topic(name);
        // Bump the persistent entry TTL on every hot read so archival pressure
        // is avoided for frequently-queried topics.
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_THRESHOLD, PERSISTENT_BUMP);
        }
        env.storage()
            .persistent()
            .get(&key)
            .ok_or(TopicsError::TopicNotFound)
    }

    /// Check whether a topic is currently active.
    ///
    /// No auth required.
    ///
    /// # Errors
    /// - [`TopicsError::NotInitialized`] if `init` has not been called.
    /// - [`TopicsError::TopicNotFound`] if `name` was never registered.
    pub fn is_active(env: Env, name: Symbol) -> Result<bool, TopicsError> {
        Self::require_admin(&env)?;
        let key = StorageKey::Topic(name);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_THRESHOLD, PERSISTENT_BUMP);
        }
        let record: TopicRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(TopicsError::TopicNotFound)?;
        Ok(record.active)
    }

    /// Total number of topics ever registered (monotonically increasing).
    ///
    /// No auth required.
    ///
    /// # Errors
    /// Returns [`TopicsError::NotInitialized`] if `init` has not been called.
    pub fn topic_count(env: Env) -> Result<u32, TopicsError> {
        Self::require_admin(&env)?;
        Ok(env
            .storage()
            .instance()
            .get(&StorageKey::TopicCount)
            .unwrap_or(0))
    }

    /// Return the admin address.
    ///
    /// No auth required.
    ///
    /// # Errors
    /// Returns [`TopicsError::NotInitialized`] if `init` has not been called.
    pub fn get_admin(env: Env) -> Result<Address, TopicsError> {
        Self::require_admin(&env)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Fetch the admin or return `NotInitialized`.
    fn require_admin(env: &Env) -> Result<Address, TopicsError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(TopicsError::NotInitialized)
    }

    /// Bump instance storage TTL to keep the contract live.
    fn bump_instance(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }
}
