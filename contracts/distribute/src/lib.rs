#![no_std]

//! Callora Distribute contract.
//!
//! Immediate, admin-authorized token distributions with a per-leg amount cap
//! and a per-call batch size limit. Payments do not create per-account state
//! entries or pending payouts, and there is no per-account state cap.
//!
//! # Instance storage TTL policy
//!
//! Critical configuration lives in Soroban instance storage. Reads count as
//! active use: every public view refreshes instance TTL at or below a 30-day
//! threshold, extending it back to a 60-day target, matching the vault policy.
//!
//! TTL changes persist only when the invocation is submitted and committed.
//! RPC simulation alone does not extend on-chain lifetime; clients that only
//! simulate views still need a submitted invocation or TTL-extension transaction.

pub mod events;
pub mod errors;
pub mod limits;

use crate::errors::DistributeError;

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage key constants
// ---------------------------------------------------------------------------

const ADMIN_KEY: &str = "admin";
const USDC_KEY: &str = "usdc";
const PAUSED_KEY: &str = "paused";
const PENDING_ADMIN_KEY: &str = "pending_admin";
const MAX_DISTRIBUTE_KEY: &str = "max_distribute";
const VERSION_KEY: &str = "version";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default per-leg distribution cap â€” effectively unlimited until explicitly set.
pub const DEFAULT_MAX_DISTRIBUTE: i128 = i128::MAX;

/// Ledgers per day at the network's approximately five-second close cadence.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// Refresh instance storage when at most ~30 days of TTL remain.
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

/// Extend instance storage back to ~60 days from the current ledger.
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 60;

/// Backwards-compatible alias for the previous public constant name.
pub const LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_THRESHOLD;

/// Backwards-compatible alias for the previous public constant name.
pub const BUMP_AMOUNT: u32 = INSTANCE_BUMP_AMOUNT;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct Distribute;

#[contractimpl]
impl Distribute {
    /// Refresh instance storage TTL using the workspace-wide 30/60-day policy.
    #[inline]
    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the distribute contract with an admin and the USDC token address.
    ///
    /// Can only be called once. Rejects `usdc_token == contract address`.
    ///
    /// # Panics
    /// * `DistributeError::AlreadyInitialized` â€” called more than once.
    /// * `DistributeError::InvalidConfig` â€” bad token address or token/admin aliasing.
    ///
    /// # Events
    /// Emits `init` with `admin` as topic and `usdc_token` as data.
    pub fn init(env: Env, admin: Address, usdc_token: Address) {
        if env.storage().instance().has(&Symbol::new(&env, ADMIN_KEY)) {
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
        inst.set(&Symbol::new(&env, ADMIN_KEY), &admin);
        inst.set(&Symbol::new(&env, USDC_KEY), &usdc_token);
        inst.set(&Symbol::new(&env, PAUSED_KEY), &false);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_init(&env),
                events::event_version_v1(&env),
                admin,
            ),
            usdc_token,
        );
    }

    // -----------------------------------------------------------------------
    // Admin helpers (internal)
    // -----------------------------------------------------------------------

    fn admin(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(env, ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized))
    }

    fn require_admin(env: &Env, caller: &Address) {
        if *caller != Self::admin(env) {
            env.panic_with_error(DistributeError::Unauthorized);
        }
    }

    fn require_not_paused(env: &Env) {
        if env
            .storage()
            .instance()
            .get::<_, bool>(&Symbol::new(env, PAUSED_KEY))
            .unwrap_or(false)
        {
            env.panic_with_error(DistributeError::Paused);
        }
    }

    fn is_paused(env: &Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&Symbol::new(env, PAUSED_KEY))
            .unwrap_or(false)
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
    /// # Panics
    /// * `DistributeError::NotInitialized` â€” called before `init`.
    pub fn get_admin(env: Env) -> Address {
        let admin = Self::admin(&env);
        Self::bump_instance_ttl(&env);
        admin
    }

    /// Return the USDC token address configured for this contract.
    ///
    /// # Panics
    /// * `DistributeError::NotInitialized` â€” called before `init`.
    pub fn get_usdc_token(env: Env) -> Address {
        let usdc = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        Self::bump_instance_ttl(&env);
        usdc
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    /// The nominee must call `claim_admin` to complete.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    ///
    /// # Events
    /// Emits `admin_transfer_started` with `current` as topic and `new_admin`
    /// as data. No `admin_changed` event is published while the transfer is
    /// only pending — the admin does not change until `accept_admin`
    /// (Issue #1163), so a nomination that is later cancelled never announces
    /// a change that did not happen.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        let inst = env.storage().instance();
        inst.set(&Symbol::new(&env, PENDING_ADMIN_KEY), &new_admin);
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
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
    /// # Panics
    /// * `DistributeError::NoAdminTransferPending` â€” no transfer is in progress.
    /// * `DistributeError::Unauthorized` â€” wrong caller.
    ///
    /// # Events
    /// Emits `admin_changed` with the previous admin as topic and
    /// `(previous_admin, new_admin)` as data, followed by
    /// `admin_transfer_completed` with the new admin as topic. Both are
    /// published only after the admin slot is updated, so indexers observe
    /// the change exactly when it happens (Issue #1163).
    pub fn accept_admin(env: Env, caller: Address) {
        caller.require_auth();
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NoAdminTransferPending));
        if caller != pending {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        let previous = Self::admin(&env);
        inst.set(&Symbol::new(&env, ADMIN_KEY), &pending);
        inst.remove(&Symbol::new(&env, PENDING_ADMIN_KEY));
        inst.extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_admin_changed(&env),
                events::event_version_v1(&env),
                previous.clone(),
            ),
            (previous, pending.clone()),
        );
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
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::NoAdminTransferPending` â€” no transfer in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled` with `(current_admin, pending_admin)`.
    pub fn cancel_admin_transfer(env: Env, caller: Address) {
        caller.require_auth();
        let current = Self::admin(&env);
        if caller != current {
            env.panic_with_error(DistributeError::Unauthorized);
        }
        let inst = env.storage().instance();
        let pending: Address = inst
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NoAdminTransferPending));
        inst.remove(&Symbol::new(&env, PENDING_ADMIN_KEY));
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
        let pending = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, PENDING_ADMIN_KEY));
        Self::bump_instance_ttl(&env);
        pending
    }

    // -----------------------------------------------------------------------
    // Pause / unpause
    // -----------------------------------------------------------------------

    /// Activate the circuit-breaker. Blocks `distribute`.
    /// Only the admin may call.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::AlreadyPaused` â€” contract is already paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `true` as data.
    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if Self::is_paused(&env) {
            env.panic_with_error(DistributeError::AlreadyPaused);
        }
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PAUSED_KEY), &true);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_pause_set(&env),
                events::event_version_v1(&env),
                caller,
            ),
            true,
        );
    }

    /// Deactivate the circuit-breaker. Only the admin may call.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::NotPaused` â€” contract is not currently paused.
    ///
    /// # Events
    /// Emits `pause_set` with `caller` as topic and `false` as data.
    pub fn unpause(env: Env, caller: Address) {
        caller.require_auth();
        Self::require_admin(&env, &caller);
        if !Self::is_paused(&env) {
            env.panic_with_error(DistributeError::NotPaused);
        }
        env.storage()
            .instance()
            .set(&Symbol::new(&env, PAUSED_KEY), &false);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_pause_set(&env),
                events::event_version_v1(&env),
                caller,
            ),
            false,
        );
    }

    /// Return `true` if the contract is currently paused.
    pub fn get_paused(env: Env) -> bool {
        let paused = Self::is_paused(&env);
        Self::bump_instance_ttl(&env);
        paused
    }

    // -----------------------------------------------------------------------
    // Distribution cap
    // -----------------------------------------------------------------------

    /// Return the per-leg distribution cap. Defaults to `i128::MAX` when unset.
    pub fn get_max_distribute(env: Env) -> i128 {
        let max_distribute = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, MAX_DISTRIBUTE_KEY))
            .unwrap_or(DEFAULT_MAX_DISTRIBUTE);
        Self::bump_instance_ttl(&env);
        max_distribute
    }

    /// Return the configured maximum batch size.
    pub fn get_max_batch_size(env: Env) -> u32 {
        Self::bump_instance_ttl(&env);
        limits::MAX_BATCH_SIZE
    }

    /// Set the maximum amount distributable per leg. Must be positive. Admin only.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::CapNotPositive` â€” value â‰¤ 0.
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
            .set(&Symbol::new(&env, MAX_DISTRIBUTE_KEY), &max_distribute);
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
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
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::Paused` â€” contract is paused.
    /// * `DistributeError::AmountNotPositive` â€” amount â‰¤ 0.
    /// * `DistributeError::AmountExceedsMaxDistribute` â€” amount exceeds the cap.
    /// * `DistributeError::InvalidRecipient`.
    /// * `DistributeError::InsufficientBalance` â€” contract holds less than `amount`.
    ///
    /// # Events
    /// Emits `distribute_started` with `to` as topic and `amount` as data.
    /// Emits `distribute` with `to` as topic and `amount` as data (legacy).
    /// Emits `distribute_completed` with `to` as topic and `amount` as data.
    pub fn distribute(env: Env, caller: Address, to: Address, amount: i128) {
        caller.require_auth();
        Self::require_not_paused(&env);
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
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        Self::validate_recipient(&env, &to, &contract_address);
        if usdc.balance(&contract_address) < amount {
            env.panic_with_error(DistributeError::InsufficientBalance);
        }
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_distribute_started(&env),
                events::event_version_v1(&env),
                to.clone(),
            ),
            amount,
        );
        usdc.transfer(&contract_address, &to, &amount);
        env.events().publish(
            (
                events::event_distribute(&env),
                events::event_version_v1(&env),
                to.clone(),
            ),
            amount,
        );
        env.events().publish(
            (
                events::event_distribute_completed(&env),
                events::event_version_v1(&env),
                to,
            ),
            amount,
        );
    }

    /// Distribute USDC from this contract to multiple recipients in a single
    /// atomic transaction.
    ///
    /// Each leg is a `(Address, i128)` tuple of `(recipient, amount)`.  The
    /// function validates every leg before transferring any USDC â€” if *any*
    /// leg fails validation the entire batch is reverted (fail-early / all-or-nothing).
    ///
    /// # Validation (per-leg, fail-early)
    /// - `amount` must be positive.
    /// - `amount` must not exceed `max_distribute` (per-leg cap).
    /// - `recipient` must not be the contract address itself.
    ///
    /// # Validation (batch-level)
    /// - Caller must be authorised admin.
    /// - Contract must not be paused.
    /// - Batch must not be empty.
    /// - Batch size must not exceed `MAX_BATCH_SIZE`.
    /// - The sum of all amounts must not exceed the contract's USDC balance.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
    /// * `DistributeError::Paused` â€” contract is paused.
    /// * `DistributeError::BatchEmpty` â€” no payment legs provided.
    /// * `DistributeError::BatchTooLarge` â€” more than `MAX_BATCH_SIZE` legs.
    /// * `DistributeError::Overflow` - the sum of all leg amounts overflows `i128`.
    /// * `DistributeError::AmountNotPositive` â€” any leg has amount â‰¤ 0.
    /// * `DistributeError::AmountExceedsMaxDistribute` â€” any leg exceeds per-leg cap.
    /// * `DistributeError::InvalidRecipient`.
    /// * `DistributeError::InsufficientBalance` â€” contract holds less than `total`.
    ///
    /// # Events
    /// - Emits `batch_distribute_started` with `caller` as topic and `(total, count)` as data.
    /// - For each payment leg, emits in payment order:
    ///   - `distribute_started` with `(distribute_started, callora_v1, recipient)` topic and
    ///     `DistributionLifecycleEvent` payload before the transfer.
    ///   - `distribute` with `(distribute, callora_v1, recipient)` topic and `amount` data.
    ///   - `distribute_completed` with `(distribute_completed, callora_v1, recipient)` topic and
    ///     `DistributionLifecycleEvent` payload after successful transfer.
    /// - Emits `batch_distribute_completed` with `caller` as topic and `(total, count)` as data.
    pub fn batch_distribute(env: Env, caller: Address, payments: Vec<(Address, i128)>) {
        caller.require_auth();
        Self::require_not_paused(&env);
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
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_address);
        let contract_address = env.current_contract_address();
        let max_distribute = Self::get_max_distribute(env.clone());

        // Phase 1 — validate all legs and compute total
        let mut total: i128 = 0;
        for i in 0..n {
            let (ref to, amount) = payments.get(i).expect("payment leg");
            if amount <= 0 {
                env.panic_with_error(DistributeError::AmountNotPositive);
            }
            if amount > max_distribute {
                env.panic_with_error(DistributeError::AmountExceedsMaxDistribute);
            }
            Self::validate_recipient(&env, to, &contract_address);
            // Overflow-safe accumulation
            total = total
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(DistributeError::Overflow));
        }

        // Phase 2 — check total balance
        if usdc.balance(&contract_address) < total {
            env.panic_with_error(DistributeError::InsufficientBalance);
        }

        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);

        // Phase 3 — emit started event
        env.events().publish(
            (
                events::event_batch_distribute_started(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            (total, n),
        );

        // Phase 4 — execute transfers with per-leg transfer and lifecycle events
        for i in 0..n {
            let (to, amount) = payments.get(i).expect("payment leg");
            let lifecycle = events::DistributionLifecycleEvent::new(
                &env,
                amount,
                events::DistributionMode::Batch,
                i,
                n,
            );
            events::emit_distribute_started(&env, &to, &lifecycle);
            usdc.transfer(&contract_address, &to, &amount);
            events::emit_distribute(&env, &to, amount);
            events::emit_distribute_completed(&env, &to, &lifecycle);
        }

        // Phase 5 — emit completed event
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
    ///
    /// # Panics
    /// * `DistributeError::NotInitialized` â€” called before `init`.
    pub fn balance(env: Env) -> i128 {
        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, USDC_KEY))
            .unwrap_or_else(|| env.panic_with_error(DistributeError::NotInitialized));
        let usdc = token::Client::new(&env, &usdc_addr);
        let balance = usdc.balance(&env.current_contract_address());
        Self::bump_instance_ttl(&env);
        balance
    }

    // -----------------------------------------------------------------------
    // Upgrade
    // -----------------------------------------------------------------------

    /// Admin-gated contract upgrade. Replaces the WASM and persists the version.
    ///
    /// # Panics
    /// * `DistributeError::Unauthorized` â€” caller is not the current admin.
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
            .set(&Symbol::new(&env, VERSION_KEY), &new_wasm_hash.clone());
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
        env.events().publish(
            (
                events::event_upgraded(&env),
                events::event_version_v1(&env),
                Self::admin(&env),
            ),
            new_wasm_hash,
        );
    }

    /// Return the stored WASM version hash, or `None` if never upgraded.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        let version = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, VERSION_KEY));
        Self::bump_instance_ttl(&env);
        version
    }

    /// Return the crate version string baked in at compile time.
    pub fn version(env: Env) -> soroban_sdk::String {
        let version = soroban_sdk::String::from_str(&env, env!("CARGO_PKG_VERSION"));
        Self::bump_instance_ttl(&env);
        version
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test_ttl;
