#![no_std]

pub mod errors;
pub mod events;
pub mod limits;
pub mod pause;

use crate::errors::DistributeError;
use crate::limits::StorageKey;

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, BytesN, Env, String, Vec,
};

// ---------------------------------------------------------------------------
// Storage key constants (non-pause keys — pause uses StorageKey::Paused)
// ---------------------------------------------------------------------------

const USDC_KEY: &str = "usdc";
const PENDING_ADMIN_KEY: &str = "pending_admin";
const MAX_DISTRIBUTE_KEY: &str = "max_distribute";
const VERSION_KEY: &str = "version";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default per-leg distribution cap — effectively unlimited until explicitly set.
pub const DEFAULT_MAX_DISTRIBUTE: i128 = i128::MAX;

/// TTL bump constants for instance storage archival risk mitigation.
pub const BUMP_AMOUNT: u32 = 10_000;
pub const LIFETIME_THRESHOLD: u32 = 1_000;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// A single payment leg in a batch distribution: `(recipient, amount)`.
///
/// Using a named `contracttype` struct avoids the Soroban restriction on
/// anonymous tuple generics in contract function signatures.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BatchItem {
    pub to: Address,
    pub amount: i128,
}

/// Severity levels for admin broadcast messages.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraDistribute;

/// Re-export the struct under the legacy name so existing test helpers that
/// import `Distribute` keep compiling without changes.
pub use CalloraDistribute as Distribute;
#[contractimpl]
impl CalloraDistribute {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the distribute contract with an admin and the USDC token address.
    ///
    /// Can only be called once. Rejects `usdc_token == contract address` or
    /// `usdc_token == admin`.
    ///
    /// # Errors
    /// * [`DistributeError::AlreadyInitialized`] — called more than once.
    /// * [`DistributeError::InvalidConfig`] — bad token/admin combination.
    ///
    /// # Events
    /// Emits `init` with `(admin)` as topics and `usdc_token` as data.
    pub fn init(env: Env, admin: Address, usdc_token: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            env.panic_with_error(DistributeError::AlreadyInitialized);
        }
        let contract_addr = env.current_contract_address();
        if usdc_token == contract_addr {
            env.panic_with_error(DistributeError::InvalidConfig);
        }
        if usdc_token == admin {
            env.panic_with_error(DistributeError::InvalidConfig);
        }
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::Usdc, &usdc_token);
        // Initialise the pause flag to false via the canonical key.
        inst.set(&StorageKey::Paused, &false);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events()
            .publish((events::event_init(&env), events::event_version_v1(&env), admin), usdc_token);
    }

    // -----------------------------------------------------------------------
    // Admin helpers (internal)
    // -----------------------------------------------------------------------

    fn admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized))
    }

    fn require_admin(env: &Env, caller: &Address) {
        if *caller != Self::admin(env) {
            env.panic_with_error(DistributeError::Unauthorized);
        }
    }

    fn validate_recipient(env: &Env, recipient: &Address, contract_self: &Address) {
        if recipient == contract_self {
            env.panic_with_error(DistributeError::InvalidRecipient);
        }
    }

    // -----------------------------------------------------------------------
    // Admin view
    // -----------------------------------------------------------------------

    /// Return the current admin address.
    ///
    /// # Errors
    /// * [`DistributeError::NotInitialized`] — called before `init`.
    pub fn get_admin(env: Env) -> Result<Address, DistributeError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(DistributeError::NotInitialized)
    }

    /// Return the USDC token address configured for this contract.
    ///
    /// # Errors
    /// * [`DistributeError::NotInitialized`] — called before `init`.
    pub fn get_usdc_token(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&StorageKey::Usdc)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized))
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    /// The nominee must call `accept_admin` to complete.
    ///
    /// # Events
    /// Emits `admin_changed` and `admin_transfer_started`.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        let inst = env.storage().instance();
        inst.set(&StorageKey::PendingAdmin, &new_admin);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_admin_changed(&env),
                events::event_version_v1(&env),
                current.clone(),
            ),
            (current.clone(), new_admin.clone()),
        );
        env.events().publish(
            (
                events::event_admin_transfer_started(&env),
                events::event_version_v1(&env),
                current,
            ),
            new_admin,
        );
    }

    /// Complete the admin transfer. Only the pending admin may call.
    ///
    /// # Errors
    /// * [`DistributeError::NoAdminTransferPending`] — no transfer in progress.
    /// * [`DistributeError::Unauthorized`] — wrong caller.
    ///
    /// # Events
    /// Emits `admin_transfer_completed`.
    pub fn accept_admin(env: Env, caller: Address) {
        caller.require_auth();
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&StorageKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NoAdminTransferPending));
        if caller != pending {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        inst.set(&StorageKey::Admin, &pending);
        inst.remove(&StorageKey::PendingAdmin);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_admin_transfer_completed(&env),
                events::event_version_v1(&env),
                pending,
            ),
            (),
        );
    }

    /// Alias for `accept_admin`.
    pub fn claim_admin(env: Env, caller: Address) {
        Self::accept_admin(env, caller);
    }

    /// Cancel a pending admin transfer. Only the current admin may call.
    ///
    /// # Events
    /// Emits `admin_cancelled`.
    pub fn cancel_admin_transfer(env: Env, caller: Address) {
        caller.require_auth();
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&StorageKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NoAdminTransferPending));
        inst.remove(&StorageKey::PendingAdmin);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_admin_cancelled(&env),
                events::event_version_v1(&env),
                current,
                pending,
            ),
            (),
        );
    }

    /// Return the pending admin address, or `None` if no transfer is in progress.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::PendingAdmin)
    }

    // -----------------------------------------------------------------------
    // Pause / unpause — delegated to the `pause` module
    //
    // All reads and writes use `StorageKey::Paused` — the single canonical key.
    // -----------------------------------------------------------------------

    /// Activate the circuit-breaker. Blocks `distribute` and `batch_distribute`.
    /// Only the admin may call.
    ///
    /// # Errors
    /// * [`DistributeError::Unauthorized`] — caller is not the admin.
    /// * [`DistributeError::AlreadyPaused`] — contract is already paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `true` as data.
    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if pause::is_paused(&env) {
            env.panic_with_error(DistributeError::AlreadyPaused);
        }
        // Delegate the write to the pause module (uses StorageKey::Paused).
        pause::set_paused(&env, true);
        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_pause_set(&env), events::event_version_v1(&env), caller),
            true,
        );
    }

    /// Deactivate the circuit-breaker. Only the admin may call.
    ///
    /// # Errors
    /// * [`DistributeError::Unauthorized`] — caller is not the admin.
    /// * [`DistributeError::NotPaused`] — contract is not currently paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `false` as data.
    pub fn unpause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if !pause::is_paused(&env) {
            env.panic_with_error(DistributeError::NotPaused);
        }
        // Delegate the write to the pause module (uses StorageKey::Paused).
        pause::set_paused(&env, false);
        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_pause_set(&env), events::event_version_v1(&env), caller),
            false,
        );
    }

    /// Return `true` if the contract is currently paused.
    /// Reads from the canonical `StorageKey::Paused` key.
    pub fn is_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    /// Alias for `is_paused` for backwards compatibility.
    pub fn get_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    // -----------------------------------------------------------------------
    // Distribution cap
    // -----------------------------------------------------------------------

    /// Return the per-leg distribution cap. Defaults to `i128::MAX` when unset.
    pub fn get_max_distribute(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MaxDistribute)
            .unwrap_or(DEFAULT_MAX_DISTRIBUTE)
    }

    /// Return the configured maximum batch size.
    pub fn get_max_batch_size(_env: Env) -> u32 {
        limits::MAX_BATCH_SIZE
    }

    /// Set the maximum amount distributable per leg. Must be positive. Admin only.
    ///
    /// # Events
    /// Emits `set_max_distribute` with `(old_max, new_max)`.
    pub fn set_max_distribute(env: Env, caller: Address, max_distribute: i128) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if max_distribute <= 0 {
            env.panic_with_error(DistributeError::CapNotPositive);
        }
        let old_max = Self::get_max_distribute(env.clone());
        env.storage()
            .instance()
            .set(&StorageKey::MaxDistribute, &max_distribute);
        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_set_max_distribute(&env),
                events::event_version_v1(&env),
                Self::admin(&env),
            ),
            (old_max, max_distribute),
        );
    }

    // -----------------------------------------------------------------------
    // Distribution
    // -----------------------------------------------------------------------

    /// Distribute USDC from this contract to a single recipient.
    ///
    /// # Events
    /// Emits `distribute_started`, `distribute`, and `distribute_completed`.
    pub fn distribute(env: Env, caller: Address, to: Address, amount: i128) {
        caller.require_auth();
        pause::require_not_paused(&env);
        Self::require_admin(&env, &caller);
        if amount <= 0 {
            env.panic_with_error(DistributeError::AmountNotPositive);
        }
        let max_distribute = Self::get_max_distribute(env.clone());
        if amount > max_distribute {
            env.panic_with_error(DistributeError::AmountExceedsMaxDistribute);
        }
        let usdc_address: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Usdc)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        Self::validate_recipient(&env, &to, &contract_address);
        if usdc.balance(&contract_address) < amount {
            env.panic_with_error(DistributeError::InsufficientBalance);
        }
        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_distribute_started(&env), events::event_version_v1(&env), to.clone()),
            amount,
        );
        usdc.transfer(&contract_address, &to, &amount);
        env.events().publish(
            (events::event_distribute(&env), events::event_version_v1(&env), to.clone()),
            amount,
        );
        env.events().publish(
            (events::event_distribute_completed(&env), events::event_version_v1(&env), to),
            amount,
        );
    }

    /// Distribute USDC to multiple recipients atomically.
    ///
    /// Each [`BatchItem`] carries a recipient and amount. All legs are
    /// validated before any USDC is transferred (fail-early, all-or-nothing).
    ///
    /// # Events
    /// Emits `batch_distribute_started` and `batch_distribute_completed`.
    pub fn batch_distribute(env: Env, caller: Address, payments: Vec<BatchItem>) {
        caller.require_auth();
        pause::require_not_paused(&env);
        Self::require_admin(&env, &caller);

        let n = payments.len();
        if n == 0 {
            env.panic_with_error(DistributeError::BatchEmpty);
        }
        let max_batch = limits::MAX_BATCH_SIZE;
        if n > max_batch {
            env.panic_with_error(DistributeError::BatchTooLarge);
        }

        let usdc_address: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Usdc)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        let max_distribute = Self::get_max_distribute(env.clone());

        // Phase 1 — validate all legs and compute total.
        let mut total: i128 = 0;
        for i in 0..n {
            let item = payments.get(i).unwrap_or_else(|| env.panic_with_error(DistributeError::BatchEmpty));
            if item.amount <= 0 {
                env.panic_with_error(DistributeError::AmountNotPositive);
            }
            if item.amount > max_distribute {
                env.panic_with_error(DistributeError::AmountExceedsMaxDistribute);
            }
            Self::validate_recipient(&env, &item.to, &contract_address);
            total = total
                .checked_add(item.amount)
                .unwrap_or_else(|| env.panic_with_error(DistributeError::Overflow));
        }

        // Phase 2 — check total balance.
        if usdc.balance(&contract_address) < total {
            env.panic_with_error(DistributeError::InsufficientBalance);
        }

        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        // Phase 3 — emit started event.
        env.events().publish(
            (
                events::event_batch_distribute_started(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            (total, n),
        );

        // Phase 4 — execute transfers.
        for i in 0..n {
            let item = payments.get(i).unwrap_or_else(|| env.panic_with_error(DistributeError::BatchEmpty));
            usdc.transfer(&contract_address, &item.to, &item.amount);
        }

        // Phase 5 — emit completed event.
        env.events().publish(
            (
                events::event_batch_distribute_completed(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (total, n),
        );
    }

    // -----------------------------------------------------------------------
    // Balance view
    // -----------------------------------------------------------------------

    /// Return this contract's on-ledger USDC balance.
    pub fn balance(env: Env) -> i128 {
        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Usdc)
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_addr);
        usdc.balance(&env.current_contract_address())
    }

    // -----------------------------------------------------------------------
    // Broadcast
    // -----------------------------------------------------------------------

    /// Emit an admin broadcast message. Admin only.
    ///
    /// # Events
    /// Emits `broadcast` with `(caller, severity)` as topics and `message` as data.
    pub fn broadcast(env: Env, caller: Address, severity: Severity, message: String) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        env.events().publish(
            (events::event_broadcast(&env), events::event_version_v1(&env), caller, severity),
            message,
        );
    }

    // -----------------------------------------------------------------------
    // Upgrade
    // -----------------------------------------------------------------------

    /// Admin-gated contract upgrade. Replaces the WASM and persists the version.
    ///
    /// # Events
    /// Emits `upgraded` with `admin` as topic and `new_wasm_hash` as data.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.storage()
            .instance()
            .set(&StorageKey::ContractVersion, &new_wasm_hash.clone());
        env.storage().instance().extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (events::event_upgraded(&env), events::event_version_v1(&env), Self::admin(&env)),
            new_wasm_hash,
        );
    }

    /// Return the stored WASM version hash, or `None` if never upgraded.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        env.storage().instance().get(&StorageKey::ContractVersion)
    }

    /// Return the crate version string baked in at compile time.
    pub fn version(env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION"))
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
extern crate std;

// Re-export legacy client alias so existing tests continue to compile.
#[cfg(any(test, feature = "testutils"))]
pub use CalloraDistributeClient as DistributeClient;
