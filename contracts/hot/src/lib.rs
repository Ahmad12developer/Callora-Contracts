#![no_std]

//! # Callora Hot contract
//!
//! The `hot` contract is the operational "hot" control surface for the Callora
//! marketplace: the place where privileged admin keys perform time-sensitive,
//! high-impact operations. Because those keys are, by definition, kept online
//! ("hot"), they are the most exposed to compromise.
//!
//! This contract's defining feature is an **admin cool-off window**: every
//! critical action is rate-limited so that two invocations of the *same* action
//! cannot fire within the configured window. See [`admin`] for the cool-off
//! engine and [issue #743](https://github.com/CalloraOrg/Callora-Contracts/issues/743).

#[cfg(test)]
extern crate std;

pub mod admin;
pub mod errors;
pub mod events;
pub mod pause;

use errors::HotError;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Action tags
// ---------------------------------------------------------------------------

/// Action tag for `pause`. Retained for API compatibility (fuzz targets,
/// benches, and `cooldown_remaining` / `is_ready` callers); `pause` itself is
/// no longer cooldown-gated, so this slot is never armed by `pause`.
pub const ACTION_PAUSE: &str = "pause";

/// Cool-off tag for the `unpause` critical action.
pub const ACTION_UNPAUSE: &str = "unpause";

/// Cool-off tag for the `rotate_signer` critical action.
pub const ACTION_ROTATE: &str = "rotate";

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// Instance storage keys for the hot contract.
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
    /// Instance: the currently-authorized hot signer [`Address`].
    Signer,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraHot;

#[contractimpl]
impl CalloraHot {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the hot contract.
    ///
    /// Can only be called once. Sets the admin, the initial hot signer, and the
    /// cool-off window. Pass `None` for `cooldown_secs` to adopt
    /// [`admin::DEFAULT_COOLDOWN_SECS`].
    ///
    /// # Parameters
    /// * `admin` -- Address permitted to call admin-only entrypoints; must authorize.
    /// * `signer` -- The initial hot signer address.
    /// * `cooldown_secs` -- Optional cool-off window in seconds.
    ///
    /// # Errors
    /// * [`HotError::AlreadyInitialized`] -- `init` was called more than once.
    /// * [`HotError::InvalidCooldown`] -- explicit `cooldown_secs` is out of range.
    ///
    /// # Events
    /// Emits `init` with `admin` as topic and the cooldown window as data.
    pub fn init(
        env: Env,
        admin: Address,
        signer: Address,
        cooldown_secs: Option<u64>,
    ) -> Result<(), HotError> {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(HotError::AlreadyInitialized);
        }

        let cooldown = cooldown_secs.unwrap_or(admin::DEFAULT_COOLDOWN_SECS);

        // Validate and persist the cooldown through the module's checked setter
        // *before* writing any other state, so a bad value leaves storage clean.
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

    /// Read the current admin address from instance storage.
    ///
    /// Returns [`HotError::NotInitialized`] when no admin has been set.
    fn admin(env: &Env) -> Result<Address, HotError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(HotError::NotInitialized)
    }

    /// Verify that `caller` is the current admin.
    ///
    /// Consumes the caller's auth via `require_auth`, then checks instance
    /// storage for the admin address. Returns [`HotError::Unauthorized`] when
    /// the caller does not match.
    fn require_admin(env: &Env, caller: &Address) -> Result<(), HotError> {
        caller.require_auth();
        let admin = Self::admin(env)?;
        if caller != &admin {
            return Err(HotError::Unauthorized);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Cooldown administration
    // -----------------------------------------------------------------------

    /// Return the currently-configured cool-off window, in seconds.
    ///
    /// # Errors
    /// * [`HotError::NotInitialized`] -- contract was never initialized.
    pub fn get_cooldown(env: Env) -> Result<u64, HotError> {
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
    /// * [`HotError::Unauthorized`] -- caller is not the current admin.
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::InvalidCooldown`] -- `secs` is out of range.
    ///
    /// # Events
    /// Emits `cooldown_set` with `caller` as topic and the new window as data.
    pub fn set_cooldown(env: Env, caller: Address, secs: u64) -> Result<(), HotError> {
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
        admin::remaining(&env, &action)
    }

    /// Return `true` when the action tagged `action` may run now.
    pub fn is_ready(env: Env, action: Symbol) -> bool {
        admin::is_ready(&env, &action)
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    /// Return the current admin address.
    ///
    /// # Errors
    /// * [`HotError::NotInitialized`] -- contract was never initialized.
    pub fn get_admin(env: Env) -> Result<Address, HotError> {
        Self::admin(&env)
    }

    /// Return the pending admin for a two-step rotation, or `None`.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::PendingAdmin)
    }

    /// Return the current hot signer address.
    ///
    /// # Errors
    /// * [`HotError::NotInitialized`] -- contract was never initialized.
    pub fn get_signer(env: Env) -> Result<Address, HotError> {
        env.storage()
            .instance()
            .get(&StorageKey::Signer)
            .ok_or(HotError::NotInitialized)
    }

    /// Return whether the contract is currently paused.
    pub fn is_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    // -----------------------------------------------------------------------
    // Guarded critical actions
    // -----------------------------------------------------------------------

    /// Pause the contract. **Not** cooldown-gated — circuit-breakers must be
    /// available instantly.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`HotError::Unauthorized`] -- caller is not the current admin.
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::AlreadyPaused`] -- the contract is already paused.
    ///
    /// # Events
    /// Emits `paused` with `caller` as topic.
    pub fn pause(env: Env, caller: Address) -> Result<(), HotError> {
        Self::require_admin(&env, &caller)?;
        pause::do_pause(&env, &caller)
    }

    /// Unpause the contract. Cool-off-guarded critical action (tag `"unpause"`).
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`HotError::Unauthorized`] -- caller is not the current admin.
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::NotPaused`] -- the contract is not currently paused.
    /// * [`HotError::CooldownActive`] -- an `unpause` ran within the cool-off window.
    ///
    /// # Events
    /// Emits `unpaused` with `caller` as topic.
    pub fn unpause(env: Env, caller: Address) -> Result<(), HotError> {
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_UNPAUSE);
        pause::do_unpause(&env, &caller, &action)
    }

    /// Rotate the hot signer. Cool-off-guarded critical action (tag `"rotate"`).
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_signer` -- The replacement hot signer address.
    ///
    /// # Errors
    /// * [`HotError::Unauthorized`] -- caller is not the current admin.
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::SameSigner`] -- `new_signer` equals the current signer.
    /// * [`HotError::CooldownActive`] -- a `rotate` ran within the cool-off window.
    ///
    /// # Events
    /// Emits `signer_rotated` with `caller` as topic and `(old_signer,
    /// new_signer)` as data.
    pub fn rotate_signer(env: Env, caller: Address, new_signer: Address) -> Result<(), HotError> {
        Self::require_admin(&env, &caller)?;
        let action = Symbol::new(&env, ACTION_ROTATE);
        admin::guard(&env, &action)?;

        let old_signer: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Signer)
            .ok_or(HotError::NotInitialized)?;

        if old_signer == new_signer {
            return Err(HotError::SameSigner);
        }

        env.storage()
            .instance()
            .set(&StorageKey::Signer, &new_signer);

        env.events()
            .publish((events::event_signer_rotated(&env), caller), (old_signer, new_signer));

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    ///
    /// The nominee must call [`CalloraHot::accept_admin`] to complete the
    /// transfer. Until then the current admin retains full authority.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_admin` -- Address of the proposed new admin.
    ///
    /// # Errors
    /// * [`HotError::Unauthorized`] -- caller is not the current admin.
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    ///
    /// # Events
    /// Emits `admin_nominated` with `(caller)` as topic and `new_admin` as data.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), HotError> {
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
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::NoPendingAdmin`] -- no nomination is in progress.
    /// * [`HotError::Unauthorized`] -- caller is not the pending admin.
    ///
    /// # Events
    /// Emits `admin_accepted` with `(old_admin)` as topic and `new_admin` as data.
    pub fn accept_admin(env: Env, caller: Address) -> Result<(), HotError> {
        caller.require_auth();

        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(HotError::NoPendingAdmin)?;

        if caller != pending {
            return Err(HotError::Unauthorized);
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
    /// Returns [`HotError::NoPendingAdmin`] when no nomination is in progress.
    /// Clears the pending admin and emits `admin_cancelled` with the previous
    /// pending admin address.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`HotError::NotInitialized`] -- contract not initialized.
    /// * [`HotError::NoPendingAdmin`] -- no nomination is in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled` with `(caller)` as topic and the cancelled
    /// pending admin address as data.
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), HotError> {
        Self::require_admin(&env, &caller)?;

        let cancelled: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(HotError::NoPendingAdmin)?;

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
