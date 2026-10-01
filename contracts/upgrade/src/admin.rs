//! Admin module with a cool-off window between upgrade actions.
//!
//! Enforces a cooldown period between critical upgrade actions to prevent
//! rapid abuse. This implements the requirements for the GrantFox FWC26 campaign.
//!
//! `set_cooldown` is restricted to the admin stored via [`init_admin`] and the
//! new value must lie in `MIN_COOLDOWN_SECONDS..=MAX_COOLDOWN_SECONDS`.
//!
//! ## Events
//!
//! | Function                  | Topic              | Topics                          | Data                              |
//! |---------------------------|--------------------|---------------------------------|-----------------------------------|
//! | `check_and_record_upgrade`| `upgrade_started`  | `(topic, caller)`               | `(current_timestamp, cooldown)`   |
//! | `check_and_record_upgrade`| `upgrade_recorded` | `(topic, caller)`               | `recorded_timestamp`              |
//! | `set_cooldown`            | `cooldown_set`     | `(topic, caller)`               | `(old_cooldown_secs, new_cooldown_secs)` |

use soroban_sdk::{Address, Env, Symbol};

use crate::errors::UpgradeError;
use crate::events;

const LAST_UPGRADE_TIME_KEY: &str = "last_upg_tm";
const UPGRADE_COOLDOWN_KEY: &str = "upg_cooldown";
const ADMIN_KEY: &str = "upg_admin";

/// Default cooldown is 24 hours (86400 seconds).
pub const DEFAULT_COOLDOWN_SECONDS: u64 = 86400;

/// Minimum allowed cooldown: 1 hour (3600 seconds).
///
/// A floor prevents the admin (or a compromised admin key) from effectively
/// disabling the cool-off window by setting it to 0 or a trivially small value.
pub const MIN_COOLDOWN_SECONDS: u64 = 3_600;

/// Maximum allowed cooldown: 30 days (2,592,000 seconds).
///
/// A ceiling prevents a bad value from locking upgrades out for an unbounded time.
pub const MAX_COOLDOWN_SECONDS: u64 = 2_592_000;

/// Store the admin address. May be called exactly once.
///
/// Requires auth from `admin`, so nobody can install an address they don't control.
/// The host contract should call this from its own initializer, in the same
/// transaction as deployment, so the first-call race never applies.
///
/// # Errors
/// `AlreadyInitialized` if an admin is already stored.
pub fn init_admin(env: &Env, admin: &Address) -> Result<(), UpgradeError> {
    admin.require_auth();
    let key = Symbol::new(env, ADMIN_KEY);
    if env.storage().instance().has(&key) {
        return Err(UpgradeError::AlreadyInitialized);
    }
    env.storage().instance().set(&key, admin);
    Ok(())
}

/// Return the stored admin, if any.
pub fn get_admin(env: &Env) -> Option<Address> {
    env.storage()
        .instance()
        .get::<_, Address>(&Symbol::new(env, ADMIN_KEY))
}

/// Set the cooldown window (in seconds). Admin only.
///
/// Checks, in order: caller auth, admin is initialized, caller is the admin,
/// cooldown within bounds. Any failure returns an error and leaves state untouched.
///
/// # Errors
/// - `NotInitialized` if no admin is stored.
/// - `Unauthorized` if `caller` is not the stored admin.
/// - `InvalidCooldown` if `cooldown` is outside `MIN_COOLDOWN_SECONDS..=MAX_COOLDOWN_SECONDS`.
///
/// # Events
/// Emits `cooldown_set` with `caller` as topic and `(old_cooldown, new_cooldown)` as data.
/// `old_cooldown` is `DEFAULT_COOLDOWN_SECONDS` if none was set before.
pub fn set_cooldown(env: &Env, caller: &Address, cooldown: u64) -> Result<(), UpgradeError> {
    caller.require_auth();

    let admin = get_admin(env).ok_or(UpgradeError::NotInitialized)?;
    if *caller != admin {
        return Err(UpgradeError::Unauthorized);
    }

    if !(MIN_COOLDOWN_SECONDS..=MAX_COOLDOWN_SECONDS).contains(&cooldown) {
        return Err(UpgradeError::InvalidCooldown);
    }

    let old_cooldown = get_cooldown(env);
    env.storage()
        .instance()
        .set(&Symbol::new(env, UPGRADE_COOLDOWN_KEY), &cooldown);

    env.events().publish(
        (events::event_cooldown_set(env), caller),
        (old_cooldown, cooldown),
    );

    Ok(())
}

/// Retrieve the current cooldown window.
///
/// Returns the configured cooldown window, or `DEFAULT_COOLDOWN_SECONDS` if not set.
pub fn get_cooldown(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get::<_, u64>(&Symbol::new(env, UPGRADE_COOLDOWN_KEY))
        .unwrap_or(DEFAULT_COOLDOWN_SECONDS)
}

/// Verify that the cooldown period has elapsed since the last upgrade.
///
/// Requires auth from the caller. Returns `Ok(())` if the cooldown has elapsed
/// or if this is the first upgrade. Otherwise, returns `UpgradeError::CooldownNotElapsed`.
/// Also updates the last upgrade time to the current ledger timestamp upon success.
///
/// # Events
/// On success, emits two events in order:
/// 1. `upgrade_started` — topics `(upgrade_started, caller)`, data `(current_timestamp, cooldown)`.
///    Signals that auth passed and the cooldown constraint was satisfied.
/// 2. `upgrade_recorded` — topics `(upgrade_recorded, caller)`, data `recorded_timestamp`.
///    Signals that the new baseline timestamp has been persisted to storage.
///
/// No events are emitted when the call returns `Err`.
pub fn check_and_record_upgrade(env: &Env, caller: &Address) -> Result<(), UpgradeError> {
    caller.require_auth();

    let current_time = env.ledger().timestamp();
    let last_time = env
        .storage()
        .instance()
        .get::<_, u64>(&Symbol::new(env, LAST_UPGRADE_TIME_KEY))
        .unwrap_or(0);

    let cooldown = get_cooldown(env);

    if last_time != 0 {
        let elapsed = current_time
            .checked_sub(last_time)
            .ok_or(UpgradeError::Overflow)?;
        if elapsed < cooldown {
            return Err(UpgradeError::CooldownNotElapsed);
        }
    }

    // Auth passed and cooldown satisfied — signal the start of the upgrade.
    env.events().publish(
        (events::event_upgrade_started(env), caller),
        (current_time, cooldown),
    );

    env.storage()
        .instance()
        .set(&Symbol::new(env, LAST_UPGRADE_TIME_KEY), &current_time);

    // Timestamp is now persisted — confirm the record.
    env.events()
        .publish((events::event_upgrade_recorded(env), caller), current_time);

    Ok(())
}
