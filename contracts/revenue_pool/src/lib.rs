#![no_std]

pub mod emergency;
pub mod errors;
pub mod events;

use callora_storage_migration::StorageMigrationValidator;
use emergency::{PendingEmergencyDrain, EMERGENCY_DRAIN_KEY, EMERGENCY_DRAIN_TIMELOCK_SECONDS};
pub use errors::RevenuePoolError;
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, BytesN, Env, Map, String, Symbol, Vec,
};

/// Storage-layout version recorded by [`StorageMigrationValidator`] for this
/// contract. Bumped whenever the on-ledger schema changes so that the
/// pre-upgrade validation gate can enforce ordered, single-step migrations.
const STORAGE_MIGRATION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// String-keyed storage identifiers.  Using a single `DataKey` enum keeps the
/// key space tidy and avoids accidental key collisions with raw strings.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    UsdcToken,
    Paused,
}

// String keys for fields that must remain stable across upgrades.
const ADMIN_KEY: &str = "admin";
const USDC_KEY: &str = "usdc";
const PAUSED_KEY: &str = "paused";
const EMERGENCY_PAUSED_KEY: &str = "emergency_paused";
const PENDING_ADMIN_KEY: &str = "pending_admin";
const PAUSE_GUARDIAN_KEY: &str = "pause_guardian";
const MAX_DISTRIBUTE_KEY: &str = "max_distribute";
const CUMULATIVE_YIELD_DEPOSITED_KEY: &str = "cumulative_yield";
const VERSION_KEY: &str = "version";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default per-leg distribution cap — effectively unlimited until explicitly set.
pub const DEFAULT_MAX_DISTRIBUTE: i128 = i128::MAX;

/// Maximum number of payments in a single `batch_distribute` call.
pub const MAX_BATCH_SIZE: u32 = 50;

/// Maximum admin broadcast message length in characters.
pub const MAX_MESSAGE_LEN: u32 = 256;

/// Number of ledgers in a single day, assuming ~5 s close time.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// TTL bump constants for instance storage archival risk mitigation.
///
/// The revenue pool holds critical operational state (admin, USDC address,
/// pause flags, pending drain). If the instance entry archives, every call
/// fails until an explicit restore, which can stall payouts during quiet
/// periods or incidents. We therefore keep the instance alive for at least
/// 30 days after any call, matching the vault and settlement contracts.
///
/// - `BUMP_AMOUNT`: extend TTL by 30 days of ledgers
/// - `LIFETIME_THRESHOLD`: minimum TTL before triggering a bump (1 day)
pub const BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 30;
pub const LIFETIME_THRESHOLD: u32 = LEDGERS_PER_DAY;

// ---------------------------------------------------------------------------
// Auxiliary contract-types
// ---------------------------------------------------------------------------

/// Severity levels for admin broadcast messages.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

/// Event payload for admin broadcast messages.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AdminBroadcast {
    pub severity: Severity,
    pub message: String,
}

/// TTL policy (threshold / bump constants) this contract applies to a storage
/// category.
///
/// This deliberately carries **no live-TTL field**. Contract code cannot observe
/// the remaining TTL of a ledger entry, so any such value would be the
/// [`BUMP_AMOUNT`] constant mislabelled as a measurement. For live TTLs, read
/// the ledger entries over Soroban RPC `getLedgerEntries` and compare
/// `liveUntilLedgerSeq` against the current ledger sequence; see
/// `docs/STORAGE_TTL_DOCTOR.md` and `scripts/storage-ttl-doctor.ts`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TtlPolicy {
    pub category: String,
    pub key_desc: String,
    pub storage_type: String,
    pub threshold: u32,
    pub bump_amount: u32,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct RevenuePool;

#[contractimpl]
impl RevenuePool {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the revenue pool with an admin and the USDC token address.
    ///
    /// Can only be called once. The configured admin must authorize the call,
    /// and the USDC token address must differ from both the pool and admin.
    ///
    /// # Errors
    /// * [`RevenuePoolError::AlreadyInitialized`] - called more than once.
    /// * [`RevenuePoolError::InvalidUsdcToken`] - token aliases the pool or admin.
    ///
    /// # Events
    /// Emits `init` with `admin` as topic and `usdc_token` as data.
    pub fn init(env: Env, admin: Address, usdc_token: Address) {
        admin.require_auth();
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
            env.panic_with_error(RevenuePoolError::AlreadyInitialized);
        }
        let contract_addr = env.current_contract_address();
        if usdc_token == contract_addr || usdc_token == admin {
            env.panic_with_error(RevenuePoolError::InvalidUsdcToken);
        }
        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, ADMIN_KEY), &admin);
        inst.set(&Symbol::new(&env, USDC_KEY), &usdc_token);
        inst.set(&Symbol::new(&env, PAUSED_KEY), &false);
        inst.set(&Symbol::new(&env, EMERGENCY_PAUSED_KEY), &false);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_init(&env), admin), usdc_token);
    }

    // -----------------------------------------------------------------------
    // Admin helpers (internal)
    // -----------------------------------------------------------------------

    fn admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(env, ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized))
    }

    fn require_admin(env: &Env, caller: &Address) {
        if *caller != Self::admin(env) {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
    }

    /// Extend instance storage TTL on hot read paths so frequently-queried
    /// getters do not silently archive while only being read.
    #[inline]
    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }

    fn require_not_paused(env: &Env) {
        Self::require_not_emergency_paused(env);
        if env
            .storage()
            .instance()
            .get::<_, bool>(&Symbol::new(env, PAUSED_KEY))
            .unwrap_or(false)
        {
            env.panic_with_error(RevenuePoolError::Paused);
        }
    }

    fn require_not_emergency_paused(env: &Env) {
        if env
            .storage()
            .instance()
            .get::<_, bool>(&Symbol::new(env, EMERGENCY_PAUSED_KEY))
            .unwrap_or(false)
        {
            env.panic_with_error(RevenuePoolError::EmergencyPaused);
        }
    }

    fn validate_recipient(env: &Env, recipient: &Address, contract_self: &Address) {
        // Rule 1 — no self-distributions.
        if recipient == contract_self {
            env.panic_with_error(RevenuePoolError::InvalidRecipient);
        }
    }

    // -----------------------------------------------------------------------
    // Admin view
    // -----------------------------------------------------------------------

    /// Return the current admin address.
    ///
    /// # Errors
    /// * [`RevenuePoolError::NotInitialized`] - called before `init`.
    pub fn get_admin(env: Env) -> Address {
        Self::bump_instance_ttl(&env);
        Self::admin(&env)
    }

    /// Return the USDC token address configured for this pool.
    ///
    /// # Errors
    /// * [`RevenuePoolError::NotInitialized`] - called before `init`.
    pub fn get_usdc_token(env: Env) -> Address {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized))
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    /// The nominee must call `claim_admin` (alias `accept_admin`) to complete.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    ///
    /// # Events
    /// Emits `admin_transfer_started` with `current` as topic and `new_admin`
    /// as data. No `admin_changed` event is published while the transfer is
    /// only pending — the admin does not change until `accept_admin`
    /// (Issue #1163), so a nomination that is later cancelled never announces
    /// a change that did not happen.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, PENDING_ADMIN_KEY), &new_admin);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_admin_transfer_started(&env), current),
            new_admin,
        );
    }

    /// Complete the admin transfer. Only the pending admin may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::NoAdminTransferPending`] - no transfer is in progress.
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the pending admin.
    ///
    /// # Events
    /// Emits `admin_changed` with the previous admin as topic and
    /// `(previous_admin, new_admin)` as data, followed by
    /// `admin_transfer_completed` with the new admin as topic.
    /// Both are published only after the admin slot is updated, so indexers
    /// observe the change exactly when it happens (Issue #1163).
    pub fn accept_admin(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NoAdminTransferPending));
        if caller != pending {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        let previous = Self::admin(&env);
        inst.set(&Symbol::new(&env, ADMIN_KEY), &pending);
        inst.remove(&Symbol::new(&env, PENDING_ADMIN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_admin_changed(&env), previous.clone()),
            (previous, pending.clone()),
        );
        env.events()
            .publish((events::event_admin_transfer_completed(&env), pending), ());
    }

    /// Alias for `accept_admin` — legacy name kept for backward compatibility.
    ///
    /// # Errors
    /// * [`RevenuePoolError::NoAdminTransferPending`] - no transfer is in progress.
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the pending admin.
    ///
    /// # Events
    /// Emits `admin_changed` followed by `admin_transfer_completed` with the
    /// new admin as topic.
    pub fn claim_admin(env: Env, caller: Address) {
        Self::accept_admin(env, caller);
    }

    /// Cancel a pending admin transfer. Only the current admin may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NoAdminTransferPending`] - no transfer is in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled` with `(current_admin, pending_admin)`.
    pub fn cancel_admin_transfer(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NoAdminTransferPending));
        inst.remove(&Symbol::new(&env, PENDING_ADMIN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_admin_cancelled(&env), current, pending), ());
    }

    /// Return the pending admin address, or `None` if no transfer is in progress.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY))
    }

    // -----------------------------------------------------------------------
    // Pause guardian
    // -----------------------------------------------------------------------

    /// Set or replace the emergency pause guardian.
    ///
    /// The guardian may call `pause` but has no authority to unpause, distribute,
    /// rotate admin, change caps, or upgrade the contract.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    ///
    /// # Events
    /// Emits `pause_guardian_set` with `caller` as topic and `guardian` as data.
    pub fn set_pause_guardian(env: Env, caller: Address, guardian: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PAUSE_GUARDIAN_KEY), &guardian);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_pause_guardian_set(&env), caller), guardian);
    }

    /// Clear the emergency pause guardian role. Only the admin may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NoPauseGuardian`] - no guardian is configured.
    ///
    /// # Events
    /// Emits `pause_guardian_cleared` with `caller` as topic and the previous guardian as data.
    pub fn clear_pause_guardian(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        let inst = env.storage().instance();
        let guardian: Address = inst
            .get(&Symbol::new(&env, PAUSE_GUARDIAN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NoPauseGuardian));
        inst.remove(&Symbol::new(&env, PAUSE_GUARDIAN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_pause_guardian_cleared(&env), caller),
            guardian,
        );
    }

    /// Return the configured pause guardian, or `None` if unset.
    pub fn get_pause_guardian(env: Env) -> Option<Address> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, PAUSE_GUARDIAN_KEY))
    }

    // -----------------------------------------------------------------------
    // Pause / unpause
    // -----------------------------------------------------------------------

    /// Activate the circuit-breaker. Blocks `distribute` and `batch_distribute`.
    ///
    /// The admin or configured pause guardian may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is neither admin nor guardian.
    /// * [`RevenuePoolError::AlreadyPaused`] - pool is already paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `true` as data.
    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        let admin = Self::admin(&env);
        let guardian = Self::get_pause_guardian(env.clone());
        if caller != admin && guardian.as_ref() != Some(&caller) {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        if Self::is_paused(env.clone()) {
            env.panic_with_error(RevenuePoolError::AlreadyPaused);
        }
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PAUSED_KEY), &true);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_pause_set(&env), caller), true);
    }

    /// Deactivate the circuit-breaker. Only the admin may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NotPaused`] - pool is not currently paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `false` as data.
    pub fn unpause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        if !Self::is_paused(env.clone()) {
            env.panic_with_error(RevenuePoolError::NotPaused);
        }
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PAUSED_KEY), &false);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_pause_set(&env), caller), false);
    }

    /// Return `true` if the revenue pool is currently paused.
    pub fn is_paused(env: Env) -> bool {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, bool>(&Symbol::new(&env, PAUSED_KEY))
            .unwrap_or(false)
    }

    /// Activate recovery-only emergency mode.
    ///
    /// The admin or configured pause guardian may call. Once active, normal
    /// sensitive mutations fail closed; only admin recovery and drain
    /// cancellation remain available.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is neither admin nor guardian.
    /// * [`RevenuePoolError::AlreadyEmergencyPaused`] - emergency mode is already active.
    ///
    /// # Events
    /// Emits `emergency_pause_set` with `caller` as topic and `true` as data.
    pub fn emergency_pause(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::admin(&env);
        let guardian = Self::get_pause_guardian(env.clone());
        if caller != admin && guardian.as_ref() != Some(&caller) {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        if Self::is_emergency_paused(env.clone()) {
            env.panic_with_error(RevenuePoolError::AlreadyEmergencyPaused);
        }
        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, EMERGENCY_PAUSED_KEY), &true);
        inst.set(&Symbol::new(&env, PAUSED_KEY), &true);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_emergency_pause_set(&env), caller), true);
    }

    /// Clear recovery-only emergency mode after operator remediation.
    ///
    /// Only the current admin may recover the pool. Recovery also clears the
    /// regular pause flag so normal operations resume in a single authorised
    /// action.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NotEmergencyPaused`] - emergency mode is not active.
    ///
    /// # Events
    /// Emits `emergency_pause_set` with `caller` as topic and `false` as data.
    pub fn recover_from_emergency(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if !Self::is_emergency_paused(env.clone()) {
            env.panic_with_error(RevenuePoolError::NotEmergencyPaused);
        }
        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, EMERGENCY_PAUSED_KEY), &false);
        inst.set(&Symbol::new(&env, PAUSED_KEY), &false);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_emergency_pause_set(&env), caller), false);
    }

    /// Return `true` when recovery-only emergency mode is active.
    pub fn is_emergency_paused(env: Env) -> bool {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, bool>(&Symbol::new(&env, EMERGENCY_PAUSED_KEY))
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Yield deposit
    // -----------------------------------------------------------------------

    /// Event-only helper to log an incoming vault payment for indexers.
    ///
    /// Does **not** move tokens. Only the admin may call.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    ///
    /// # Events
    /// Emits `receive_payment` with `caller` as topic and `(amount, from_vault)` as data.
    pub fn receive_payment(env: Env, caller: Address, amount: i128, from_vault: bool) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        env.events().publish(
            (events::event_receive_payment(&env), caller),
            (amount, from_vault),
        );
    }

    /// Deposit accumulated protocol yield into the revenue pool.
    ///
    /// Transfers USDC from `treasury` to this contract, then updates the
    /// cumulative yield metric. The external token call happens before storage
    /// writes so a callee panic cannot leave the metric ahead of a failed
    /// transfer (Soroban still rolls the whole invocation back on panic).
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - treasury is not the current admin.
    /// * [`RevenuePoolError::AmountNotPositive`] - amount is not positive.
    /// * [`RevenuePoolError::Overflow`] - cumulative yield would overflow `i128`.
    /// * [`RevenuePoolError::NotInitialized`] - the USDC token is not configured.
    ///
    /// # Events
    /// Emits `yield_deposited` with `treasury` as topic and
    /// `(amount, source, cumulative_yield_deposited)` as data.
    pub fn deposit_yield(env: Env, treasury: Address, amount: i128, source: Symbol) {
        treasury.require_auth();
        Self::require_not_emergency_paused(&env);
        if treasury != Self::admin(&env) {
            env.panic_with_error(RevenuePoolError::Unauthorized);
        }
        if amount <= 0 {
            env.panic_with_error(RevenuePoolError::AmountNotPositive);
        }
        let previous_total = Self::get_cumulative_yield_deposited(env.clone());
        let new_total = previous_total
            .checked_add(amount)
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::Overflow));
        let usdc_address: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        // Transfer before persisting the cumulative metric so a callee panic
        // (token revert) cannot leave storage ahead of a failed transfer.
        // Soroban still rolls the whole invocation back on panic; this order
        // keeps the success path effects-after-external-call.
        usdc.transfer(&treasury, &contract_address, &amount);
        let inst = env.storage().instance();
        inst.set(
            &Symbol::new(&env, CUMULATIVE_YIELD_DEPOSITED_KEY),
            &new_total,
        );
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_yield_deposited(&env), treasury),
            (amount, source, new_total),
        );
    }

    /// Return the cumulative USDC yield deposited via `deposit_yield`. Defaults to 0.
    pub fn get_cumulative_yield_deposited(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, CUMULATIVE_YIELD_DEPOSITED_KEY))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Distribution cap
    // -----------------------------------------------------------------------

    /// Return the per-leg distribution cap. Defaults to `i128::MAX` when unset.
    pub fn get_max_distribute(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, MAX_DISTRIBUTE_KEY))
            .unwrap_or(DEFAULT_MAX_DISTRIBUTE)
    }

    /// Set the maximum amount distributable per leg. Must be positive. Admin only.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::MaxDistributeNotPositive`] - value is not positive.
    ///
    /// # Events
    /// Emits `set_max_distribute` with `(old_max, new_max)`.
    pub fn set_max_distribute(env: Env, caller: Address, max_distribute: i128) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        if max_distribute <= 0 {
            env.panic_with_error(RevenuePoolError::MaxDistributeNotPositive);
        }
        let old_max = Self::get_max_distribute(env.clone());
        env.storage()
            .instance()
            .set(&Symbol::new(&env, MAX_DISTRIBUTE_KEY), &max_distribute);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_set_max_distribute(&env), Self::admin(&env)),
            (old_max, max_distribute),
        );
    }

    // -----------------------------------------------------------------------
    // Distribution
    // -----------------------------------------------------------------------

    /// Distribute USDC from this contract to a single developer wallet.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::Paused`] - pool is paused.
    /// * [`RevenuePoolError::AmountNotPositive`] - amount is not positive.
    /// * [`RevenuePoolError::AmountExceedsMaxDistribute`] - amount exceeds the cap.
    /// * [`RevenuePoolError::InvalidRecipient`] - recipient is the pool contract.
    /// * [`RevenuePoolError::InsufficientBalance`] - pool holds less than `amount`.
    /// * [`RevenuePoolError::NotInitialized`] - the USDC token is not configured.
    ///
    /// # Events
    /// Emits `distribute_started` and `distribute_completed` with a versioned
    /// payload around the transfer. The legacy `distribute` event is preserved.
    pub fn distribute(env: Env, caller: Address, to: Address, amount: i128) {
        caller.require_auth();
        Self::require_not_paused(&env);
        Self::require_admin(&env, &caller);
        if amount <= 0 {
            env.panic_with_error(RevenuePoolError::AmountNotPositive);
        }
        let max_distribute = Self::get_max_distribute(env.clone());
        if amount > max_distribute {
            env.panic_with_error(RevenuePoolError::AmountExceedsMaxDistribute);
        }
        let usdc_address: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        Self::validate_recipient(&env, &to, &contract_address);
        if usdc.balance(&contract_address) < amount {
            env.panic_with_error(RevenuePoolError::InsufficientBalance);
        }
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        let lifecycle = events::DistributionLifecycleEvent::new(
            &env,
            amount,
            events::DistributionMode::Single,
            0,
            1,
        );
        events::emit_distribute_started(&env, &caller, &to, &lifecycle);
        usdc.transfer(&contract_address, &to, &amount);
        env.events()
            .publish((events::event_distribute(&env), to.clone()), amount);
        events::emit_distribute_completed(&env, &caller, &to, &lifecycle);
    }

    /// Distribute USDC to multiple developer wallets in a single atomic transaction.
    ///
    /// All payments are validated upfront before any USDC transfer occurs.
    /// If **any** payment fails validation **no** transfers are executed and the
    /// entire call reverts. If all payments pass validation every transfer is
    /// executed and a `batch_distribute` event is emitted per payment leg.
    ///
    /// # Arguments
    /// * `caller` — must be the current admin and provide `require_auth`.
    /// * `payments` — a vector of `(recipient: Address, amount: i128)` pairs.
    ///   Maximum length is [`MAX_BATCH_SIZE`] (currently 50).
    ///
    /// # Errors
    /// * [`RevenuePoolError::BatchEmpty`] — `payments` is empty.
    /// * [`RevenuePoolError::BatchTooLarge`] — `payments` exceeds `MAX_BATCH_SIZE`.
    /// * [`RevenuePoolError::Unauthorized`] — caller is not the current admin.
    /// * [`RevenuePoolError::Paused`] — pool is paused.
    /// * [`RevenuePoolError::AmountNotPositive`] — any amount is not positive.
    /// * [`RevenuePoolError::AmountExceedsMaxDistribute`] — a leg exceeds the cap.
    /// * [`RevenuePoolError::DuplicateRecipient`] — recipients are duplicated.
    /// * [`RevenuePoolError::Overflow`] — total amount overflows `i128`.
    /// * [`RevenuePoolError::InsufficientBalance`] — balance is below the total.
    /// * [`RevenuePoolError::InvalidRecipient`] — a recipient is the pool contract.
    /// * [`RevenuePoolError::NotInitialized`] — the USDC token is not configured.
    ///
    /// # Events
    /// Emits structured `distribute_started` and `distribute_completed` events
    /// around each transfer. The legacy [`events::event_batch_distribute`] event
    /// is preserved for every payment leg.
    ///
    /// # Atomicity
    /// The function is **all-or-nothing**: either every payment succeeds and every
    /// event is emitted, or the entire transaction reverts. No partial state is
    /// observable. Indexers can verify atomicity by checking that all
    /// `batch_distribute` events share the same `(ledger, tx)` pair.
    pub fn batch_distribute(
        env: Env,
        caller: Address,
        payments: Vec<(Address, i128)>,
    ) -> Result<(), RevenuePoolError> {
        caller.require_auth();
        Self::require_not_paused(&env);
        Self::require_admin(&env, &caller);

        let n = payments.len();
        if n == 0 {
            return Err(RevenuePoolError::BatchEmpty);
        }
        if n > MAX_BATCH_SIZE {
            return Err(RevenuePoolError::BatchTooLarge);
        }

        let max_distribute = Self::get_max_distribute(env.clone());
        let mut seen: Map<Address, bool> = Map::new(&env);
        let mut total_amount: i128 = 0;
        let contract_address = env.current_contract_address();

        for payment in payments.iter() {
            let (to, amount) = payment;
            Self::validate_recipient(&env, &to, &contract_address);
            if seen.contains_key(to.clone()) {
                env.panic_with_error(RevenuePoolError::DuplicateRecipient);
            }
            seen.set(to.clone(), true);
            if amount <= 0 {
                env.panic_with_error(RevenuePoolError::AmountNotPositive);
            }
            if amount > max_distribute {
                env.panic_with_error(RevenuePoolError::AmountExceedsMaxDistribute);
            }
            total_amount = total_amount
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::Overflow));
        }

        let usdc_address: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);

        if usdc.balance(&contract_address) < total_amount {
            env.panic_with_error(RevenuePoolError::InsufficientBalance);
        }

        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        let mut batch_index = 0_u32;
        for payment in payments.iter() {
            let (to, amount) = payment;

            let lifecycle = events::DistributionLifecycleEvent::new(
                &env,
                amount,
                events::DistributionMode::Batch,
                batch_index,
                n,
            );
            events::emit_distribute_started(&env, &caller, &to, &lifecycle);
            usdc.transfer(&contract_address, &to, &amount);
            env.events()
                .publish((events::event_batch_distribute(&env), to.clone()), amount);
            events::emit_distribute_completed(&env, &caller, &to, &lifecycle);
            batch_index = batch_index.saturating_add(1);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Balance view
    // -----------------------------------------------------------------------

    /// Return this contract's on-ledger USDC balance.
    ///
    /// # Errors
    /// * [`RevenuePoolError::NotInitialized`] - called before `init`.
    pub fn balance(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_addr);
        usdc.balance(&env.current_contract_address())
    }

    // -----------------------------------------------------------------------
    // Upgrade
    // -----------------------------------------------------------------------

    /// Admin-gated contract upgrade. Replaces the WASM and persists the version.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    ///
    /// # Events
    /// Emits `upgraded` with `admin` as topic and `new_wasm_hash` as data.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);

        // ── Pre-upgrade storage-migration validation ──────────────────────
        // Runs in the *current* (old) code, before the WASM is swapped, and
        // never mutates business state. It enforces ordered, single-step
        // upgrades, rejects all-zero WASM hashes, and prevents unsanctioned
        // rollbacks — ensuring existing deployed data stays readable and no
        // implicit destructive transformation occurs.
        let placeholder_layout = callora_storage_migration::zero_layout_hash(&env);
        if let Err(e) = StorageMigrationValidator::validate_before_upgrade(
            &env,
            STORAGE_MIGRATION_VERSION,
            &placeholder_layout,
            &placeholder_layout,
            &new_wasm_hash,
            false,
        ) {
            env.panic_with_error(e);
        }
        if let Err(e) = StorageMigrationValidator::finalize_migration(
            &env,
            STORAGE_MIGRATION_VERSION,
            &placeholder_layout,
            &new_wasm_hash,
        ) {
            env.panic_with_error(e);
        }

        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.storage()
            .instance()
            .set(&Symbol::new(&env, VERSION_KEY), &new_wasm_hash.clone());
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_upgraded(&env), Self::admin(&env)),
            new_wasm_hash,
        );
    }

    /// Return the stored WASM version hash, or `None` if never upgraded.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, VERSION_KEY))
    }

    /// Return the crate version string baked in at compile time.
    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION"))
    }

    // -----------------------------------------------------------------------
    // Admin broadcast
    // -----------------------------------------------------------------------

    /// Broadcast an emergency message from the admin.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::MessageEmpty`] - message is empty.
    /// * [`RevenuePoolError::MessageTooLong`] - message exceeds [`MAX_MESSAGE_LEN`].
    ///
    /// # Events
    /// Emits `admin_broadcast` with `caller` as topic and `AdminBroadcast` as data.
    pub fn broadcast(env: Env, caller: Address, severity: Severity, message: String) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        let len = message.len();
        if len == 0 {
            env.panic_with_error(RevenuePoolError::MessageEmpty);
        }
        if len > MAX_MESSAGE_LEN {
            env.panic_with_error(RevenuePoolError::MessageTooLong);
        }
        env.events().publish(
            (events::event_admin_broadcast(&env), caller),
            AdminBroadcast { severity, message },
        );
    }

    // -----------------------------------------------------------------------
    // Storage TTL policy introspection
    // -----------------------------------------------------------------------

    /// Return the TTL policy this contract applies to each storage category.
    ///
    /// This view reports **policy constants only** (`threshold` and
    /// `bump_amount`). Earlier revisions also returned a `ttl` field that was
    /// the live instance TTL under `cfg(test)` but the constant [`BUMP_AMOUNT`]
    /// in production builds — a fabricated measurement. Contract code cannot
    /// observe the remaining TTL of a ledger entry at runtime, so the field was
    /// removed rather than guessed, and the view was renamed from
    /// `get_storage_ttl` to `get_ttl_policy` to match what it returns.
    ///
    /// For live TTLs, read the ledger entries over Soroban RPC
    /// `getLedgerEntries` and compare `liveUntilLedgerSeq` against the current
    /// ledger sequence. See `docs/STORAGE_TTL_DOCTOR.md` and
    /// `scripts/storage-ttl-doctor.ts`.
    pub fn get_ttl_policy(env: Env) -> Vec<TtlPolicy> {
        let mut result = Vec::new(&env);
        result.push_back(TtlPolicy {
            category: String::from_str(&env, "Instance"),
            key_desc: String::from_str(&env, "Instance"),
            storage_type: String::from_str(&env, "Instance"),
            threshold: LIFETIME_THRESHOLD,
            bump_amount: BUMP_AMOUNT,
        });
        result
    }

    // -----------------------------------------------------------------------
    // Emergency drain — Multisig + timelock
    // -----------------------------------------------------------------------

    /// Propose a timelocked USDC emergency drain to `treasury`.
    ///
    /// Stores a [`PendingEmergencyDrain`] snapshot. The drain may only be
    /// executed after `EMERGENCY_DRAIN_TIMELOCK_SECONDS` (24 h) have elapsed,
    /// giving operators a window to cancel a fraudulent proposal.
    ///
    /// A new call **replaces** any existing pending proposal, resetting the
    /// timelock clock — this is intentional to allow the admin to correct a
    /// mistaken amount or destination without first cancelling.
    ///
    /// When the admin is a Stellar multisig account, `require_auth` enforces
    /// the native multi-signature threshold automatically.
    ///
    /// # Arguments
    /// * `caller` — Must be the current admin; must authorize.
    /// * `treasury` — Destination address for the drained USDC. Cannot be the
    ///   contract itself.
    /// * `amount` — USDC amount in base units. Must be positive.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::AmountNotPositive`] - amount is not positive.
    /// * [`RevenuePoolError::InvalidRecipient`] - treasury is the pool contract.
    /// * [`RevenuePoolError::Overflow`] - timelock addition would overflow `u64`.
    ///
    /// # Events
    /// Emits `emergency_drain_proposed` with `caller` as topic and the full
    /// [`PendingEmergencyDrain`] snapshot as data.
    pub fn propose_emergency_drain(env: Env, caller: Address, treasury: Address, amount: i128) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);
        if amount <= 0 {
            env.panic_with_error(RevenuePoolError::AmountNotPositive);
        }
        let contract_address = env.current_contract_address();
        if treasury == contract_address {
            env.panic_with_error(RevenuePoolError::InvalidRecipient);
        }
        let proposed_at: u64 = env.ledger().timestamp();
        let execute_after: u64 = proposed_at
            .checked_add(EMERGENCY_DRAIN_TIMELOCK_SECONDS)
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::Overflow));

        let drain = PendingEmergencyDrain {
            to: treasury,
            amount,
            proposed_at,
            execute_after,
        };

        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, EMERGENCY_DRAIN_KEY), &drain);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        env.events().publish(
            (events::event_emergency_drain_proposed(&env), caller),
            drain,
        );
    }

    /// Execute a pending emergency drain after the timelock has expired.
    ///
    /// Transfers `pending.amount` USDC from this contract to `pending.to`,
    /// then removes the pending snapshot to prevent replay.
    ///
    /// # Arguments
    /// * `caller` — Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NoPendingEmergencyDrain`] - no proposal exists.
    /// * [`RevenuePoolError::TimelockNotExpired`] - execution is too early.
    /// * [`RevenuePoolError::InsufficientBalance`] - pool balance is too low.
    /// * [`RevenuePoolError::NotInitialized`] - the USDC token is not configured.
    ///
    /// # Events
    /// Emits `emergency_drain_executed` with `caller` as topic and
    /// `(to, amount, proposed_at, executed_at)` as data.
    pub fn execute_emergency_drain(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_not_emergency_paused(&env);
        Self::require_admin(&env, &caller);

        let inst = env.storage().instance();
        let drain: PendingEmergencyDrain = inst
            .get(&Symbol::new(&env, EMERGENCY_DRAIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NoPendingEmergencyDrain));

        let now: u64 = env.ledger().timestamp();
        if now < drain.execute_after {
            env.panic_with_error(RevenuePoolError::TimelockNotExpired);
        }

        let usdc_address: Address = inst
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();

        if usdc.balance(&contract_address) < drain.amount {
            env.panic_with_error(RevenuePoolError::InsufficientBalance);
        }

        // Remove before transfer to prevent re-entrancy replay.
        inst.remove(&Symbol::new(&env, EMERGENCY_DRAIN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        usdc.transfer(&contract_address, &drain.to, &drain.amount);

        env.events().publish(
            (events::event_emergency_drain_executed(&env), caller),
            (drain.to, drain.amount, drain.proposed_at, now),
        );
    }

    /// Cancel a pending emergency drain. Only the admin may call.
    ///
    /// Removes the pending snapshot and emits an audit event. May be called at
    /// any time before execution, including before the timelock expires.
    ///
    /// # Arguments
    /// * `caller` — Must be the current admin; must authorize.
    ///
    /// # Errors
    /// * [`RevenuePoolError::Unauthorized`] - caller is not the current admin.
    /// * [`RevenuePoolError::NoPendingEmergencyDrain`] - no proposal exists.
    ///
    /// # Events
    /// Emits `emergency_drain_cancelled` with `caller` as topic and the full
    /// [`PendingEmergencyDrain`] snapshot as data.
    pub fn cancel_emergency_drain(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);

        let inst = env.storage().instance();
        let drain: PendingEmergencyDrain = inst
            .get(&Symbol::new(&env, EMERGENCY_DRAIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(RevenuePoolError::NoPendingEmergencyDrain));

        inst.remove(&Symbol::new(&env, EMERGENCY_DRAIN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        env.events().publish(
            (events::event_emergency_drain_cancelled(&env), caller),
            drain,
        );
    }

    /// Return the pending emergency drain proposal, or `None` if none exists.
    ///
    /// Off-chain monitors and the admin can poll this to verify or cancel a
    /// pending drain before the timelock expires.
    pub fn get_pending_emergency_drain(env: Env) -> Option<PendingEmergencyDrain> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, EMERGENCY_DRAIN_KEY))
    }
}

// ---------------------------------------------------------------------------
// chunk_iter — pure batch-splitting helper
// ---------------------------------------------------------------------------

/// Split `payments` into consecutive chunks of at most `chunk_size` legs each,
/// preserving order.
///
/// Intended for backend integrators who need to distribute to more than
/// [`MAX_BATCH_SIZE`] developers: pre-chunk the full payout list and submit one
/// [`RevenuePool::batch_distribute`] call per chunk.
///
/// A zero `chunk_size` or empty `payments` yields no chunks.
/// This is a pure helper: no storage access, no auth, no token transfers.
///
/// # Arguments
/// * `env` - Soroban environment used to allocate the returned vectors.
/// * `payments` - Ordered `(recipient, amount)` pairs to split.
/// * `chunk_size` - Maximum number of payment legs per returned chunk.
///
/// # Returns
/// A vector of consecutive payment chunks, each no larger than `chunk_size`.
pub fn chunk_iter(
    env: &Env,
    payments: Vec<(Address, i128)>,
    chunk_size: u32,
) -> Vec<Vec<(Address, i128)>> {
    let mut chunks: Vec<Vec<(Address, i128)>> = Vec::new(env);
    if chunk_size == 0 {
        return chunks;
    }
    let mut current: Vec<(Address, i128)> = Vec::new(env);
    for payment in payments.iter() {
        current.push_back(payment);
        if current.len() == chunk_size {
            chunks.push_back(current);
            current = Vec::new(env);
        }
    }
    if !current.is_empty() {
        chunks.push_back(current);
    }
    chunks
}

// ---------------------------------------------------------------------------
// Test modules
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test_balance;

#[cfg(test)]
mod test_contract_errors;

#[cfg(test)]
mod test_error_codes;

#[cfg(test)]
mod test_emergency;

#[cfg(test)]
mod test_events;

#[cfg(test)]
mod test_invariant;

#[cfg(test)]
mod test_proptest;

#[cfg(test)]
mod test_reentrancy;

#[cfg(test)]
mod test_storage_migration;

#[cfg(test)]
extern crate std;

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
