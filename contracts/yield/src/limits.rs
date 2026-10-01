//! Per-account state caps for the Callora Yield surface.
//!
//! # Problem
//!
//! Yield-bearing products (bets, positions, subscriptions) can be abused by a
//! single account farming thousands of small positions to drain fees or grief
//! indexers. The Callora yield surface therefore enforces per-account caps on
//! open-bets, open-positions, and active-subscriptions.
//!
//! # Architecture
//!
//! Two [`contracttype`] structs drive the limits surface:
//!
//! | Type           | Purpose                                                       |
//! |----------------|---------------------------------------------------------------|
//! | [`AccountLimits`] | The configured `(max_bets, max_positions, max_subscriptions)` caps for an account. |
//! | [`AccountState`]  | The current `(bets, positions, subscriptions)` counters for an account.          |
//!
//! Both types are `Clone + Debug + PartialEq` so off-chain consumers can
//! round-trip them through the SDK.
//!
//! # Storage Layout
//!
//! - [`AccountLimits`] are stored in **instance** storage under
//!   [`StorageKey::AccountLimits`]`(Address)`. They are sparse overrides; an
//!   account with no explicit override falls back to the global
//!   [`DEFAULT_LIMITS`] constant.
//!
//! - [`AccountState`] counters are stored in **persistent** storage under
//!   [`StorageKey::AccountState`]`(Address)`. Persistent storage lets the
//!   contract scale to many accounts (instance storage is small and shared
//!   with config) and keeps counters alive across the typical 7-day ledger
//!   archival window via TTL extensions on every increment/decrement.
//!
//! # Auth Model
//!
//! | Entrypoint                                       | Authorized by                              |
//! |--------------------------------------------------|-------------------------------------------|
//! | `init`, `set_admin`, `accept_admin`, `upgrade`   | admin (`caller == admin`)                |
//! | `cancel_admin_transfer`                          | admin (`caller == admin`)                |
//! | `set_default_limits`, `set_account_limits`, `clear_account_limits` | admin (`caller == admin`) |
//! | `place_bet`, `clear_bet`, `open_position`, `close_position`,       | caller (their own counter) |
//! | `subscribe`, `unsubscribe`                      | caller (their own counter)                |
//!
//! Read-only views (`get_admin`, `get_default_limits`, `get_account_limits`,
//! `get_account_state`, `can_*`) do **not** call `require_auth`.
//!
//! # Overflow Safety
//!
//! Every count mutation uses `u32::checked_add` / `u32::checked_sub`. The
//! outcomes are folded into typed [`YieldLimitError`] variants
//! ([`YieldLimitError::Overflow`] / [`YieldLimitError::CounterUnderflow`]) so
//! production code paths never invoke `unwrap()`.

use callora_storage_migration::StorageMigrationValidator;
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env};

use crate::errors::YieldLimitError;
use crate::events;

// ---------------------------------------------------------------------
// Constants — storage-layout versioning
// ---------------------------------------------------------------------

/// Storage-layout version recorded by [`StorageMigrationValidator`] for this
/// contract. Bumped whenever the on-ledger schema changes so the pre-upgrade
/// validation gate can enforce ordered, single-step migrations.
const STORAGE_MIGRATION_VERSION: u32 = 1;

/// Human-readable descriptor of the on-ledger storage layout.
///
/// The descriptor is hashed by [`storage_layout_hash`] and recorded at `init`
/// time and on every upgrade. Any change to the shape of [`AccountLimits`],
/// [`AccountState`], or the [`StorageKey`] enum MUST be reflected here so the
/// [`StorageMigrationValidator`] can detect an accidental silent layout change
/// (`StorageMigrationError::SilentLayoutChange`).
const STORAGE_LAYOUT_DESCRIPTOR: &str = concat!(
    "callora-yield-limits:v1:",
    "StorageKey[Admin,PendingAdmin,DefaultLimits,AccountLimits,AccountState,WasmVersion];",
    "AccountLimits[max_bets:u32,max_positions:u32,max_subscriptions:u32];",
    "AccountState[bets:u32,positions:u32,subscriptions:u32]"
);

/// Deterministic SHA-256 hash of the current on-ledger storage layout.
///
/// This is the "real layout hash" passed to [`StorageMigrationValidator`] on
/// both the pre-upgrade validation and the post-validation finalize call, so a
/// same-version re-deploy cannot silently change the layout.
pub fn storage_layout_hash(env: &Env) -> BytesN<32> {
    callora_storage_migration::layout_hash(env, STORAGE_LAYOUT_DESCRIPTOR)
}

// ---------------------------------------------------------------------
// Constants — TTL and defaults
// ---------------------------------------------------------------------

/// Per-day ledger count at a 5-second close cadence (matches vault).
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// TTL bump threshold for persistent storage keys (`AccountState`).
///
/// When the remaining TTL of the key falls below this value the contract
/// re-extends the TTL on every increment / decrement so account counters do
/// not silently archive.
pub const STATE_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 7;

/// TTL bump amount for persistent storage keys (`AccountState`).
pub const STATE_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 30;

/// TTL bump threshold for instance storage keys (`AccountLimits`,
/// `DefaultLimits`, config).
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

/// TTL bump amount for instance storage keys.
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 60;

/// Global default per-account limits applied when no explicit override exists
/// for an account.
///
/// # Default rationale
///
/// - `100` open bets caps the worst-case bet-farming at 100× per account.
/// - `50` open positions caps positions similarly while leaving headroom for
///   legitimate power-users.
/// - `20` active subscriptions caps subscription-farming while still
///   allowing routine yield-vault subscriptions.
///
/// These values are intentionally conservative so the contract is safe-by-default
/// even before the admin sets global defaults via
/// [`CalloraYieldLimits::set_default_limits`].
pub const DEFAULT_LIMITS: AccountLimits = AccountLimits {
    max_bets: 100,
    max_positions: 50,
    max_subscriptions: 20,
};

/// Maximum allowable value for any single cap dimension.
///
/// Counts are stored as `u32`, so this matches the practical ceiling. The
/// cap itself is never compared against `u32::MAX` so this is a conservative
/// sanity ceiling rather than the absolute type ceiling.
pub const MAX_CAP: u32 = 1_000_000;

// ---------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------

/// Instance / persistent storage keys for the yield per-account limits
/// contract.
///
/// Using a single [`contracttype`] enum keeps the key space tidy and protects
/// against accidental key collisions across this module and future modules.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageKey {
    /// Admin address (set by [`CalloraYieldLimits::init`]).
    Admin,
    /// Pending admin awaiting acceptance (two-step transfer).
    PendingAdmin,
    /// Global default caps applied when an account has no explicit override.
    DefaultLimits,
    /// Per-account cap override (instance storage; sparse).
    AccountLimits(Address),
    /// Per-account live counters (persistent storage).
    AccountState(Address),
    /// Hash of the most recently installed WASM (`BytesN<32>`, instance
    /// storage).
    ///
    /// Deliberately named `WasmVersion` (rather than `Version`) to avoid
    /// colliding with the `StorageKey::Version` marker written by
    /// [`StorageMigrationValidator`] in the same instance-storage namespace.
    WasmVersion,
}

// ---------------------------------------------------------------------
// Aux structs
// ---------------------------------------------------------------------

/// Per-account state caps configured by the admin.
///
/// Each field sets the maximum allowed concurrent state of that kind for a
/// single account. A value of `0` disables that kind entirely (no new bets,
/// positions, or subscriptions can be opened while the cap is `0`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountLimits {
    /// Maximum concurrent open bets.
    pub max_bets: u32,
    /// Maximum concurrent open positions.
    pub max_positions: u32,
    /// Maximum concurrent active subscriptions.
    pub max_subscriptions: u32,
}

impl AccountLimits {
    /// Construct a new [`AccountLimits`] with all three caps set to `count`.
    pub const fn uniform(count: u32) -> Self {
        Self {
            max_bets: count,
            max_positions: count,
            max_subscriptions: count,
        }
    }

    /// Return `true` if any individual cap exceeds [`MAX_CAP`] or any cap is
    /// non-sensical (no upper bound on `u32` makes a `-1`-style value
    /// impossible, but the function reserves headroom for future validation
    /// rules).
    pub fn is_valid(&self) -> bool {
        self.max_bets <= MAX_CAP
            && self.max_positions <= MAX_CAP
            && self.max_subscriptions <= MAX_CAP
    }
}

/// Per-account live state counters.
///
/// Only the contract mutates fields on this struct. Counter arithmetic is
/// performed via `checked_add` / `checked_sub` so a buggy caller can never
/// drive any field into `u32::MAX + 1`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AccountState {
    /// Current number of open bets.
    pub bets: u32,
    /// Current number of open positions.
    pub positions: u32,
    /// Current number of active subscriptions.
    pub subscriptions: u32,
}

impl AccountState {
    /// Return a zeroed [`AccountState`].
    pub const fn zero() -> Self {
        Self {
            bets: 0,
            positions: 0,
            subscriptions: 0,
        }
    }

    /// Increment the bet counter using `checked_add`.
    ///
    /// # Errors
    /// - [`YieldLimitError::Overflow`] — counter would saturate `u32::MAX`.
    pub fn add_bet(&mut self) -> Result<(), YieldLimitError> {
        self.bets = self.bets.checked_add(1).ok_or(YieldLimitError::Overflow)?;
        Ok(())
    }

    /// Decrement the bet counter using `checked_sub`.
    ///
    /// # Errors
    /// - [`YieldLimitError::CounterUnderflow`] — counter is already 0.
    pub fn sub_bet(&mut self) -> Result<(), YieldLimitError> {
        self.bets = self
            .bets
            .checked_sub(1)
            .ok_or(YieldLimitError::CounterUnderflow)?;
        Ok(())
    }

    /// Increment the position counter using `checked_add`.
    pub fn add_position(&mut self) -> Result<(), YieldLimitError> {
        self.positions = self
            .positions
            .checked_add(1)
            .ok_or(YieldLimitError::Overflow)?;
        Ok(())
    }

    /// Decrement the position counter using `checked_sub`.
    pub fn sub_position(&mut self) -> Result<(), YieldLimitError> {
        self.positions = self
            .positions
            .checked_sub(1)
            .ok_or(YieldLimitError::CounterUnderflow)?;
        Ok(())
    }

    /// Increment the subscription counter using `checked_add`.
    pub fn add_subscription(&mut self) -> Result<(), YieldLimitError> {
        self.subscriptions = self
            .subscriptions
            .checked_add(1)
            .ok_or(YieldLimitError::Overflow)?;
        Ok(())
    }

    /// Decrement the subscription counter using `checked_sub`.
    pub fn sub_subscription(&mut self) -> Result<(), YieldLimitError> {
        self.subscriptions = self
            .subscriptions
            .checked_sub(1)
            .ok_or(YieldLimitError::CounterUnderflow)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Free functions — storage helpers
// ---------------------------------------------------------------------

/// Read the admin address from instance storage.
///
/// # Errors
/// - [`YieldLimitError::NotInitialized`] — `Admin` key is absent.
pub fn read_admin(env: &Env) -> Result<Address, YieldLimitError> {
    env.storage()
        .instance()
        .get::<_, Address>(&StorageKey::Admin)
        .ok_or(YieldLimitError::NotInitialized)
}

/// Assert `caller` equals the stored admin.
///
/// Runs `caller.require_auth()` first so misconfigured callers are rejected
/// deterministically without consuming the underlying signature.
///
/// # Errors
/// - [`YieldLimitError::Unauthorized`] — caller is not the stored admin.
/// - [`YieldLimitError::NotInitialized`] — admin has never been set.
pub fn require_admin(env: &Env, caller: &Address) -> Result<(), YieldLimitError> {
    let admin = read_admin(env)?;
    caller.require_auth();
    if *caller != admin {
        return Err(YieldLimitError::Unauthorized);
    }
    Ok(())
}

/// Read the global default caps (or fall back to [`DEFAULT_LIMITS`]) and
/// extend instance TTL.
pub fn read_default_limits(env: &Env) -> AccountLimits {
    let caps: AccountLimits = env
        .storage()
        .instance()
        .get::<_, AccountLimits>(&StorageKey::DefaultLimits)
        .unwrap_or(DEFAULT_LIMITS);
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    caps
}

/// Persist the global default caps and extend instance TTL.
///
/// # Errors
/// - [`YieldLimitError::InvalidLimit`] — any cap exceeds [`MAX_CAP`].
pub fn write_default_limits(env: &Env, caps: &AccountLimits) -> Result<(), YieldLimitError> {
    if !caps.is_valid() {
        return Err(YieldLimitError::InvalidLimit);
    }
    env.storage()
        .instance()
        .set(&StorageKey::DefaultLimits, caps);
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    Ok(())
}

/// Read the per-account override or fall back to [`read_default_limits`].
///
/// Per-account overrides stored in instance storage survive across calls
/// along with other config; persistent storage is reserved for the live
/// state counters.
pub fn read_account_limits(env: &Env, account: &Address) -> AccountLimits {
    if let Some(caps) = env
        .storage()
        .instance()
        .get::<_, AccountLimits>(&StorageKey::AccountLimits(account.clone()))
    {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        return caps;
    }
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    read_default_limits(env)
}

/// Persist a per-account override and extend instance TTL.
///
/// # Errors
/// - [`YieldLimitError::InvalidLimit`] — any cap exceeds [`MAX_CAP`].
pub fn write_account_limits(
    env: &Env,
    account: &Address,
    caps: &AccountLimits,
) -> Result<(), YieldLimitError> {
    if !caps.is_valid() {
        return Err(YieldLimitError::InvalidLimit);
    }
    env.storage()
        .instance()
        .set(&StorageKey::AccountLimits(account.clone()), caps);
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    Ok(())
}

/// Remove any per-account override. After this call the account falls back
/// to the global default caps.
pub fn clear_account_limits(env: &Env, account: &Address) {
    env.storage()
        .instance()
        .remove(&StorageKey::AccountLimits(account.clone()));
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Read the live per-account counters, returning a zero struct if the
/// caller has no recorded state yet. Bumps persistent TTL on read.
pub fn read_account_state(env: &Env, account: &Address) -> AccountState {
    let key = StorageKey::AccountState(account.clone());
    match env.storage().persistent().get::<_, AccountState>(&key) {
        Some(state) => {
            env.storage()
                .persistent()
                .extend_ttl(&key, STATE_BUMP_THRESHOLD, STATE_BUMP_AMOUNT);
            state
        }
        // No recorded state yet: a zero struct is the correct default, and
        // there is no entry whose TTL could (or should) be extended.
        None => AccountState::zero(),
    }
}

/// Persist the live per-account counters and bump persistent TTL.
pub fn write_account_state(env: &Env, account: &Address, state: &AccountState) {
    let key = StorageKey::AccountState(account.clone());
    env.storage().persistent().set(&key, state);
    env.storage()
        .persistent()
        .extend_ttl(&key, STATE_BUMP_THRESHOLD, STATE_BUMP_AMOUNT);
}

// ---------------------------------------------------------------------
// Free functions — gate checks
// ---------------------------------------------------------------------

/// Return `true` if `account` can place another bet without exceeding caps.
pub fn can_place_bet(env: &Env, account: &Address) -> bool {
    let caps = read_account_limits(env, account);
    let state = read_account_state(env, account);
    state.bets < caps.max_bets
}

/// Return `true` if `account` can open another position without exceeding caps.
pub fn can_open_position(env: &Env, account: &Address) -> bool {
    let caps = read_account_limits(env, account);
    let state = read_account_state(env, account);
    state.positions < caps.max_positions
}

/// Return `true` if `account` can subscribe again without exceeding caps.
pub fn can_subscribe(env: &Env, account: &Address) -> bool {
    let caps = read_account_limits(env, account);
    let state = read_account_state(env, account);
    state.subscriptions < caps.max_subscriptions
}

// ---------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------

/// Callora Yield per-account limits enforcement contract.
///
/// `init` registers an `admin`; the admin (and only the admin) may configure
/// per-account caps and the global default caps. End-users may increment or
/// decrement their own counters via `place_bet`, `open_position`,
/// `subscribe`, `clear_bet`, `close_position`, `unsubscribe`.
#[contract]
pub struct CalloraYieldLimits;

#[contractimpl]
impl CalloraYieldLimits {
    // -----------------------------------------------------------------
    // init + lifecycle
    // -----------------------------------------------------------------

    /// Initialize the contract with an admin address.
    ///
    /// # Arguments
    /// * `admin` — Address authorised to mutate per-account caps, swap
    ///   admins, upgrade the contract, and pause/unpause the surface.
    ///
    /// # Errors
    /// - [`YieldLimitError::AlreadyInitialized`] — admin already set.
    pub fn init(env: Env, admin: Address) -> Result<(), YieldLimitError> {
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(YieldLimitError::AlreadyInitialized);
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        // Record the deployed storage-layout version + hash so the upgrade
        // path can enforce ordered, single-step migrations against a known
        // schema from the very first deployment.
        StorageMigrationValidator::record_deployed_schema(
            &env,
            STORAGE_MIGRATION_VERSION,
            &storage_layout_hash(&env),
        )
        .map_err(|_| YieldLimitError::UpgradeRejected)?;

        env.events().publish((events::event_init(&env), admin), ());
        Ok(())
    }

    /// Return the stored admin address.
    ///
    /// # Errors
    /// - [`YieldLimitError::NotInitialized`] — `init` has not been called.
    pub fn get_admin(env: Env) -> Result<Address, YieldLimitError> {
        let admin = read_admin(&env)?;
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(admin)
    }

    // -----------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------

    /// Initiate a two-step admin transfer (`caller` must be current admin).
    ///
    /// Re-nominating the current admin is permitted — once the pending
    /// transfer is accepted the admin role remains effectively unchanged,
    /// which is safe because every other admin-gated path still requires a
    /// fresh `require_auth` round-trip through the nominated address.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;
        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);
        env.events()
            .publish((events::event_admin_nominated(&env), caller), new_admin);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Complete a pending admin transfer (`caller` must be the pending admin).
    pub fn accept_admin(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        let pending = env
            .storage()
            .instance()
            .get::<_, Address>(&StorageKey::PendingAdmin)
            .ok_or(YieldLimitError::Unauthorized)?;
        caller.require_auth();
        if caller != pending {
            return Err(YieldLimitError::Unauthorized);
        }
        env.storage().instance().set(&StorageKey::Admin, &caller);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        env.events()
            .publish((events::event_admin_accepted(&env), caller.clone()), ());
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    /// Cancel a pending admin transfer (`caller` must be current admin).
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;
        let pending = env
            .storage()
            .instance()
            .get::<_, Address>(&StorageKey::PendingAdmin)
            .ok_or(YieldLimitError::Unauthorized)?;
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        env.events()
            .publish((events::event_admin_cancelled(&env), caller), pending);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        Ok(())
    }

    // -----------------------------------------------------------------
    // Limits configuration
    // -----------------------------------------------------------------

    /// Replace the global default caps (admin only).
    pub fn set_default_limits(
        env: Env,
        caller: Address,
        max_bets: u32,
        max_positions: u32,
        max_subscriptions: u32,
    ) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;
        let caps = AccountLimits {
            max_bets,
            max_positions,
            max_subscriptions,
        };
        write_default_limits(&env, &caps)?;
        env.events().publish(
            (events::event_default_limits_set(&env), caller, caps.clone()),
            caps,
        );
        Ok(())
    }

    /// Set explicit per-account caps overriding the global default
    /// (admin only).
    pub fn set_account_limits(
        env: Env,
        caller: Address,
        account: Address,
        max_bets: u32,
        max_positions: u32,
        max_subscriptions: u32,
    ) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;
        let caps = AccountLimits {
            max_bets,
            max_positions,
            max_subscriptions,
        };
        write_account_limits(&env, &account, &caps)?;
        env.events().publish(
            (
                events::event_account_limits_set(&env),
                caller,
                account.clone(),
            ),
            caps,
        );
        Ok(())
    }

    /// Remove any per-account override so the account falls back to global
    /// defaults (admin only).
    pub fn clear_account_limits(
        env: Env,
        caller: Address,
        account: Address,
    ) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;
        clear_account_limits(&env, &account);
        env.events().publish(
            (events::event_account_limits_cleared(&env), caller, account),
            (),
        );
        Ok(())
    }

    // -----------------------------------------------------------------
    // User-gated counter mutators
    // -----------------------------------------------------------------

    /// Increment the caller's open-bet counter after enforcing auth and
    /// the per-account cap.
    pub fn place_bet(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let caps = read_account_limits(&env, &caller);
        let mut state = read_account_state(&env, &caller);
        // Pre-check: surfacing BetsAtCap as a typed error gives callers a
        // stable code to branch on rather than a generic panic.
        if state.bets >= caps.max_bets {
            return Err(YieldLimitError::BetsAtCap);
        }
        state.add_bet()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_bet_placed(&env), caller.clone()),
            (state.bets, caps.max_bets),
        );
        Ok(())
    }

    /// Decrement the caller's open-bet counter after enforcing auth.
    pub fn clear_bet(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let mut state = read_account_state(&env, &caller);
        state.sub_bet()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_bet_cleared(&env), caller.clone()),
            state.bets,
        );
        Ok(())
    }

    /// Increment the caller's open-position counter after enforcing auth
    /// and the per-account cap.
    pub fn open_position(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let caps = read_account_limits(&env, &caller);
        let mut state = read_account_state(&env, &caller);
        if state.positions >= caps.max_positions {
            return Err(YieldLimitError::PositionsAtCap);
        }
        state.add_position()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_position_opened(&env), caller.clone()),
            (state.positions, caps.max_positions),
        );
        Ok(())
    }

    /// Decrement the caller's open-position counter after enforcing auth.
    pub fn close_position(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let mut state = read_account_state(&env, &caller);
        state.sub_position()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_position_closed(&env), caller.clone()),
            state.positions,
        );
        Ok(())
    }

    /// Increment the caller's subscription counter after enforcing auth
    /// and the per-account cap.
    pub fn subscribe(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let caps = read_account_limits(&env, &caller);
        let mut state = read_account_state(&env, &caller);
        if state.subscriptions >= caps.max_subscriptions {
            return Err(YieldLimitError::SubscriptionsAtCap);
        }
        state.add_subscription()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_subscription_added(&env), caller.clone()),
            (state.subscriptions, caps.max_subscriptions),
        );
        Ok(())
    }

    /// Decrement the caller's subscription counter after enforcing auth.
    pub fn unsubscribe(env: Env, caller: Address) -> Result<(), YieldLimitError> {
        caller.require_auth();
        let mut state = read_account_state(&env, &caller);
        state.sub_subscription()?;
        write_account_state(&env, &caller, &state);
        env.events().publish(
            (events::event_subscription_removed(&env), caller.clone()),
            state.subscriptions,
        );
        Ok(())
    }

    // -----------------------------------------------------------------
    // Read-only views
    // -----------------------------------------------------------------

    /// Read the global default caps.
    pub fn get_default_limits(env: Env) -> AccountLimits {
        read_default_limits(&env)
    }

    /// Read the effective per-account caps.
    pub fn get_account_limits(env: Env, account: Address) -> AccountLimits {
        read_account_limits(&env, &account)
    }

    /// Read the live per-account counters.
    pub fn get_account_state(env: Env, account: Address) -> AccountState {
        read_account_state(&env, &account)
    }

    /// Dry-run check: would `place_bet` succeed for `account`?
    pub fn can_place_bet(env: Env, account: Address) -> bool {
        can_place_bet(&env, &account)
    }

    /// Dry-run check: would `open_position` succeed for `account`?
    pub fn can_open_position(env: Env, account: Address) -> bool {
        can_open_position(&env, &account)
    }

    /// Dry-run check: would `subscribe` succeed for `account`?
    pub fn can_subscribe(env: Env, account: Address) -> bool {
        can_subscribe(&env, &account)
    }

    // -----------------------------------------------------------------
    // Upgrade
    // -----------------------------------------------------------------

    /// Replace the WASM, run the storage-migration guard, and persist the new
    /// hash under [`StorageKey::WasmVersion`] (admin only).
    ///
    /// The guard runs in the *current* (old) code, before
    /// `update_current_contract_wasm`, and never mutates business state. It
    /// rejects an all-zero WASM hash, enforces ordered single-step migrations,
    /// and detects a silent layout change to [`AccountLimits`]/[`AccountState`].
    ///
    /// # Errors
    /// - [`YieldLimitError::Unauthorized`] — caller is not the current admin.
    /// - [`YieldLimitError::NotInitialized`] — `init` has not been called.
    /// - [`YieldLimitError::UpgradeRejected`] — the migration validator
    ///   rejected the upgrade (all-zero hash, version skip, rollback without a
    ///   backup, or a silent storage-layout change).
    pub fn upgrade(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), YieldLimitError> {
        require_admin(&env, &caller)?;

        // ── Pre-upgrade storage-migration validation ──────────────────────
        let layout_hash = storage_layout_hash(&env);
        StorageMigrationValidator::validate_before_upgrade(
            &env,
            STORAGE_MIGRATION_VERSION,
            &layout_hash,
            &callora_storage_migration::zero_layout_hash(&env),
            &new_wasm_hash,
            false,
        )
        .map_err(|_| YieldLimitError::UpgradeRejected)?;
        StorageMigrationValidator::finalize_migration(
            &env,
            STORAGE_MIGRATION_VERSION,
            &layout_hash,
            &new_wasm_hash,
        )
        .map_err(|_| YieldLimitError::UpgradeRejected)?;

        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.storage()
            .instance()
            .set(&StorageKey::WasmVersion, &new_wasm_hash);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.events()
            .publish((events::event_upgraded(&env), caller), new_wasm_hash);
        Ok(())
    }

    /// Return the last upgraded WASM hash, or `None` if never upgraded.
    ///
    /// This is the on-chain source of truth for "which code is live" after an
    /// upgrade, mirroring `get_version` on the revenue pool and checkpoint
    /// contracts.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage().instance().get(&StorageKey::WasmVersion)
    }
}
