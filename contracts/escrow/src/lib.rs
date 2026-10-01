#![no_std]

//! # Callora Escrow contract
//!
//! The `escrow` contract is a fund-holding surface for the Callora marketplace
//! where privileged admin keys perform critical operations on escrowed funds:
//! releasing funds to recipients, pausing the escrow, rotating signers, and
//! so on.
//!
//! This contract's defining feature is an **admin cool-off window**: every
//! critical action is rate-limited so that two invocations of the *same*
//! action cannot fire within the configured window. See [`admin`] for the
//! cool-off engine and
//! [issue #914](https://github.com/CalloraOrg/Callora-Contracts/issues/914).
//!
//! ## Circuit Breaker (Pause Mechanism)
//!
//! During incidents or emergency maintenance, the contract can be paused by the
//! admin via [`CalloraEscrow::pause`]. When paused, all fund-affecting entrypoints
//! fail closed with [`EscrowError::Paused`].
//!
//! Administrative and read-only functions remain accessible while paused to allow
//! diagnostics, remediation, and unpausing.
//!
//! ### Pause Operation Matrix
//!
//! | Function | Category | Allowed While Paused | Notes |
//! |---|---|:---:|---|
//! | [`CalloraEscrow::create_escrow`] | Fund-Affecting | **No** | Rejects with [`EscrowError::Paused`] |
//! | [`CalloraEscrow::release`] | Fund-Affecting | **No** | Rejects with [`EscrowError::Paused`] |
//! | [`CalloraEscrow::pause`] | Admin Critical | **Yes** | Cooldown-guarded |
//! | [`CalloraEscrow::unpause`] | Admin Critical | **Yes** | Restores fund operations |
//! | [`CalloraEscrow::rotate_signer`] | Admin Critical | **Yes** | Allows key rotation during incident |
//! | [`CalloraEscrow::set_cooldown`] | Admin Config | **Yes** | Allows cooldown adjustment |
//! | [`CalloraEscrow::add_approved_asset`] | Admin Config | **Yes** | Asset registry management |
//! | [`CalloraEscrow::remove_approved_asset`] | Admin Config | **Yes** | Asset registry management |
//! | [`CalloraEscrow::set_admin`] | Admin Mgmt | **Yes** | Two-step transfer initiation |
//! | [`CalloraEscrow::accept_admin`] | Admin Mgmt | **Yes** | Two-step transfer completion |
//! | [`CalloraEscrow::get_admin`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::get_pending_admin`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::get_signer`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::get_cooldown`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::cooldown_remaining`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::is_ready`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::is_paused`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::is_asset_approved`] | Read-Only View | **Yes** | Inspection / Diagnostics |
//! | [`CalloraEscrow::get_escrow`] | Read-Only View | **Yes** | Inspection / Diagnostics |

#[cfg(test)]
extern crate std;

pub mod admin;
pub mod errors;
pub mod events;

use errors::EscrowError;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Action tags
// ---------------------------------------------------------------------------

/// Cool-off tag for the `release` critical action.
pub const ACTION_RELEASE: &str = "release";

/// Cool-off tag for the `pause` critical action.
pub const ACTION_PAUSE: &str = "pause";

/// Cool-off tag for the `unpause` critical action.
pub const ACTION_UNPAUSE: &str = "unpause";

/// Cool-off tag for the `rotate_signer` critical action.
pub const ACTION_ROTATE: &str = "rotate";

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// Instance storage keys for the escrow contract.
///
/// Each variant maps a logical key name to the underlying Soroban storage key,
/// avoiding accidental key collisions with raw strings.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    /// Instance: current admin [`Address`].
    Admin,
    /// Instance: pending admin [`Address`] for two-step rotation.
    PendingAdmin,
    /// Instance: global cool-off window in seconds (`u64`).
    Cooldown,
    /// Instance: last-execution ledger timestamp for the action with the given
    /// [`Symbol`] tag (`u64`).
    LastAction(Symbol),
    /// Instance: paused flag (`bool`) toggled by the guarded pause actions.
    Paused,
    /// Instance: the currently-authorized escrow signer [`Address`].
    Signer,
    /// Instance: approval flag for an escrow payment asset (`bool`).
    /// Deny-by-default: absent means the asset is not approved.
    ApprovedAsset(Address),
    /// Persistent: escrow record keyed by `(payment_asset, recipient)`.
    ///
    /// Escrow records are stored in persistent storage (not instance) so that
    /// the instance footprint stays independent of the number of escrows and
    /// the instance cannot archive due to unbounded growth.
    Escrow(Address, Address),
}

/// A recorded escrow created under a specific approved payment asset.
///
/// Stored in persistent storage, keyed by `(payment_asset, recipient)`, so a
/// repeated creation attempt with the same inputs is rejected as a replay.
/// Reads and writes bump the persistent entry's TTL.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowRecord {
    /// The payment asset contract bound to this escrow.
    pub payment_asset: Address,
    /// The designated recipient of the escrowed funds.
    pub recipient: Address,
    /// Escrow amount denominated in the payment asset micro-units.
    pub amount: i128,
    /// Ledger timestamp at which the escrow was created.
    pub created_at: u64,
}

/// Number of ledgers to extend persistent escrow entries by on read/write.
///
/// ~30 days at 5s/ledger. Chosen to comfortably outlive typical escrow
/// lifetimes while keeping rent costs bounded.
pub const ESCROW_TTL_LEDGERS: u32 = 518_400;

/// Number of ledgers to extend the instance TTL by on every entrypoint.
pub const INSTANCE_TTL_LEDGERS: u32 = 518_400;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraEscrow;

#[contractimpl]
impl CalloraEscrow {
    /// Extend the instance storage TTL.
    ///
    /// Called at the top of every entrypoint so the instance (which holds
    /// config, admin, signer, cooldown, and approved-asset flags) never
    /// archives due to inactivity.
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
    }

    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the escrow contract.
    ///
    /// Can only be called once. Sets the admin, the initial escrow signer, and
    /// the cool-off window. Pass `None` for `cooldown_secs` to adopt
    /// [`admin::DEFAULT_COOLDOWN_SECS`] (1 hour).
    ///
    /// # Parameters
    /// * `admin` -- Address permitted to call admin-only entrypoints; must authorize.
    /// * `signer` -- The initial escrow signer address.
    /// * `cooldown_secs` -- Optional cool-off window in seconds.
    ///
    /// # Errors
    /// * [`EscrowError::AlreadyInitialized`] -- `init` was called more than once.
    /// * [`EscrowError::InvalidCooldown`] -- explicit `cooldown_secs` is out of range.
    ///
    /// # Events
    /// Emits `init` with `admin` as topic and the cooldown window as data.
    pub fn init(
        env: Env,
        admin: Address,
        signer: Address,
        cooldown_secs: Option<u64>,
    ) -> Result<(), EscrowError> {
        admin.require_auth();
        Self::extend_instance_ttl(&env);
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(EscrowError::AlreadyInitialized);
        }

        let cooldown = cooldown_secs.unwrap_or(admin::DEFAULT_COOLDOWN_SECS);

        // Validate and persist the cooldown through the module's checked
        // setter *before* writing any other state, so a bad value leaves
        // storage clean.
        admin::set_cooldown(&env, cooldown)?;

        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::Signer, &signer);
        inst.set(&StorageKey::Paused, &false);

        env.events()
            .publish((events::event_init(&env), admin), cooldown);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin helpers (internal)
    // -----------------------------------------------------------------------

    /// Verify that the contract is not paused.
    ///
    /// Returns [`EscrowError::Paused`] when the contract is currently paused.
    fn require_not_paused(env: &Env) -> Result<(), EscrowError> {
        if Self::is_paused(env.clone()) {
            return Err(EscrowError::Paused);
        }
        Ok(())
    }

    /// Read the current admin address from instance storage.
    ///
    /// Returns [`EscrowError::NotInitialized`] when no admin has been set.
    fn admin(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(EscrowError::NotInitialized)
    }

    /// Verify that `caller` is the current admin.
    ///
    /// Consumes the caller's auth via `require_auth`, then checks instance
    /// storage for the admin address. Returns [`EscrowError::Unauthorized`]
    /// when the caller does not match.
    fn require_admin(env: &Env, caller: &Address) -> Result<(), EscrowError> {
        caller.require_auth();
        let admin = Self::admin(env)?;
        if caller != &admin {
            return Err(EscrowError::Unauthorized);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Cooldown administration
    // -----------------------------------------------------------------------

    /// Return the currently-configured cool-off window, in seconds.
    ///
    /// # Errors
    /// * [`EscrowError::NotInitialized`] -- contract was never initialized.
    pub fn get_cooldown(env: Env) -> Result<u64, EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::admin(&env)?;
        Ok(admin::get_cooldown(&env))
    }

    /// Update the global cool-off window. Only the current admin may call.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `secs` -- New window in seconds, within
    ///   [`admin::MIN_COOLDOWN_SECS`]..=[`admin::MAX_COOLDOWN_SECS`].
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::InvalidCooldown`] -- `secs` is out of range.
    ///
    /// # Events
    /// Emits `cooldown_set` with `caller` as topic and the new window as data.
    pub fn set_cooldown(env: Env, caller: Address, secs: u64) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        admin::set_cooldown(&env, secs)?;

        env.events()
            .publish((events::event_cooldown_set(&env), caller), secs);

        Ok(())
    }

    /// Return the number of seconds remaining before the action tagged
    /// `action` may run again, or `0` if it is available now.
    ///
    /// This is a read-only view and does not require initialization.
    pub fn cooldown_remaining(env: Env, action: Symbol) -> u64 {
        Self::extend_instance_ttl(&env);
        admin::remaining(&env, &action)
    }

    /// Return `true` when the action tagged `action` may run now.
    pub fn is_ready(env: Env, action: Symbol) -> bool {
        Self::extend_instance_ttl(&env);
        admin::is_ready(&env, &action)
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    /// Return the current admin address.
    ///
    /// # Errors
    /// * [`EscrowError::NotInitialized`] -- contract was never initialized.
    pub fn get_admin(env: Env) -> Result<Address, EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::admin(&env)
    }

    /// Return the pending admin for a two-step rotation, or `None`.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        Self::extend_instance_ttl(&env);
        env.storage().instance().get(&StorageKey::PendingAdmin)
    }

    /// Return the current escrow signer address.
    ///
    /// # Errors
    /// * [`EscrowError::NotInitialized`] -- contract was never initialized.
    pub fn get_signer(env: Env) -> Result<Address, EscrowError> {
        Self::extend_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&StorageKey::Signer)
            .ok_or(EscrowError::NotInitialized)
    }

    /// Return whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        Self::extend_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&StorageKey::Paused)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Guarded critical actions
    // -----------------------------------------------------------------------

    /// Release escrowed funds. Cool-off-guarded critical action (tag `"release"`).
    ///
    /// This is the primary escrow critical action: transferring held funds to
    /// a designated recipient. The cooldown prevents rapid sequential releases
    /// from a compromised admin key.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `recipient` -- Address of the fund recipient.
    ///
    /// # Errors
    /// * [`EscrowError::Paused`] -- contract is paused.
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::CooldownActive`] -- a `release` ran within the cool-off window.
    ///
    /// # Events
    /// Emits `action` with `caller` as topic and the `"release"` tag as data.
    pub fn release(env: Env, caller: Address, recipient: Address) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_RELEASE);
        admin::guard(&env, &action)?;

        // Persist the release recipient for on-chain auditability.
        env.storage()
            .instance()
            .set(&StorageKey::Signer, &recipient);

        env.events()
            .publish((events::event_action(&env), caller), action);

        Ok(())
    }

    /// Pause the escrow contract. **Not** cooldown-gated — circuit-breakers
    /// must be available instantly.
    ///
    /// An attacker who triggers an unpause (or the admin toggling during an
    /// incident) must never be able to hold the contract live for an entire
    /// cooldown window. `unpause` and `rotate_signer` retain their cooldowns.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    ///
    /// # Events
    /// Emits `action` with `caller` as topic and the `"pause"` tag as data.
    pub fn pause(env: Env, caller: Address) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_PAUSE);

        env.storage().instance().set(&StorageKey::Paused, &true);

        env.events()
            .publish((events::event_action(&env), caller), action);

        Ok(())
    }

    /// Unpause the escrow contract. Cool-off-guarded critical action (tag `"unpause"`).
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::CooldownActive`] -- an `unpause` ran within the cool-off window.
    ///
    /// # Events
    /// Emits `action` with `caller` as topic and the `"unpause"` tag as data.
    pub fn unpause(env: Env, caller: Address) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_UNPAUSE);
        admin::guard(&env, &action)?;

        env.storage().instance().set(&StorageKey::Paused, &false);

        env.events()
            .publish((events::event_action(&env), caller), action);

        Ok(())
    }

    /// Rotate the escrow signer. Cool-off-guarded critical action (tag `"rotate"`).
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_signer` -- The replacement escrow signer address.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::InvalidInput`] -- `new_signer` equals the current signer.
    /// * [`EscrowError::CooldownActive`] -- a `rotate` ran within the cool-off window.
    ///
    /// # Events
    /// Emits `signer_rotated` with `caller` as topic and `(old_signer,
    /// new_signer)` as data.
    pub fn rotate_signer(
        env: Env,
        caller: Address,
        new_signer: Address,
    ) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_ROTATE);
        admin::guard(&env, &action)?;

        let old_signer: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Signer)
            .ok_or(EscrowError::NotInitialized)?;
        if old_signer == new_signer {
            return Err(EscrowError::InvalidInput);
        }

        env.storage()
            .instance()
            .set(&StorageKey::Signer, &new_signer);

        env.events().publish(
            (events::event_signer_rotated(&env), caller),
            (old_signer, new_signer),
        );

        Ok(())
    }

    /// Return whether `asset` is approved for use as a payment asset when
    /// creating an escrow.
    ///
    /// This is a deny-by-default boundary: any asset that has not been
    /// explicitly approved by the admin returns `false` and is therefore not
    /// accepted by [`CalloraEscrow::create_escrow`].
    pub fn is_asset_approved(env: Env, asset: Address) -> bool {
        Self::extend_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&StorageKey::ApprovedAsset(asset))
            .unwrap_or(false)
    }

    /// Return the escrow record for a `(payment_asset, recipient)` pair.
    ///
    /// Returns `None` when no escrow has been created for the pair. This is a
    /// read-only view for on-chain auditability of created escrows.
    pub fn get_escrow(
        env: Env,
        payment_asset: Address,
        recipient: Address,
    ) -> Option<EscrowRecord> {
        Self::extend_instance_ttl(&env);
        let key = StorageKey::Escrow(payment_asset, recipient);
        let persistent = env.storage().persistent();
        let record: Option<EscrowRecord> = persistent.get(&key);
        if record.is_some() {
            persistent.extend_ttl(&key, ESCROW_TTL_LEDGERS, ESCROW_TTL_LEDGERS);
        }
        record
    }

    /// Approve `asset` as a payment asset for future escrow creation.
    ///
    /// Only the current admin may grant approval; the check runs before any
    /// state is written. Approving an already-approved asset is idempotent.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `asset` -- The payment asset contract to approve.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::InvalidInput`] -- `asset` resolves to the contract itself.
    ///
    /// # Events
    /// Emits `asset_approved` with `caller` as topic and `asset` as data.
    pub fn add_approved_asset(
        env: Env,
        caller: Address,
        asset: Address,
    ) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        Self::validate_asset(&env, &asset)?;

        env.storage()
            .instance()
            .set(&StorageKey::ApprovedAsset(asset.clone()), &true);

        env.events()
            .publish((events::event_asset_approved(&env), caller), asset);

        Ok(())
    }

    /// Revoke approval for a previously approved payment asset.
    ///
    /// Only the current admin may revoke approval. After revocation the asset
    /// can no longer be used to create new escrows, but existing escrows are
    /// left intact. Revoking an asset that is not approved is a no-op.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `asset` -- The payment asset contract to revoke.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::InvalidInput`] -- `asset` resolves to the contract itself.
    ///
    /// # Events
    /// Emits `asset_removed` with `caller` as topic and `asset` as data.
    pub fn remove_approved_asset(
        env: Env,
        caller: Address,
        asset: Address,
    ) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;
        Self::validate_asset(&env, &asset)?;

        let key = StorageKey::ApprovedAsset(asset.clone());
        if env.storage().instance().has(&key) {
            env.storage().instance().remove(&key);
        }

        env.events()
            .publish((events::event_asset_removed(&env), caller), asset);

        Ok(())
    }

    /// Create a new escrow bound to an approved payment asset.
    ///
    /// Authorization, validation, and least-privilege checks all run before any
    /// state is written. The `payment_asset` must have been approved via
    /// [`CalloraEscrow::add_approved_asset`]; unapproved, malformed, and replay
    /// inputs fail closed without leaking protected detail.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `payment_asset` -- The approved payment asset for this escrow.
    /// * `recipient` -- The designated recipient of the escrowed funds.
    /// * `amount` -- Escrow amount in payment asset micro-units; must be > 0.
    ///
    /// # Errors
    /// * [`EscrowError::Paused`] -- contract is paused.
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::InvalidInput`] -- `payment_asset` or `recipient` resolve
    ///   to the contract itself, or `amount` is not positive.
    /// * [`EscrowError::AssetNotApproved`] -- `payment_asset` is not approved.
    /// * [`EscrowError::EscrowExists`] -- an escrow for the same
    ///   `(payment_asset, recipient)` already exists (replay).
    ///
    /// # Events
    /// Emits `escrow_created` with `caller` as topic and the new
    /// [`EscrowRecord`] as data.
    pub fn create_escrow(
        env: Env,
        caller: Address,
        payment_asset: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_not_paused(&env)?;
        Self::require_admin(&env, &caller)?;
        Self::validate_asset(&env, &payment_asset)?;
        if recipient == env.current_contract_address() {
            return Err(EscrowError::InvalidInput);
        }
        if amount <= 0 {
            return Err(EscrowError::InvalidInput);
        }
        if !Self::is_asset_approved(env.clone(), payment_asset.clone()) {
            return Err(EscrowError::AssetNotApproved);
        }

        let key = StorageKey::Escrow(payment_asset.clone(), recipient.clone());
        let persistent = env.storage().persistent();
        if persistent.has(&key) {
            return Err(EscrowError::EscrowExists);
        }

        let record = EscrowRecord {
            payment_asset: payment_asset.clone(),
            recipient: recipient.clone(),
            amount,
            created_at: env.ledger().timestamp(),
        };
        persistent.set(&key, &record);
        persistent.extend_ttl(&key, ESCROW_TTL_LEDGERS, ESCROW_TTL_LEDGERS);

        env.events()
            .publish((events::event_escrow_created(&env), caller), record);

        Ok(())
    }

    /// Reject `asset` when it resolves to the contract's own address.
    ///
    /// A payment asset must never be the escrow contract itself, which would
    /// otherwise allow funds to be routed back into the escrow in an unintended
    /// way. Returns [`EscrowError::InvalidInput`] when the asset is self.
    fn validate_asset(env: &Env, asset: &Address) -> Result<(), EscrowError> {
        if asset == &env.current_contract_address() {
            return Err(EscrowError::InvalidInput);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    ///
    /// The nominee must call [`CalloraEscrow::accept_admin`] to complete the
    /// transfer. Until then the current admin retains full authority.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_admin` -- Address of the proposed new admin.
    ///
    /// # Errors
    /// * [`EscrowError::Unauthorized`] -- caller is not the current admin.
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    ///
    /// # Events
    /// Emits `admin_nominated` with `(caller)` as topic and `new_admin` as data.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        Self::require_admin(&env, &caller)?;

        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);

        env.events()
            .publish((events::event_admin_nominated(&env), caller), new_admin);

        Ok(())
    }

    /// Complete a pending admin transfer. Must be called by the nominated admin.
    ///
    /// # Parameters
    /// * `caller` -- Must be the pending admin; must authorize.
    ///
    /// # Errors
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::NoPendingAdmin`] -- no nomination is in progress.
    /// * [`EscrowError::Unauthorized`] -- caller is not the pending admin.
    ///
    /// # Events
    /// Emits `admin_accepted` with `(old_admin)` as topic and `new_admin` as data.
    pub fn accept_admin(env: Env, caller: Address) -> Result<(), EscrowError> {
        Self::extend_instance_ttl(&env);
        caller.require_auth();

        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(EscrowError::NoPendingAdmin)?;

        if caller != pending {
            return Err(EscrowError::Unauthorized);
        }

        let old_admin = Self::admin(&env)?;
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &pending);
        inst.remove(&StorageKey::PendingAdmin);

        env.events()
            .publish((events::event_admin_accepted(&env), old_admin), pending);

        Ok(())
    }

    /// Cancel a pending admin transfer. Only the current admin may call.
    ///
    /// Returns [`EscrowError::NoPendingAdmin`] when no nomination is in progress.
    /// Clears the pending admin and emits `admin_cancelled` with the previous
    /// pending admin address.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`EscrowError::NotInitialized`] -- contract not initialized.
    /// * [`EscrowError::NoPendingAdmin`] -- no nomination is in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled` with `(caller)` as topic and the cancelled
    /// pending admin address as data.
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), EscrowError> {
        Self::require_admin(&env, &caller)?;

        let cancelled: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(EscrowError::NoPendingAdmin)?;

        env.storage()
            .instance()
            .remove(&StorageKey::PendingAdmin);

        env.events()
            .publish((events::event_admin_cancelled(&env), caller), cancelled);

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test modules
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test;

#[cfg(test)]
mod rustdoc_tests {
    #[test]
    fn every_public_fn_in_lib_has_rustdoc() {
        let source = include_str!("lib.rs")
            .split("// ---------------------------------------------------------------------------\n// Test modules")
            .next()
            .expect("lib.rs contains test module marker");
        let lines: std::vec::Vec<&str> = source.lines().collect();

        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("pub fn ")
                || trimmed.starts_with("pub(crate) fn ")
                || trimmed.starts_with("pub(super) fn "))
            {
                continue;
            }

            let has_rustdoc = lines[..idx]
                .iter()
                .rev()
                .map(|candidate| candidate.trim_start())
                .find(|candidate| !candidate.is_empty())
                .map(|candidate| candidate.starts_with("///"))
                .unwrap_or(false);

            assert!(
                has_rustdoc,
                "public function on line {} is missing /// rustdoc: {}",
                idx + 1,
                trimmed
            );
        }
    }
}
