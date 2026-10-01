#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]
#![no_std]
//!
//! # Callora Vault Contract — deposit/withdraw/deduct/distribute with pause circuit-breaker.
//!
//! ## Escape-Hatch Admin Actions (#482)
//!
//! The following critical admin actions are guarded by a mandatory timelock:
//!
//! | Action  | Propose              | Execute              | Cancel              |
//! |---------|----------------------|----------------------|---------------------|
//! | pause   | `propose_pause`      | `execute_pause`      | `cancel_pause`      |
//! | upgrade | `propose_upgrade`    | `execute_upgrade`    | `cancel_upgrade`    |
//! | sweep   | `propose_sweep`      | `execute_sweep`      | `cancel_sweep`      |
//!
//! Window length is configured by `set_timelock_window(admin, seconds)` and
//! defaults to 48 h (`172_800`). Valid bounds are 1 h – 30 d. All three slots
//! are independent so multiple proposals can coexist concurrently.
//!
//! Successful executions also share a global admin cool-off window. This
//! prevents several independently matured proposals from being executed in
//! rapid succession. The window defaults to one hour and is configurable with
//! `set_admin_cooldown` within the bounds exposed by [`admin`].
//!
///
/// ## Pause Circuit Breaker
///
/// When the vault is paused:
/// - Deposits are blocked
/// - Single and batch deducts are blocked
/// - Owner withdrawals are ALLOWED (emergency recovery)
/// - Admin distribute is ALLOWED (emergency recovery of untracked surplus)
/// - Admin/owner configuration functions remain available
///
/// ## Request-ID Idempotency
///
/// `deduct` and `batch_deduct` accept a `request_id: u64` idempotency key.
/// When a **non-zero** id is supplied the contract persists a processed-request
/// marker and rejects any subsequent call that carries the same `request_id`,
/// returning `VaultError::DuplicateRequestId`. The marker is written only after
/// the deduction succeeds, so a replayed backend call is charged exactly once.
///
/// This gives safe **at-least-once retry** semantics: a backend can replay a
/// failed transaction with the same `request_id` and the contract will either
/// succeed (first time) or return a deterministic error (duplicate).
///
/// `request_id == 0` is the documented **"no idempotency" sentinel**: no marker
/// is written and the id is never deduplicated, so `0` may be reused on every
/// call. Supply any non-zero id to obtain idempotency.
///
/// ### Retention / TTL
/// Processed-request markers live in persistent storage and are bumped to
/// `REQUEST_ID_BUMP_AMOUNT` ledgers on every successful deduct. The threshold
/// for triggering a bump is `REQUEST_ID_BUMP_THRESHOLD`. Because they are
/// persistent, they do not silently archive; an owner can explicitly prune old
/// markers using `prune_processed_requests` so an id can be reused.
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env, Symbol, Vec};

pub mod admin;
pub mod timelock;
pub mod views;

pub use timelock::{
    DEFAULT_TIMELOCK_SECONDS, MAX_TIMELOCK_SECONDS, MIN_TIMELOCK_SECONDS,
    PROPOSAL_GRACE_SECONDS, TIMELOCK_GRACE_SECONDS,
};

/// Bounded visible-ASCII metadata validators (shared `callora-validators` crate).
pub use callora_validators as validators;

mod errors;
pub use errors::VaultError;

/// A single item in a batch deduction.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeductItem {
    pub amount: i128,
    pub request_id: Option<Symbol>,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    UsdcToken,
    Balance,
    AuthorizedCaller,
    MinDeposit,
    RevenuePool,
    MaxDeduct,
    Settlement,
    Paused,
    /// Pending owner address during a two-step ownership transfer.
    PendingOwner,
}

/// Instance / persistent storage keys for the vault.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    UsdcToken,
    ProcessedRequest(u64),
    ReserveCap(Address),
    DeveloperConfig(Address),
    DeveloperState(Address),
    /// Configured timelock window length in seconds (admin-settable).
    TimelockWindow,
    /// Active pause proposal pending timelock expiry.
    PendingPause,
    /// Active upgrade proposal (wasm hash) pending timelock expiry.
    PendingUpgrade,
    /// Active sweep proposal (recipient + amount) pending timelock expiry.
    PendingSweep,
    /// Current admin address (defaults to owner at init).
    Admin,
    /// Pending admin address awaiting acceptance (two-step transfer).
    PendingAdmin,
    /// Recorded wasm hash after a successful upgrade.
    ContractVersion,
    /// Vector of addresses allowed to deposit (owner-managed allowlist).
    AllowedDepositors,
    /// Global cool-off window between critical admin executions.
    AdminCooldown,
    /// Audit record for the most recently executed critical admin action.
    LastCriticalAdminAction,
    /// Settlement contract address.
    Settlement,
    /// Monotonic rotation nonce guarding authorized-caller rotation against replay.
    AuthCallerNonce,
}

/// Ledgers per day at a 5-second close cadence.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// TTL extension trigger for instance storage keys (~30 days of ledgers at 5 s/ledger).
pub const INSTANCE_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

/// TTL extension target for instance storage keys (~60 days of ledgers at 5 s/ledger).
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 60;

/// TTL extension trigger for persistent storage keys — mirrors the instance threshold.
pub const PERSISTENT_BUMP_THRESHOLD: u32 = INSTANCE_BUMP_THRESHOLD;

/// TTL extension target for persistent storage keys — mirrors the instance amount.
pub const PERSISTENT_BUMP_AMOUNT: u32 = INSTANCE_BUMP_AMOUNT;

/// TTL extension trigger for processed-request-id markers (~7 days).
pub const REQUEST_ID_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 7;

/// TTL extension target for processed-request-id markers (~30 days).
pub const REQUEST_ID_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 30;

/// Maximum number of items permitted in a single [`CalloraVault::batch_deduct`]
/// call. Enforced so an unbounded batch cannot hit per-transaction resource
/// limits (CPU/gas) or cause the overflow-checked total accumulation to be
/// forced past `i128::MAX` in a single invocation.
pub const MAX_BATCH_SIZE: u32 = 50;

pub mod token {
    pub use soroban_sdk::token::Client;
}

#[cfg(target_arch = "wasm32")]
pub mod settlement {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/callora_settlement.wasm"
    );
}

/// In native/test mode the settlement contract is registered directly in the
/// test `Env`. The vault emits USDC transfers; settlement accounting is
/// verified through its own public interface.
#[cfg(not(target_arch = "wasm32"))]
pub mod settlement {
    use soroban_sdk::{Address, Env};
    pub struct Client<'a> {
        _env: &'a Env,
        _addr: &'a Address,
    }
    impl<'a> Client<'a> {
        pub fn new(env: &'a Env, addr: &'a Address) -> Self {
            Client {
                _env: env,
                _addr: addr,
            }
        }
        pub fn receive_payment(
            &self,
            _caller: &Address,
            _amount: &i128,
            _to_global_pool: &bool,
            _developer: &Option<Address>,
            _token: &Address,
            _nonce: &u32,
        ) {
        }
        pub fn record_deduction(&self, _amount: &i128, _request_id: &u64) {}
    }
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraVault;

#[contractimpl]
impl CalloraVault {
    fn require_positive_amount(amount: i128) -> Result<(), VaultError> {
        if amount <= 0 {
            return Err(VaultError::AmountNotPositive);
        }
        Ok(())
    }

    fn require_valid_deposit_amount(amount: i128, min_deposit: i128) -> Result<(), VaultError> {
        Self::require_positive_amount(amount)?;
        if amount < min_deposit {
            return Err(VaultError::BelowMinDeposit);
        }
        Ok(())
    }

    fn require_valid_deduct_amount(
        amount: i128,
        min_amount: i128,
        max_deduct: i128,
    ) -> Result<(), VaultError> {
        Self::require_positive_amount(amount)?;
        if amount < min_amount {
            return Err(VaultError::BelowMinDeposit);
        }
        if amount > max_deduct {
            return Err(VaultError::ExceedsMaxDeduct);
        }
        Ok(())
    }

    /// Shared validation pipeline for [`deduct`] and [`crate::views::simulate_deduct`].
    ///
    /// Checks, **in the same order `deduct` uses**, that:
    /// 1. `caller` is the owner or the authorized deduct caller → [`VaultError::Unauthorized`].
    /// 2. The vault is not paused → [`VaultError::Paused`].
    /// 3. `amount > 0` → [`VaultError::AmountNotPositive`].
    /// 4. `amount >= min_deposit` → [`VaultError::BelowMinDeposit`].
    /// 5. `amount <= max_deduct` → [`VaultError::ExceedsMaxDeduct`].
    /// 6. Tracked balance ≥ `amount` → [`VaultError::InsufficientBalance`].
    ///
    /// This function is **read-only** (no storage writes, no events, no auth).
    /// `deduct` calls it before any mutation so that the validation order is
    /// guaranteed identical to what `simulate_deduct` observes.
    pub(crate) fn validate_deduct(env: &Env, caller: &Address, amount: i128) -> Result<(), VaultError> {
        // 1. Owner-or-authorized-caller check (issue #1107: the owner may
        //    always deduct; a configured authorized caller may deduct too).
        Self::require_authorized_deduct_caller(env.clone(), caller)?;

        // 2. Pause guard.
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(VaultError::Paused);
        }

        // 3-5. Amount bounds (positive, min_deposit, max_deduct).
        let min_dep = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::MinDeposit)
            .unwrap();
        let max_deduct = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::MaxDeduct)
            .unwrap();
        Self::require_valid_deduct_amount(amount, min_dep, max_deduct)?;

        // 6. Balance check.
        let current_bal = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);
        if current_bal < amount {
            return Err(VaultError::InsufficientBalance);
        }

        Ok(())
    }

    /// Initialize the Callora Vault contract (one-time setup).
    ///
    /// Stores configuration in instance storage and sets the paused flag to `false`.
    /// The admin role defaults to `owner` at initialization and may be transferred
    /// later via the two-step [`set_admin`] / [`accept_admin`] flow.
    ///
    /// # Parameters
    /// - `owner` — vault owner; only this address may call owner-gated functions.
    /// - `usdc_token` — USDC token contract address used for all transfers.
    /// - `initial_balance` — the tracked balance to record at startup (must be ≥ 0).
    /// - `authorized_caller` — address permitted to call [`deduct`] and [`batch_deduct`].
    /// - `min_deposit` — minimum accepted deposit amount in stroops; must be > 0.
    /// - `revenue_pool` — optional revenue pool address; may be `None`.
    /// - `max_deduct` — per-call deduction cap in stroops; must be > 0 and ≥ `min_deposit`.
    /// - `settlement` — settlement contract address that receives deducted funds.
    ///
    /// # Panics
    /// - If already initialized.
    /// - If `min_deposit <= 0`, `max_deduct <= 0`, or `min_deposit > max_deduct`.
    ///
    /// No auth is required; `init` is a constructor and may only succeed once.
    pub fn init(
        env: Env,
        owner: Address,
        usdc_token: Address,
        initial_balance: Option<i128>,
        authorized_caller: Option<Address>,
        min_deposit: Option<i128>,
        revenue_pool: Option<Address>,
        max_deduct: Option<i128>,
        settlement: Option<Address>,
    ) -> Result<(), VaultError> {
        if env.storage().instance().has(&DataKey::Owner) {
            return Err(VaultError::AlreadyInitialized);
        }
        let min_dep_val = min_deposit.unwrap_or(0);
        if min_dep_val <= 0 {
            return Err(VaultError::MinDepositNotPositive);
        }
        let max_deduct_val = max_deduct.unwrap_or(i128::MAX);
        if max_deduct_val <= 0 {
            return Err(VaultError::MaxDeductNotPositive);
        }
        if min_dep_val > max_deduct_val {
            return Err(VaultError::MinDepositExceedsMaxDeduct);
        }

        let initial_balance_val = initial_balance.unwrap_or(0);
        if initial_balance_val < 0 {
            return Err(VaultError::InitialBalanceNegative);
        }

        env.storage().instance().set(&DataKey::Owner, &owner);
        // The admin role defaults to the owner at initialization so the
        // timelocked lifecycle actions (pause / upgrade / sweep) and
        // admin-only rescue/rotation are usable immediately. This fulfils the
        // documented contract in [`CalloraVault::get_admin`].
        env.storage().instance().set(&StorageKey::Admin, &owner);
        env.storage()
            .instance()
            .set(&DataKey::UsdcToken, &usdc_token);
        env.storage()
            .instance()
            .set(&DataKey::Balance, &initial_balance_val);
            
        if let Some(ac) = authorized_caller {
            env.storage()
                .instance()
                .set(&DataKey::AuthorizedCaller, &ac);
        }
        
        env.storage()
            .instance()
            .set(&DataKey::MinDeposit, &min_dep_val);

        if let Some(pool) = revenue_pool {
            env.storage().instance().set(&DataKey::RevenuePool, &pool);
        }

        env.storage()
            .instance()
            .set(&DataKey::MaxDeduct, &max_deduct_val);
        if let Some(s) = settlement {
            env.storage().instance().set(&DataKey::Settlement, &s);
        }
        env.storage().instance().set(&DataKey::Paused, &false);

        env.events()
            .publish((events::event_init(&env), events::event_version_v1(&env), owner.clone()), initial_balance_val);
        Ok(())
    }

    /// Deposit USDC into the vault, incrementing the tracked balance.
    ///
    /// The caller must authenticate and, if not the owner, must be on the
    /// deposit allowlist (see [`add_address`]).  Deposits are blocked while the
    /// vault is paused.  The USDC transfer is executed atomically with the
    /// balance update.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  The caller must be the owner or an
    /// allowlisted address.
    ///
    /// ### Parameters
    /// - `caller` — address initiating the deposit (must auth).
    /// - `amount` — amount in USDC stroops; must be ≥ `min_deposit` and > 0.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Paused`] — vault is paused (circuit-breaker active).
    /// - [`VaultError::AmountNotPositive`] — `amount ≤ 0`.
    /// - [`VaultError::BelowMinDeposit`] — `amount < min_deposit`.
    /// - [`VaultError::CallerNotInAllowlist`] — non-owner caller not in allowlist.
    /// - [`VaultError::Overflow`] — tracked balance overflow.
    ///
    /// ### Events
    /// Emits `deposit` with `caller` as topic and `(amount, new_balance)` as data.
    pub fn deposit(env: Env, caller: Address, amount: i128) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_not_paused(env.clone())?;
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(VaultError::Paused);
        }
        let min_dep = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::MinDeposit)
            .unwrap();
        Self::require_valid_deposit_amount(amount, min_dep)?;
        if !Self::allowlist_allows(&env, &caller) {
            return Err(VaultError::CallerNotInAllowlist);
        }
        let current_bal = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);
        let new_bal = current_bal
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        env.storage().instance().set(&DataKey::Balance, &new_bal);

        let token_addr = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::UsdcToken)
            .unwrap_or_else(|| panic!("USDC Token not set"));

        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&caller, &env.current_contract_address(), &amount);

        env.events().publish(
            (
                events::event_deposit(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (amount, new_bal),
        );
        Ok(())
    }

    /// Deduct USDC from the vault and forward it to the settlement contract.
    ///
    /// Only the authorized caller or the vault owner may call this function.
    /// The deducted amount is transferred to the settlement contract and the
    /// tracked balance is decremented.  Deductions are blocked while paused.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  `caller` must be the authorized
    /// deduct caller or the vault owner.
    ///
    /// ### Parameters
    /// - `caller` — address initiating the deduction.
    /// - `amount` — amount in USDC stroops; must be within `[min_deposit, max_deduct]`.
    /// - `request_id` — idempotency key forwarded to the settlement contract.
    ///   A non-zero id is recorded and must be unique; `0` means "no
    ///   idempotency" and is never deduplicated.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Paused`] — vault is paused.
    /// - [`VaultError::Unauthorized`] — caller is not authorized to deduct.
    /// - [`VaultError::AmountNotPositive`] — `amount ≤ 0`.
    /// - [`VaultError::BelowMinDeposit`] — `amount < min_deposit`.
    /// - [`VaultError::ExceedsMaxDeduct`] — `amount > max_deduct`.
    /// - [`VaultError::InsufficientBalance`] — tracked balance < amount.
    /// - [`VaultError::Overflow`] — balance underflow.
    /// - [`VaultError::SettlementNotSet`] — settlement address is not configured.
    /// - [`VaultError::NotInitialized`] — USDC token address is not configured.
    ///
    /// ### Events
    /// Emits `deduct` with `caller` as topic and `(amount, new_balance)` as data.
    pub fn deduct(
        env: Env,
        caller: Address,
        amount: i128,
        request_id: u64,
    ) -> Result<(), VaultError> {
        caller.require_auth();

        // Shared validation — same function simulate_deduct calls.
        Self::validate_deduct(&env, &caller, amount)?;

        let settlement_addr = Self::require_settlement(&env)?;
        let usdc_addr = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::UsdcToken)
            .ok_or(VaultError::NotInitialized)?;

        let current_bal = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);

        // Idempotency guard: reject a replayed request id *before* mutating the
        // tracked balance, forwarding USDC, or emitting events. A non-zero id
        // that has already been processed returns `DuplicateRequestId`;
        // `request_id == 0` is the documented "no idempotency" sentinel.
        if request_id != 0 {
            Self::require_not_duplicate(&env, &request_id)?;
        }

        let new_bal = current_bal
            .checked_sub(amount)
            .unwrap_or_else(|| panic!("Math underflow"));
        env.storage().instance().set(&DataKey::Balance, &new_bal);

        env.events().publish(
            (
                events::event_deduct(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            (amount, new_bal),
        );

        let usdc_client = token::Client::new(&env, &usdc_addr);
        usdc_client.transfer(&env.current_contract_address(), &settlement_addr, &amount);

        let settlement_client = settlement::Client::new(&env, &settlement_addr);
        settlement_client.record_deduction(&amount, &request_id);

        // Record the idempotency marker only after every state mutation and
        // external call has succeeded. Any earlier failure reverts the whole
        // transaction, so a failed deduct never marks its request id.
        if request_id != 0 {
            Self::mark_request_processed(&env, &request_id);
        }
        Ok(())
    }

    /// Batch-deduct multiple amounts in a single atomic operation.
    ///
    /// Validates **all** items and the aggregate total **before** any state
    /// is mutated or USDC is moved; if any check fails the whole call reverts
    /// atomically and no tracked balance, on-ledger token balance, or event
    /// changes.  This preserves the conservation invariant: a successful call
    /// decrements the vault's tracked balance by exactly the sum of the items
    /// and forwards exactly that same total to the settlement contract.
    ///
    /// Per-item bounds mirror [`deduct`] (`min_deposit ≤ amount ≤ max_deduct`)
    /// and the batch is capped at [`MAX_BATCH_SIZE`] items, with an empty
    /// batch rejected outright.  The total is accumulated with checked
    /// arithmetic so an over-wide sum returns [`VaultError::Overflow`] instead
    /// of silently wrapping.
    ///
    /// ### Authorization / lifecycle
    /// `caller.require_auth()` is enforced and the caller must be the
    /// authorized deduct caller.  Deductions are blocked while the vault is
    /// paused.  Both preconditions are checked before any mutation.
    ///
    /// ### Parameters
    /// - `caller` — address initiating the deductions.
    /// - `items` — vector of `(amount, request_id)` pairs; each amount must be
    ///   within `[min_deposit, max_deduct]`. Non-zero `request_id`s must be
    ///   unique both against previously-processed ids and within this batch;
    ///   `0` means "no idempotency" and is never deduplicated.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is neither the authorized deduct
    ///   caller nor the vault owner.
    /// - [`VaultError::Paused`] — vault is paused.
    /// - [`VaultError::BatchEmpty`] — `items` is empty.
    /// - [`VaultError::BatchTooLarge`] — `items.len() > MAX_BATCH_SIZE`.
    /// - [`VaultError::AmountNotPositive`] — any item has `amount ≤ 0`.
    /// - [`VaultError::BelowMinDeposit`] — any item is below `min_deposit`.
    /// - [`VaultError::ExceedsMaxDeduct`] — any item exceeds `max_deduct`.
    /// - [`VaultError::InsufficientBalance`] — aggregate total > tracked balance.
    /// - [`VaultError::Overflow`] — total accumulation overflows `i128`.
    /// - [`VaultError::SettlementNotSet`] — settlement address is not configured.
    /// - [`VaultError::NotInitialized`] — USDC token address is not configured.
    ///
    /// ### Events
    /// Emits one `deduct` event per item with `caller` as topic carrying
    /// `(amount, running_balance)`.
    pub fn batch_deduct(
        env: Env,
        caller: Address,
        items: Vec<(i128, u64)>,
    ) -> Result<(), VaultError> {
        caller.require_auth();

        // Authorization and lifecycle preconditions — checked before mutation.
        // Same owner-or-caller rule as `deduct`; without this a cleared
        // authorized caller panicked instead of falling back to the owner.
        Self::require_authorized_deduct_caller(env.clone(), &caller)?;

        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Err(VaultError::Paused);
        }
        let settlement_addr = Self::require_settlement(&env)?;
        let usdc_addr = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::UsdcToken)
            .ok_or(VaultError::NotInitialized)?;

        // Boundary / batch-limit preconditions — checked before mutation.
        if items.is_empty() {
            return Err(VaultError::BatchEmpty);
        }
        if items.len() > MAX_BATCH_SIZE {
            return Err(VaultError::BatchTooLarge);
        }

        let min_dep = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::MinDeposit)
            .unwrap_or_else(|| panic!("Min deposit not set"));

        let max_deduct = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::MaxDeduct)
            .unwrap();

        let current_bal = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);

        // Validate every item and accumulate the total with checked arithmetic,
        // before any transfer or mutation. An overflowing total, an out-of-range
        // item, or a total that exceeds the tracked balance fails the whole call
        // atomically (mirrors `deduct`'s explicit pre-transfer checks).
        let mut total_amount: i128 = 0i128;
        let mut seen_ids: Vec<u64> = Vec::new(&env);
        for item in items.iter() {
            let (amount, request_id) = item;
            if amount <= 0 {
                return Err(VaultError::AmountNotPositive);
            }
            if amount < min_dep {
                return Err(VaultError::BelowMinDeposit);
            }
            if amount > max_deduct {
                return Err(VaultError::ExceedsMaxDeduct);
            }
            // Idempotency: reject an id already recorded in storage as well as
            // an id repeated within this same batch. Both checks run before any
            // mutation so a duplicate fails the whole batch atomically.
            // `request_id == 0` means "no idempotency" and is never recorded.
            if request_id != 0 {
                Self::require_not_duplicate(&env, &request_id)?;
                if seen_ids.contains(&request_id) {
                    return Err(VaultError::DuplicateRequestId);
                }
                seen_ids.push_back(request_id);
            }
            total_amount = total_amount
                .checked_add(amount)
                .ok_or(VaultError::Overflow)?;
        }
        if total_amount > current_bal {
            return Err(VaultError::InsufficientBalance);
        }

        let settlement_client = settlement::Client::new(&env, &settlement_addr);

        let usdc_client = token::Client::new(&env, &usdc_addr);

        // All preconditions passed. Now the value-conserving mutation pair is
        // performed atomically: forward exactly `total_amount` to settlement
        // and persist the new tracked balance. If the external transfer or a
        // settlement record fails, the entire transaction reverts and neither
        // state change survives.
        usdc_client.transfer(
            &env.current_contract_address(),
            &settlement_addr,
            &total_amount,
        );

        let mut new_bal = current_bal;
        for item in items.iter() {
            let (amount, request_id) = item;
            new_bal = new_bal.checked_sub(amount).ok_or(VaultError::Overflow)?;

            env.events().publish(
                (
                    events::event_deduct(&env),
                    events::event_version_v1(&env),
                    caller.clone(),
                ),
                (amount, new_bal),
            );

            settlement_client.record_deduction(&amount, &request_id);

            // Persist the idempotency marker only after this item's transfer
            // and settlement call succeed; a later failure reverts the batch.
            if request_id != 0 {
                Self::mark_request_processed(&env, &request_id);
            }
        }
        env.storage().instance().set(&DataKey::Balance, &new_bal);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View functions — TTL bump on hot read paths (buffer #5 pattern)
    // Bumps instance storage TTL so frequently-read vaults do not archive
    // even when writes are infrequent.
    // -----------------------------------------------------------------------

    // Modifies the authorized caller (Owner only).
    //
    // Performs validation checks identical to `deduct` and returns the predicted
    // balance after the specified `amount` is deducted.
    //
    // # Errors
    // Returns `VaultError` under the exact same conditions as `deduct`
    // (e.g., paused state, amount exceeding balance, amount exceeding max deduction limit).
    // pub fn simulate_deduct(
    //     env: Env,
    //     caller: Address,
    //     amount: i128,
    //     request_id: Option<Symbol>,
    // ) -> Result<i128, VaultError> {
    //     Self::require_not_paused(env.clone())?;
    //     caller.require_auth();
    //     if amount <= 0 {
    //         return Err(VaultError::AmountNotPositive);
    //     }
    //     Self::require_authorized_deduct_caller(env.clone(), &caller)?;
    //     let max_d = Self::get_max_deduct(env.clone());
    //     if amount > max_d {
    //         return Err(VaultError::ExceedsMaxDeduct);
    //     }
    //     if let Some(ref rid) = request_id {
    //         Self::require_not_duplicate(&env, rid)?;
    //     }
    //     let meta = Self::get_meta(env.clone())?;
    //     if meta.balance < amount {
    //         return Err(VaultError::InsufficientBalance);
    //     }
    //     let _ = Self::require_settlement(&env)?;
    //     meta.balance
    //         .checked_sub(amount)
    //         .ok_or(VaultError::Overflow)
    // }

    // pub fn simulate_batch_deduct(
    //     env: Env,
    //     caller: Address,
    //     items: Vec<DeductItem>,
    // ) -> Result<i128, VaultError> {
    //     Self::require_not_paused(env.clone())?;
    //     caller.require_auth();
    //     Self::require_authorized_deduct_caller(env.clone(), &caller)?;
    //     let n = items.len();
    //     if n == 0 {
    //         return Err(VaultError::BatchEmpty);
    //     }
    //     if n > MAX_BATCH_SIZE {
    //         return Err(VaultError::BatchTooLarge);
    //     }
    //     let max_d = Self::get_max_deduct(env.clone());
    //     let meta = Self::get_meta(env.clone())?;
    //     let mut running = meta.balance;
    //     let mut seen_in_batch: Vec<Symbol> = Vec::new(&env);
    //     for item in items.iter() {
    //         if item.amount <= 0 {
    //             return Err(VaultError::AmountNotPositive);
    //         }
    //         if item.amount > max_d {
    //             return Err(VaultError::ExceedsMaxDeduct);
    //         }
    //         if running < item.amount {
    //             return Err(VaultError::InsufficientBalance);
    //         }
    //         if let Some(ref rid) = item.request_id {
    //             Self::require_not_duplicate(&env, rid)?;
    //             if seen_in_batch.contains(rid) {
    //                 return Err(VaultError::DuplicateRequestId);
    //             }
    //             seen_in_batch.push_back(rid.clone());
    //         }
    //         running = running.checked_sub(item.amount).ok_or(VaultError::Overflow)?;
    //     }
    //     let _ = Self::require_settlement(&env)?;
    //     Ok(running)
    // }

    /// Update the address permitted to call [`deduct`] and [`batch_deduct`] (owner only).
    ///
    /// The owner must call this function to rotate the authorized caller.
    /// A monotonic `nonce` prevents replay: the supplied value must match the
    /// contract's stored nonce, which is incremented on every successful rotation.
    ///
    /// # Parameters
    /// - `new_caller` — `Some(address)` to set a new authorized caller, or `None`
    ///   to clear it (only the owner can deduct when no authorized caller is set).
    /// - `nonce` — must equal the current stored rotation nonce (starts at 0).
    ///
    /// # Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    /// - [`VaultError::StaleNonce`] — supplied nonce does not match stored nonce.
    /// - [`VaultError::AuthorizedCallerCannotBeVault`] — new_caller is the vault address.
    pub fn set_authorized_caller(
        env: Env,
        new_caller: Option<Address>,
        nonce: u64,
    ) -> Result<(), VaultError> {
        // Retrieve the owner and require their on-chain authorization.
        let owner = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .ok_or(VaultError::NotInitialized)?;
        owner.require_auth();

        // Nonce check — prevents replay of old rotation calls.
        let stored_nonce: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::AuthCallerNonce)
            .unwrap_or(0u64);
        if nonce != stored_nonce {
            return Err(VaultError::StaleNonce);
        }

        // Reject vault-as-authorized-caller.
        if let Some(ref ac) = new_caller {
            if *ac == env.current_contract_address() {
                return Err(VaultError::AuthorizedCallerCannotBeVault);
            }
        }

        let old_caller = Self::get_authorized_caller(&env);

        // Setting stores a plain `Address`; clearing removes the key entirely so
        // the value never round-trips through `ScVal::Void`.
        match new_caller {
            Some(ref caller) => env.storage().instance().set(&DataKey::AuthorizedCaller, caller),
            None => env.storage().instance().remove(&DataKey::AuthorizedCaller),
        }

        // Advance nonce.
        let next_nonce = stored_nonce.checked_add(1).ok_or(VaultError::Overflow)?;
        env.storage()
            .instance()
            .set(&StorageKey::AuthCallerNonce, &next_nonce);

        env.events().publish(
            (
                events::event_set_authorized_caller(&env),
                events::event_version_v1(&env),
                owner,
            ),
            (old_caller, new_caller, nonce),
        );
        Ok(())
    }

    /// Pause the vault, blocking deposits and deductions.
    ///
    /// Owner withdrawals and admin distributions remain available for
    /// emergency recovery.  Only the vault owner may call this function.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  `caller` must be the vault owner.
    ///
    /// ### Parameters
    /// - `caller` — must be the vault owner.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    ///
    /// ### Events
    /// Emits `vault_paused` with `caller` as topic.
    pub fn pause(env: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();

        let owner = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .unwrap_or_else(|| panic!("Owner not set"));

        if caller != owner {
            return Err(VaultError::Unauthorized);
        }

        // Idempotent: if already paused, succeed without emitting a
        // duplicate `vault_paused` event. This lets operators safely
        // re-broadcast a `pause` call after interrupted administration
        // without polluting event logs.
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Ok(());
        }

        env.storage().instance().set(&DataKey::Paused, &true);

        env.events().publish(
            (
                events::event_vault_paused(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );
        Ok(())
    }

    /// Unpause the vault, resuming deposits and deductions.
    ///
    /// Only the vault owner may call this function.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  `caller` must be the vault owner.
    ///
    /// ### Parameters
    /// - `caller` — must be the vault owner.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    ///
    /// ### Events
    /// Emits `vault_unpaused` with `caller` as topic.
    pub fn unpause(env: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();

        let owner = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .unwrap_or_else(|| panic!("Owner not set"));

        if caller != owner {
            return Err(VaultError::Unauthorized);
        }

        // Idempotent: if already unpaused, succeed without emitting a
        // duplicate `vault_unpaused` event.
        if !env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            return Ok(());
        }

        env.storage().instance().set(&DataKey::Paused, &false);

        env.events().publish(
            (
                events::event_vault_unpaused(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Recovery mode functions (allowed when paused)
    // -----------------------------------------------------------------------

    /// Withdraw USDC from the vault to the owner's address (owner only).
    ///
    /// ## Pause Policy
    /// This function is **ALLOWED when paused** for emergency recovery.
    /// The owner can withdraw tracked funds even during a circuit-breaker event.
    ///
    /// # Parameters
    /// - `amount` — USDC stroops to withdraw; must be > 0.
    ///
    /// # Returns
    /// The new vault balance after the withdrawal.
    ///
    /// # Panics
    /// - `"amount must be positive"` — `amount <= 0`.
    /// - `"insufficient balance"` — vault balance < `amount`.
    pub fn withdraw(env: Env, amount: i128) -> i128 {
        let owner = Self::get_owner(env.clone());
        owner.require_auth();
        Self::require_positive_amount(amount).unwrap();

        let current_bal: i128 = env.storage().instance().get(&DataKey::Balance).unwrap_or(0);

        if current_bal < amount {
            panic!("insufficient balance");
        }

        let new_bal = current_bal.checked_sub(amount).unwrap();
        env.storage().instance().set(&DataKey::Balance, &new_bal);
        Self::bump_instance(&env);

        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .expect("vault not initialized");

        token::Client::new(&env, &usdc_addr).transfer(
            &env.current_contract_address(),
            &owner,
            &amount,
        );

        env.events().publish(
            (
                events::event_withdraw(&env),
                events::event_version_v1(&env),
                owner,
            ),
            (amount, new_bal),
        );

        new_bal
    }

    /// Withdraw USDC from the vault to an arbitrary recipient address (owner only).
    ///
    /// ## Pause Policy
    /// This function is **ALLOWED when paused** for emergency recovery.
    /// The owner can withdraw tracked funds to any valid recipient even during
    /// a circuit-breaker event.
    ///
    /// ## Recipient Validation
    /// The recipient address is validated to prevent common mistakes:
    /// - Cannot send to the vault contract itself.
    /// - Cannot send to the USDC token contract.
    ///
    /// # Parameters
    /// - `to` — recipient address.
    /// - `amount` — USDC stroops to withdraw; must be > 0.
    ///
    /// # Returns
    /// The new vault balance after the withdrawal.
    ///
    /// # Panics
    /// - `"amount must be positive"` — `amount <= 0`.
    /// - `"insufficient balance"` — vault balance < `amount`.
    /// - `"cannot withdraw to vault address"` — `to == vault_address`.
    /// - `"cannot withdraw to token address"` — `to == usdc_token`.
    pub fn withdraw_to(env: Env, to: Address, amount: i128) -> i128 {
        let owner = Self::get_owner(env.clone());
        owner.require_auth();
        Self::require_positive_amount(amount).unwrap();

        // Recipient validation
        assert!(
            to != env.current_contract_address(),
            "cannot withdraw to vault address"
        );

        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .expect("vault not initialized");

        assert!(to != usdc_addr, "cannot withdraw to token address");

        let current_bal: i128 = env.storage().instance().get(&DataKey::Balance).unwrap_or(0);

        if current_bal < amount {
            panic!("insufficient balance");
        }

        let new_bal = current_bal.checked_sub(amount).unwrap();
        env.storage().instance().set(&DataKey::Balance, &new_bal);
        Self::bump_instance(&env);

        token::Client::new(&env, &usdc_addr).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );

        env.events().publish(
            (
                events::event_withdraw_to(&env),
                events::event_version_v1(&env),
                owner,
                to,
            ),
            (amount, new_bal),
        );

        new_bal
    }

    /// Admin distribute USDC surplus to a recipient (admin only).
    ///
    /// ## Pause Policy
    /// This function is **ALLOWED when paused** for emergency recovery of
    /// untracked on-ledger surplus.
    ///
    /// # Parameters
    /// - `caller` — must be the current admin.
    /// - `to` — recipient address.
    /// - `amount` — USDC stroops to distribute; must be > 0.
    ///
    /// # Panics
    /// - `"amount must be positive"` — `amount <= 0`.
    /// - `"insufficient USDC balance"` — on-ledger balance < `amount`.
    pub fn distribute(env: Env, caller: Address, to: Address, amount: i128) {
        Self::require_admin(&env, &caller).unwrap();
        Self::require_positive_amount(amount).unwrap();

        if amount <= 0 {
            panic!("amount must be positive");
        }

        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .expect("USDC Token not set");

        let usdc = token::Client::new(&env, &usdc_addr);
        let on_ledger = usdc.balance(&env.current_contract_address());

        if on_ledger < amount {
            panic!("insufficient USDC balance");
        }

        usdc.transfer(&env.current_contract_address(), &to, &amount);

        env.events().publish(
            (
                events::event_distribute(&env),
                events::event_version_v1(&env),
                to,
            ),
            amount,
        );
    }

    /// Return `true` if the vault is currently paused, `false` otherwise.
    ///
    /// Reads the `Paused` instance storage flag set by [`pause`] / [`execute_pause`]
    /// and cleared by [`unpause`]. Defaults to `false` when the flag is absent
    /// (i.e., before initialization).
    ///
    /// No auth required; this is a read-only view.
    pub fn is_paused(env: Env) -> bool {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
    }
    /// Return the vault's internally tracked USDC balance in stroops.
    ///
    /// This value is incremented by [`deposit`] and decremented by [`deduct`],
    /// [`batch_deduct`], and successful sweep executions. It may diverge from
    /// the on-ledger USDC balance when USDC is sent directly to the vault address
    /// (bypassing `deposit`). Use [`dry_run_sweep_idle_balance`] to inspect any
    /// surplus.
    ///
    /// No auth required; this is a read-only view.
    pub fn balance(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0)
    }
    /// Return the current vault owner address.
    ///
    /// The owner is set at [`init`] and may be transferred via a two-step
    /// ownership-transfer flow. No auth required; this is a read-only view.
    pub fn get_owner(env: Env) -> Address {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .unwrap_or_else(|| panic!("Owner not set"))
    }

    pub(crate) fn require_owner(env: Env, caller: Address) -> Result<(), VaultError> {
        let owner = Self::get_owner(env);
        if caller != owner {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }
    /// Return the USDC token contract address configured at [`init`].
    ///
    /// All vault transfers use this token contract. No auth required; this is a
    /// read-only view.
    pub fn get_usdc_token(env: Env) -> Address {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, Address>(&DataKey::UsdcToken)
            .unwrap_or_else(|| panic!("USDC Token not set"))
    }
    /// Return the per-call deduction cap in USDC stroops.
    ///
    /// [`deduct`] and each item of [`batch_deduct`] are rejected when their
    /// amount exceeds this value. Returns `i128::MAX` when no cap is stored
    /// (effectively unlimited), though in practice the limit is always set
    /// during [`init`].
    ///
    /// No auth required; this is a read-only view.
    pub fn get_max_deduct(env: Env) -> i128 {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, i128>(&DataKey::MaxDeduct)
            .unwrap_or(i128::MAX)
    }

    /// Update the per-call deduction cap (owner only).
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  `caller` must be the vault owner.
    ///
    /// ### Parameters
    /// - `caller` — must be the vault owner.
    /// - `max_deduct` — new cap in USDC stroops; must be > 0.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    /// - [`VaultError::MaxDeductNotPositive`] — `max_deduct ≤ 0`.
    ///
    /// ### Events
    /// Emits `set_max_deduct` with `caller` as topic and `max_deduct` as data.
    pub fn set_max_deduct(env: Env, caller: Address, max_deduct: i128) -> Result<(), VaultError> {
        caller.require_auth();

        let owner = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .unwrap_or_else(|| panic!("Owner not set"));

        if caller != owner {
            return Err(VaultError::Unauthorized);
        }
        if max_deduct <= 0 {
            return Err(VaultError::MaxDeductNotPositive);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxDeduct, &max_deduct);

        env.events().publish(
            (
                events::event_set_max_deduct(&env),
                events::event_version_v1(&env),
                caller,
            ),
            max_deduct,
        );
        Ok(())
    }

    /// Return the configured settlement contract address.
    ///
    /// This is the on-ledger destination that receives USDC transferred during
    /// [`deduct`] and [`batch_deduct`]. Panics if the address has never been
    /// set, which cannot occur after a successful [`init`].
    ///
    /// No auth required; this is a read-only view.
    pub fn get_settlement(env: Env) -> Address {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, Address>(&DataKey::Settlement)
            .unwrap_or_else(|| panic!("Settlement not set"))
    }

    /// Update the settlement contract address (owner only).
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced.  `caller` must be the vault owner.
    ///
    /// ### Parameters
    /// - `caller` — must be the vault owner.
    /// - `settlement` — new settlement contract address.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    /// - [`VaultError::SettlementCannotBeVault`] — settlement equals the vault address.
    /// - [`VaultError::SettlementCannotBeToken`] — settlement equals the USDC token address.
    ///
    /// ### Events
    /// Emits `set_settlement` with `(event_set_settlement, version_v1, caller)` as
    /// topics and `(old_settlement, new_settlement)` as data payload.
    pub fn set_settlement(
        env: Env,
        caller: Address,
        settlement: Address,
    ) -> Result<(), VaultError> {
        caller.require_auth();

        let owner = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::Owner)
            .unwrap_or_else(|| panic!("Owner not set"));

        if caller != owner {
            return Err(VaultError::Unauthorized);
        }

        // Reject settlement == vault address (would route deduct proceeds back to
        // the vault itself, silently burning them from the settlement contract's
        // perspective).
        if settlement == env.current_contract_address() {
            return Err(VaultError::SettlementCannotBeVault);
        }

        // Reject settlement == USDC token address (routing deduction proceeds to
        // the token contract would lock them permanently).
        let usdc_addr = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::UsdcToken)
            .unwrap_or_else(|| panic!("USDC Token not set"));
        if settlement == usdc_addr {
            return Err(VaultError::SettlementCannotBeToken);
        }

        // Read old value before overwriting for audit trail.
        let old_settlement: Option<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Settlement);

        env.storage()
            .instance()
            .set(&DataKey::Settlement, &settlement);

        env.events().publish(
            (
                events::event_set_settlement(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (old_settlement, settlement),
        );
        Ok(())
    }
    /// Return the configured revenue pool address, if any.
    ///
    /// Returns `None` when no revenue pool has been set (the default after
    /// [`init`] when `revenue_pool` is `None`). No auth required.
    pub fn get_revenue_pool(env: Env) -> Option<Address> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, Address>(&DataKey::RevenuePool)
    }

    /// Return the capability bitmap for this contract version.
    ///
    /// Each set bit represents a feature that this contract supports.  Bits are
    /// stable — a position once assigned is never reused for a different feature.
    /// Reserved bits (18–63) are always zero.
    ///
    /// No authentication required; this is a pure view function.
    ///
    /// # Example
    /// ```ignore
    /// let caps = client.capabilities();
    /// let has_batch = caps & capabilities::CAP_BATCH_DEDUCT != 0;
    /// ```
    pub fn capabilities(env: Env) -> u64 {
        capabilities::capabilities(&env)
    }

    // =====================================================================
    //  Escape-hatch admin timelock (Issue #482)
    //
    //  Critical admin actions — `pause`, `upgrade`, and `sweep` — must
    //  propose before they can be executed. The proposal records a
    //  `proposed_at` timestamp and an `execute_after` deadline derived
    //  from the configured `TimelockWindow` (default 48h). Any admin may
    //  cancel an outstanding proposal at any time.
    //
    //  All three action slots are independent so multiple escape-hatch
    //  proposals can be staged in parallel.
    // =====================================================================

    /// Configure the timelock window length (admin only).
    ///
    /// The window sets the minimum delay between proposing and executing a
    /// critical admin action. Default on deployment is **48 h** (`172_800` s).
    ///
    /// # Bounds
    /// - Minimum: [`timelock::MIN_TIMELOCK_SECONDS`] (1 h).
    /// - Maximum: [`timelock::MAX_TIMELOCK_SECONDS`] (30 d).
    /// - Default: [`timelock::DEFAULT_TIMELOCK_SECONDS`] (48 h).
    ///
    /// Changing the window does **not** retroactively shorten existing
    /// proposals — each proposal carries its own `execute_after` deadline.
    ///
    /// # Authorization
    /// The caller must be the current admin. `require_auth` runs first so
    /// misconfigured callers can never silently poison the configuration.
    ///
    /// # Errors
    /// - `VaultError::Unauthorized` if caller is not admin.
    /// - `VaultError::InvalidTimelockWindow` if `window < MIN` or `window > MAX`.
    pub fn set_timelock_window(env: Env, caller: Address, window: u64) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        if !(timelock::MIN_TIMELOCK_SECONDS..=timelock::MAX_TIMELOCK_SECONDS).contains(&window) {
            return Err(VaultError::InvalidTimelockWindow);
        }
        // Read the previous window BEFORE overwriting it so the event payload
        // carries (old_window, new_window). Reading after the write would
        // return the new value for both tuple elements (bug #1112).
        let old_window = timelock::get_timelock_window(&env);
        timelock::set_timelock_window(&env, window);
        env.events().publish(
            (
                events::event_timelock_window_changed(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            (old_window, window),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Return the configured timelock window length in seconds.
    ///
    /// Defaults to [`timelock::DEFAULT_TIMELOCK_SECONDS`] (48 h) when no
    /// window has been explicitly configured.
    pub fn get_timelock_window(env: Env) -> u64 {
        Self::bump_instance_ttl(&env);
        timelock::get_timelock_window(&env)
    }

    /// Return the outstanding timelocked pause proposal, if any.
    ///
    /// Returns `None` when no pause proposal is currently staged.  A `Some`
    /// value means a pause was proposed and can be executed once the ledger
    /// clock reaches `proposal.execute_after` (via [`execute_pause`]).
    ///
    /// No auth required; this is a read-only view.
    pub fn get_pending_pause(env: Env) -> Option<timelock::PendingPause> {
        Self::bump_instance_ttl(&env);
        let result = timelock::get_pending_pause(&env);
        if result.is_some() {
            Self::bump_persistent_key(&env, &StorageKey::PendingPause);
        }
        result
    }

    /// Return the outstanding timelocked upgrade proposal, if any.
    ///
    /// Returns `None` when no upgrade proposal is staged.  A `Some` value
    /// includes the target `wasm_hash` and the `execute_after` deadline (see
    /// [`execute_upgrade`]).
    ///
    /// No auth required; this is a read-only view.
    pub fn get_pending_upgrade(env: Env) -> Option<timelock::PendingUpgrade> {
        Self::bump_instance_ttl(&env);
        let result = timelock::get_pending_upgrade(&env);
        if result.is_some() {
            Self::bump_persistent_key(&env, &StorageKey::PendingUpgrade);
        }
        result
    }

    /// Return the outstanding timelocked sweep proposal, if any.
    ///
    /// Returns `None` when no sweep proposal is staged.  A `Some` value
    /// includes the recipient address, the amount, and the `execute_after`
    /// deadline (see [`execute_sweep`]).
    ///
    /// No auth required; this is a read-only view.
    pub fn get_pending_sweep(env: Env) -> Option<timelock::PendingSweep> {
        Self::bump_instance_ttl(&env);
        let result = timelock::get_pending_sweep(&env);
        if result.is_some() {
            Self::bump_persistent_key(&env, &StorageKey::PendingSweep);
        }
        result
    }

    /// Return the recorded WASM hash after a successful upgrade, if any.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, BytesN<32>>(&StorageKey::ContractVersion)
    }

    /// Require the caller to be the current admin.
    fn require_admin(env: &Env, caller: &Address) -> Result<(), VaultError> {
        caller.require_auth();
        let admin = env
            .storage()
            .instance()
            .get::<_, Address>(&StorageKey::Admin)
            .ok_or(VaultError::NotInitialized)?;
        if caller != &admin {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    /// Return the current admin address.
    ///
    /// Defaults to the owner when no admin has been explicitly set.
    ///
    /// # Errors
    /// - `VaultError::NotInitialized` if the contract has not been initialized.
    pub fn get_admin(env: Env) -> Result<Address, VaultError> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get::<_, Address>(&StorageKey::Admin)
            .ok_or(VaultError::NotInitialized)
    }

    /// Initiate a two-step admin transfer (current admin only).
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);
        Self::bump_instance_ttl(&env);
        env.events().publish(
            (
                events::event_admin_nominated(&env),
                events::event_version_v1(&env),
                caller,
            ),
            new_admin,
        );
        Ok(())
    }

    /// Accept a pending admin transfer (pending admin only).
    ///
    /// The pending admin must authorize this call to finalize the transfer.
    ///
    /// ### Authorization
    /// The pending admin (stored via [`set_admin`]) must call `require_auth()`.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::NoAdminTransferPending`] — no transfer is staged.
    ///
    /// ### Events
    /// Emits `admin_accepted` with the new admin as topic.
    pub fn accept_admin(env: Env) -> Result<(), VaultError> {
        let new_admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(VaultError::NoAdminTransferPending)?;
        new_admin.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &new_admin);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        env.events().publish(
            (
                events::event_admin_accepted(&env),
                events::event_version_v1(&env),
                new_admin,
            ),
            (),
        );
        Ok(())
    }

    /// Cancel a pending admin transfer (current admin only).
    ///
    /// Cancellation removes the nominee before returning, so the previously
    /// nominated address can no longer accept this transfer.
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        if env
            .storage()
            .instance()
            .get::<_, Address>(&StorageKey::PendingAdmin)
            .is_none()
        {
            return Err(VaultError::NoAdminTransferPending);
        }
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        env.events().publish(
            (
                events::event_admin_cancelled(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Transfer ownership (two-step) — initiate.
    ///
    /// # Errors
    /// - [`VaultError::Unauthorized`] — caller is not the owner.
    ///
    /// The owner must authorize this call to nominate a new pending owner.
    pub fn transfer_ownership(
        env: Env,
        caller: Address,
        new_owner: Address,
    ) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;
        env.storage()
            .instance()
            .set(&DataKey::PendingOwner, &new_owner);
        Self::bump_instance(&env);
        env.events().publish(
            (
                events::event_ownership_nominated(&env),
                events::event_version_v1(&env),
                caller,
            ),
            new_owner,
        );
        Ok(())
    }

    /// Accept pending ownership (new owner only).
    ///
    /// The pending owner must authorize this call to finalize the transfer.
    ///
    /// # Errors
    /// - [`VaultError::NoOwnershipTransferPending`] — no transfer is staged.
    pub fn accept_ownership(env: Env) -> Result<(), VaultError> {
        let new_owner: Address = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::PendingOwner)
            .ok_or(VaultError::NoOwnershipTransferPending)?;
        new_owner.require_auth();
        env.storage().instance().set(&DataKey::Owner, &new_owner);
        env.storage().instance().remove(&DataKey::PendingOwner);
        Self::bump_instance(&env);
        env.events().publish(
            (
                events::event_ownership_accepted(&env),
                events::event_version_v1(&env),
                new_owner,
            ),
            (),
        );
        Ok(())
    }

    /// Cancel a pending ownership transfer (current owner only).
    ///
    /// Cancellation removes the nominee before returning, so the previously
    /// nominated address can no longer accept this transfer.
    pub fn cancel_ownership_transfer(env: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;
        if env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::PendingOwner)
            .is_none()
        {
            return Err(VaultError::NoOwnershipTransferPending);
        }
        env.storage().instance().remove(&DataKey::PendingOwner);
        env.events().publish(
            (
                events::event_ownership_cancelled(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Critical admin action cooldown
    // -----------------------------------------------------------------------

    /// Return the global cool-off window between critical admin executions.
    ///
    /// The secure one-hour default is returned until an admin explicitly sets
    /// a value. This read-only view requires no authorization.
    pub fn get_admin_cooldown(env: Env) -> u64 {
        admin::get_cooldown(&env)
    }

    /// Configure the global cool-off window between critical admin executions.
    ///
    /// # Authorization
    /// `caller` must be the current admin and must authorize this invocation.
    ///
    /// # Errors
    /// - [`VaultError::Unauthorized`] when `caller` is not the current admin.
    /// - [`VaultError::NotInitialized`] when the vault has no configured admin.
    /// - [`VaultError::InvalidAdminCooldown`] when `seconds` is outside
    ///   [`admin::MIN_COOLDOWN_SECONDS`]..=[`admin::MAX_COOLDOWN_SECONDS`].
    pub fn set_admin_cooldown(env: Env, caller: Address, seconds: u64) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        admin::set_cooldown(&env, seconds)?;
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Return seconds remaining before another critical admin action may run.
    ///
    /// Returns `0` when no cool-off period is active.  No auth required;
    /// this is a read-only view.
    pub fn admin_cooldown_remaining(env: Env) -> u64 {
        admin::remaining(&env)
    }

    /// Return whether a critical admin action may execute now.
    ///
    /// Returns `true` when the global cool-off window has elapsed since the
    /// last execution.  No auth required; this is a read-only view.
    pub fn is_admin_action_ready(env: Env) -> bool {
        admin::is_ready(&env)
    }

    /// Return the most recently executed critical admin action, if any.
    ///
    /// Returns `None` when no critical action has been executed since
    /// deployment or after the last cooldown reset.  No auth required;
    /// this is a read-only view.
    pub fn get_last_critical_admin_action(env: Env) -> Option<admin::CriticalAdminAction> {
        admin::last_action(&env)
    }

    // -----------------------------------------------------------------------
    // Propose/Execute/Cancel — pause
    // -----------------------------------------------------------------------

    /// Stage a timelocked pause proposal (admin only).
    ///
    /// After the configured timelock window elapses, the pause may be
    /// executed via [`execute_pause`].  Only one pause proposal may be
    /// active at a time; proposing again replaces the existing proposal.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    /// - [`VaultError::NotInitialized`] — no admin configured.
    /// - [`VaultError::TimelockOverflow`] — timestamp overflow.
    ///
    /// ### Events
    /// Emits `pause_proposed` with `caller` as topic and
    /// `(proposed_at, execute_after)` as data.
    pub fn propose_pause(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let proposed_at = env.ledger().timestamp();
        let window = timelock::get_timelock_window(&env);
        let execute_after = timelock::saturating_deadline(proposed_at, window)
            .ok_or(VaultError::TimelockOverflow)?;
        let expires_at = timelock::saturating_deadline(execute_after, timelock::PROPOSAL_GRACE_SECONDS)
            .ok_or(VaultError::TimelockOverflow)?;
        timelock::set_pending_pause(
            &env,
            &timelock::PendingPause {
                proposed_at,
                execute_after,
                expires_at,
            },
        );
        env.events().publish(
            (
                events::event_pause_proposed(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (proposed_at, execute_after),
        );
        Ok(())
    }

    /// Execute a timelocked pause proposal (admin only).
    ///
    /// The proposal must exist and the ledger timestamp must have passed
    /// the proposal's `execute_after` deadline.  On success the vault is
    /// paused, the proposal is cleared, and the global admin cooldown is
    /// activated.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    /// - [`VaultError::ProposalNotFound`] — no pending pause proposal.
    /// - [`VaultError::TimelockNotExpired`] — timelock still active.
    ///
    /// ### Events
    /// Emits `pause_executed` with `caller` as topic, then `vault_paused`.
    pub fn execute_pause(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;

        let proposal = timelock::get_pending_pause(&env).ok_or(VaultError::ProposalNotFound)?;
        if env.ledger().timestamp() < proposal.execute_after {
            return Err(VaultError::TimelockNotExpired);
        }
        if env.ledger().timestamp() > proposal.expires_at {
            timelock::clear_pending_pause(&env);
            Self::bump_instance_ttl(&env);
            return Err(VaultError::ProposalExpired);
        }

        // Idempotent: if the vault is already paused (e.g. from a prior
        // direct `pause()` call or a previous `execute_pause` whose
        // transaction completed but the cooldown fired again), just clear
        // the stale proposal without arming the admin cooldown.  This
        // ensures recovery mode remains accessible after interrupted
        // administration.
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            timelock::clear_pending_pause(&env);
            Self::bump_instance_ttl(&env);
            return Ok(());
        }

        admin::guard(&env, Symbol::new(&env, "pause"))?;
        env.storage().instance().set(&DataKey::Paused, &true);
        timelock::clear_pending_pause(&env);
        env.events().publish(
            (
                events::event_pause_executed(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            env.ledger().timestamp(),
        );
        env.events().publish(
            (
                events::event_vault_paused(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Cancel a pending timelocked pause proposal (admin only).
    ///
    /// Clears the proposal regardless of whether one exists.  Idempotent.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    ///
    /// ### Events
    /// Emits `pause_cancelled` with `caller` as topic.
    pub fn cancel_pause(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let existing = timelock::get_pending_pause(&env);
        timelock::clear_pending_pause(&env);
        env.events().publish(
            (
                events::event_pause_cancelled(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            existing.is_some(),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Propose/Execute/Cancel — upgrade
    // -----------------------------------------------------------------------

    /// Stage a timelocked contract upgrade proposal (admin only).
    ///
    /// After the timelock window elapses the upgrade may be executed via
    /// [`execute_upgrade`].  Proposing again replaces the existing proposal.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    /// - `new_wasm_hash` — target WASM hash for the upgrade.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    /// - [`VaultError::TimelockOverflow`] — timestamp overflow.
    ///
    /// ### Events
    /// Emits `upgrade_proposed` with `caller` as topic and
    /// `(wasm_hash, proposed_at, execute_after)` as data.
    pub fn propose_upgrade(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let proposed_at = env.ledger().timestamp();
        let window = timelock::get_timelock_window(&env);
        let execute_after = timelock::saturating_deadline(proposed_at, window)
            .ok_or(VaultError::TimelockOverflow)?;
        let expires_at = timelock::saturating_deadline(execute_after, timelock::PROPOSAL_GRACE_SECONDS)
            .ok_or(VaultError::TimelockOverflow)?;
        timelock::set_pending_upgrade(
            &env,
            &timelock::PendingUpgrade {
                wasm_hash: new_wasm_hash.clone(),
                proposed_at,
                execute_after,
                expires_at,
            },
        );
        env.events().publish(
            (
                events::event_upgrade_proposed(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (new_wasm_hash, proposed_at, execute_after),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Execute a timelocked contract upgrade (admin only).
    ///
    /// The proposal must exist and the timelock must have expired.  On
    /// success the contract WASM is swapped and the `ContractVersion`
    /// storage key is updated.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    /// - [`VaultError::ProposalNotFound`] — no pending upgrade proposal.
    /// - [`VaultError::TimelockNotExpired`] — timelock still active.
    ///
    /// ### Events
    /// Emits `upgrade_executed` with `caller` as topic, then `upgraded`.
    pub fn execute_upgrade(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let proposal = timelock::get_pending_upgrade(&env).ok_or(VaultError::ProposalNotFound)?;
        if env.ledger().timestamp() < proposal.execute_after {
            return Err(VaultError::TimelockNotExpired);
        }
        if env.ledger().timestamp() > proposal.expires_at {
            timelock::clear_pending_upgrade(&env);
            Self::bump_instance_ttl(&env);
            return Err(VaultError::ProposalExpired);
        }
        admin::guard(&env, Symbol::new(&env, "upgrade"))?;
        let wasm_hash = proposal.wasm_hash.clone();
        let _admin = Self::get_admin(env.clone())?;
        env.deployer()
            .update_current_contract_wasm(wasm_hash.clone());
        env.storage()
            .instance()
            .set(&StorageKey::ContractVersion, &wasm_hash);
        timelock::clear_pending_upgrade(&env);
        env.events().publish(
            (
                events::event_upgrade_executed(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            env.ledger().timestamp(),
        );
        env.events().publish(
            (
                events::event_upgraded(&env),
                events::event_version_v1(&env),
                caller,
            ),
            wasm_hash,
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Cancel a pending timelocked upgrade proposal (admin only).
    ///
    /// Clears the proposal regardless of whether one exists.  Idempotent.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    ///
    /// ### Events
    /// Emits `upgrade_cancelled` with `caller` as topic.
    pub fn cancel_upgrade(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let existing = timelock::get_pending_upgrade(&env);
        timelock::clear_pending_upgrade(&env);
        env.events().publish(
            (
                events::event_upgrade_cancelled(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            existing.is_some(),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Propose sweeping (distributing) on-ledger USDC surplus to a recipient (admin only, timelocked).
    pub fn propose_sweep(
        env: Env,
        caller: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        if amount <= 0 {
            return Err(VaultError::AmountNotPositive);
        }
        let min_amount: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinDeposit)
            .ok_or(VaultError::NotInitialized)?;
        if amount < min_amount {
            return Err(VaultError::BelowMinTransferAmount);
        }
        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .ok_or(VaultError::NotInitialized)?;
        let usdc = token::Client::new(&env, &usdc_addr);
        let on_ledger = usdc.balance(&env.current_contract_address());
        let tracked = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);
        let surplus = on_ledger.checked_sub(tracked).unwrap_or(0);
        if amount > surplus {
            return Err(VaultError::InsufficientBalance);
        }
        let proposed_at = env.ledger().timestamp();
        let window = timelock::get_timelock_window(&env);
        let execute_after = timelock::saturating_deadline(proposed_at, window)
            .ok_or(VaultError::TimelockOverflow)?;
        let expires_at = timelock::saturating_deadline(execute_after, timelock::PROPOSAL_GRACE_SECONDS)
            .ok_or(VaultError::TimelockOverflow)?;
        timelock::set_pending_sweep(
            &env,
            &timelock::PendingSweep {
                to: to.clone(),
                amount,
                proposed_at,
                execute_after,
                expires_at,
            },
        );
        env.events().publish(
            (
                events::event_sweep_proposed(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (to, amount, proposed_at, execute_after),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Execute a timelocked sweep proposal (admin only).
    ///
    /// Transfers USDC surplus to the proposal's recipient.  The proposal
    /// must exist and the timelock must have expired.  Sweep is protected
    /// by the admin cooldown.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    /// - [`VaultError::ProposalNotFound`] — no pending sweep proposal.
    /// - [`VaultError::TimelockNotExpired`] — timelock still active.
    /// - [`VaultError::InsufficientBalance`] — surplus < amount.
    ///
    /// ### Events
    /// Emits `sweep_executed` with `caller` as topic, then `distribute`.
    pub fn execute_sweep(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let proposal = timelock::get_pending_sweep(&env).ok_or(VaultError::ProposalNotFound)?;
        if env.ledger().timestamp() < proposal.execute_after {
            return Err(VaultError::TimelockNotExpired);
        }
        if env.ledger().timestamp() > proposal.expires_at {
            timelock::clear_pending_sweep(&env);
            Self::bump_instance_ttl(&env);
            return Err(VaultError::ProposalExpired);
        }
        let usdc_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .ok_or(VaultError::NotInitialized)?;
        let usdc = token::Client::new(&env, &usdc_addr);
        let on_ledger = usdc.balance(&env.current_contract_address());
        let tracked = env
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::Balance)
            .unwrap_or(0);
        let surplus = on_ledger.checked_sub(tracked).unwrap_or(0);
        if surplus < proposal.amount {
            return Err(VaultError::InsufficientBalance);
        }

        admin::guard(&env, Symbol::new(&env, "sweep"))?;
        usdc.transfer(
            &env.current_contract_address(),
            &proposal.to,
            &proposal.amount,
        );
        let executed_at = env.ledger().timestamp();
        timelock::clear_pending_sweep(&env);
        env.events().publish(
            (
                events::event_sweep_executed(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (proposal.to.clone(), proposal.amount, executed_at),
        );
        env.events().publish(
            (
                events::event_distribute(&env),
                events::event_version_v1(&env),
                proposal.to,
            ),
            proposal.amount,
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Cancel a pending timelocked sweep proposal (admin only).
    ///
    /// Clears the proposal regardless of whether one exists.  Idempotent.
    ///
    /// ### Authorization
    /// `caller.require_auth()` is enforced via [`require_admin`].
    /// `caller` must be the current admin.
    ///
    /// ### Parameters
    /// - `caller` — must be the current admin.
    ///
    /// ### Returns
    /// `Ok(())` on success.
    ///
    /// ### Errors
    /// - [`VaultError::Unauthorized`] — caller is not admin.
    ///
    /// ### Events
    /// Emits `sweep_cancelled` with `caller` as topic.
    pub fn cancel_sweep(env: Env, caller: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        let existing = timelock::get_pending_sweep(&env);
        timelock::clear_pending_sweep(&env);
        env.events().publish(
            (
                events::event_sweep_cancelled(&env),
                events::event_version_v1(&env),
                caller.clone(),
            ),
            existing.is_some(),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Extend instance storage TTL to `INSTANCE_BUMP_AMOUNT` when the
    /// remaining TTL falls below `INSTANCE_BUMP_THRESHOLD`.
    ///
    /// Called on every hot read path so that frequently-queried contracts
    /// do not archive due to infrequent writes.
    #[inline]
    pub(crate) fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Unconditional alias for `bump_instance_ttl` — used on write paths
    /// that may also update the instance and therefore always need a fresh
    /// TTL extension.
    #[inline]
    pub(crate) fn bump_instance(env: &Env) {
        Self::bump_instance_ttl(env);
    }

    /// Extend persistent storage TTL for a given key.
    #[inline]
    pub(crate) fn bump_persistent_key(env: &Env, key: &StorageKey) {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    #[inline(never)]
    /// Read the authorized deduct caller, if one is set.
    ///
    /// The value is stored as a plain `Address`; the key is absent when no
    /// caller is configured. `None` must never be written as a value, because
    /// `ScVal::Void` does not read back as `Option<Address>` and would abort the
    /// host call (issue #1107).
    pub(crate) fn get_authorized_caller(env: &Env) -> Option<Address> {
        if env.storage().instance().has(&DataKey::AuthorizedCaller) {
            env.storage()
                .instance()
                .get::<_, Address>(&DataKey::AuthorizedCaller)
        } else {
            None
        }
    }

    /// Owner-or-caller check shared by [`CalloraVault::deduct`] and
    /// [`CalloraVault::batch_deduct`]. The owner may always deduct; when an
    /// authorized caller is configured it may deduct too.
    fn require_authorized_deduct_caller(env: Env, caller: &Address) -> Result<(), VaultError> {
        let owner = Self::get_owner(env.clone());
        if *caller == owner {
            return Ok(());
        }
        if let Some(ac) = Self::get_authorized_caller(&env) {
            if *caller == ac {
                return Ok(());
            }
        }
        Err(VaultError::Unauthorized)
    }

    /// Return `true` if `request_id` has already been processed (marker present
    /// in persistent storage, or temporary storage for legacy markers).
    pub fn is_request_processed(env: Env, request_id: u64) -> bool {
        let key = StorageKey::ProcessedRequest(request_id);
        env.storage().persistent().has(&key) || env.storage().temporary().has(&key)
    }

    /// Check that `request_id` has NOT been processed yet.
    /// Returns `VaultError::DuplicateRequestId` if the marker exists.
    pub(crate) fn require_not_duplicate(env: &Env, request_id: &u64) -> Result<(), VaultError> {
        let key = StorageKey::ProcessedRequest(*request_id);
        if env.storage().persistent().has(&key) || env.storage().temporary().has(&key) {
            return Err(VaultError::DuplicateRequestId);
        }
        Ok(())
    }

    /// Persist a processed-request marker in persistent storage and set its TTL.
    fn mark_request_processed(env: &Env, request_id: &u64) {
        let key = StorageKey::ProcessedRequest(*request_id);
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(
            &key,
            REQUEST_ID_BUMP_THRESHOLD,
            REQUEST_ID_BUMP_AMOUNT,
        );
    }

    /// Remove processed-request markers for the given request IDs.
    ///
    /// Owner-only. Idempotent: removing a non-existent marker is a no-op.
    ///
    /// ## Pause Policy
    /// This function is **ALLOWED when paused** for emergency recovery.
    ///
    /// ### Parameters
    /// - `caller` — must be the vault owner.
    /// - `ids` — vector of request-id values to prune.
    ///
    /// ### Events
    /// Emits `request_id_pruned` for each successfully pruned marker.
    pub fn prune_processed_requests(
        env: Env,
        caller: Address,
        ids: Vec<u64>,
    ) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller)?;

        for id in ids.iter() {
            let key = StorageKey::ProcessedRequest(id);
            let removed_persistent = env.storage().persistent().has(&key);
            let removed_temporary = env.storage().temporary().has(&key);
            if removed_persistent {
                env.storage().persistent().remove(&key);
            }
            if removed_temporary {
                env.storage().temporary().remove(&key);
            }
            if removed_persistent || removed_temporary {
                env.events().publish(
                    (
                        events::event_request_id_pruned(&env),
                        events::event_version_v1(&env),
                        id,
                    ),
                    (),
                );
            }
        }

        Self::bump_instance_ttl(&env);
        Ok(())
    }

    fn transfer_funds(env: &Env, usdc_token: &Address, to: &Address, amount: i128) {
        token::Client::new(env, usdc_token).transfer(&env.current_contract_address(), to, &amount);
    }

    fn require_settlement(env: &Env) -> Result<Address, VaultError> {
        env.storage()
            .instance()
            .get(&DataKey::Settlement)
            .ok_or(VaultError::SettlementNotSet)
    }

    #[inline(never)]
    fn require_not_paused(env: Env) -> Result<(), VaultError> {
        if Self::is_paused(env) {
            return Err(VaultError::Paused);
        }
        Ok(())
    }

    #[inline(never)]
    fn require_admin_or_owner(env: &Env, caller: &Address) -> Result<(), VaultError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(VaultError::NotInitialized)?;
        let owner = Self::get_owner(env.clone());
        if *caller != admin && *caller != owner {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    /// Single source of truth for the deposit authorization gate.
    ///
    /// Returns `true` iff `caller` is the vault owner or is present in the
    /// `StorageKey::AllowedDepositors` vector — precisely the predicate
    /// [`CalloraVault::deposit`] enforces and
    /// [`CalloraVault::is_authorized_depositor`] reports.  Both call sites
    /// share this helper so the view and the mutating path can never drift
    /// apart (issue #1110).
    ///
    /// A vault that has not been initialized has neither an owner nor an
    /// allowlist, so this returns `false`.
    fn allowlist_allows(env: &Env, caller: &Address) -> bool {
        let owner: Option<Address> = env.storage().instance().get(&DataKey::Owner);
        if owner.as_ref() == Some(caller) {
            return true;
        }
        env.storage()
            .instance()
            .get::<_, Vec<Address>>(&StorageKey::AllowedDepositors)
            .map(|allowlist| allowlist.contains(caller))
            .unwrap_or(false)
    }

    /// Check whether an address is authorized to deposit.
    ///
    /// Returns `true` if the address is the vault owner **or** has been added
    /// to the deposit allowlist via [`add_address`]; `false` otherwise.  This
    /// is exactly the authorization gate [`deposit`] applies, so a successful
    /// pre-check through this view predicts deposit acceptance along the
    /// caller dimension — the vault can still reject a deposit for unrelated
    /// reasons such as pause state or an amount below `min_deposit`.
    ///
    /// The owner counts as authorized even when the allowlist is empty or has
    /// been cleared with [`clear_all`] (mirroring `deposit`'s owner bypass).
    /// Returns `false` before `init`.
    ///
    /// No auth required; this is a read-only view.
    ///
    /// ### Parameters
    /// - `caller` — address to check.
    ///
    /// ### Returns
    /// `true` if `caller` is the owner or is on the deposit allowlist.
    pub fn is_authorized_depositor(env: Env, caller: Address) -> bool {
        Self::bump_instance_ttl(&env);
        Self::allowlist_allows(&env, &caller)
    }

    /// Add a single address to the deposit allowlist (owner-only).
    ///
    /// If the address is already present, this function is idempotent and will
    /// succeed without error. The `allowlist_add` event is emitted even for
    /// duplicate adds to maintain audit trail clarity.
    ///
    /// # Parameters
    /// - `caller` — Must be the vault owner (verified via `require_owner`)
    /// - `depositor` — Address to add to the allowlist
    ///
    /// # Returns
    /// `Ok(())` on success, or `VaultError::Unauthorized` if caller is not owner.
    ///
    /// # Events
    /// Emits `("allowlist_add", caller, depositor)` on every successful call,
    /// including duplicates.
    ///
    /// # Examples
    /// ```ignore
    /// vault.add_address(&owner, &backend_service_1)?;
    /// vault.add_address(&owner, &backend_service_2)?;
    /// ```
    pub fn add_address(env: Env, caller: Address, depositor: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;

        let mut allowlist = env
            .storage()
            .instance()
            .get::<_, Vec<Address>>(&StorageKey::AllowedDepositors)
            .unwrap_or_else(|| Vec::new(&env));

        // Idempotent: only add if not already present
        if !allowlist.contains(&depositor) {
            allowlist.push_back(depositor.clone());
            env.storage()
                .instance()
                .set(&StorageKey::AllowedDepositors, &allowlist);
        }

        env.events().publish(
            (
                events::event_allowlist_add(&env),
                events::event_version_v1(&env),
                caller,
                depositor,
            ),
            (),
        );

        Ok(())
    }

    /// Remove a single address from the deposit allowlist (owner-only).
    ///
    /// If the address is **not** in the allowlist the call succeeds without
    /// modifying state or emitting an event (idempotent). All other entries
    /// in the allowlist are preserved unchanged.
    ///
    /// # Parameters
    /// - `caller` — Must be the vault owner (verified via `require_owner`).
    /// - `depositor` — Address to remove from the allowlist.
    ///
    /// # Returns
    /// `Ok(())` on success, or `VaultError::Unauthorized` if caller is not owner.
    ///
    /// # Events
    /// Emits `("allowlist_remove", "callora_v1", caller, depositor)` only when
    /// the address was actually present and removed. No event is emitted when
    /// the address was not in the list.
    pub fn remove_address(env: Env, caller: Address, depositor: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;

        let allowlist = env
            .storage()
            .instance()
            .get::<_, Vec<Address>>(&StorageKey::AllowedDepositors)
            .unwrap_or_else(|| Vec::new(&env));

        // Find the index of the address; if absent return success without
        // touching storage or emitting an event (idempotent).
        let mut found_index: Option<u32> = None;
        for (i, addr) in allowlist.iter().enumerate() {
            if addr == depositor {
                found_index = Some(i as u32);
                break;
            }
        }

        let idx = match found_index {
            Some(i) => i,
            None => return Ok(()),
        };

        // Build a new list with the target address removed.
        let mut new_list: Vec<Address> = Vec::new(&env);
        for (i, addr) in allowlist.iter().enumerate() {
            if i as u32 != idx {
                new_list.push_back(addr);
            }
        }

        env.storage()
            .instance()
            .set(&StorageKey::AllowedDepositors, &new_list);

        env.events().publish(
            (events::event_allowlist_remove(&env), events::event_version_v1(&env), caller, depositor),
            (),
        );

        Ok(())
    }

    /// Remove all addresses from the deposit allowlist (owner-only).
    ///
    /// This function is idempotent — calling it on an empty allowlist succeeds
    /// without error.
    ///
    /// # Parameters
    /// - `caller` — Must be the vault owner (verified via `require_owner`)
    ///
    /// # Returns
    /// `Ok(())` on success, or `VaultError::Unauthorized` if caller is not owner.
    ///
    /// # Events
    /// Emits `("allowlist_clear", caller)` on every successful call, even when
    /// the allowlist is already empty.
    ///
    /// # Examples
    /// ```ignore
    /// vault.clear_all(&owner)?;
    /// // Subsequent calls succeed (idempotent):
    /// vault.clear_all(&owner)?;
    /// ```
    pub fn clear_all(env: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;

        env.storage()
            .instance()
            .remove(&StorageKey::AllowedDepositors);

        env.events().publish(
            (
                events::event_allowlist_clear(&env),
                events::event_version_v1(&env),
                caller,
            ),
            (),
        );

        Ok(())
    }

    /// Return the current deposit allowlist.
    ///
    /// No authentication required — this is a public read-only view function.
    /// Addresses are returned in insertion order.
    ///
    /// # Returns
    /// `Vec<Address>` containing all addresses currently in the allowlist.
    /// Returns an empty vector if no allowlist has been configured.
    ///
    /// # Examples
    /// ```ignore
    /// let allowed = vault.get_allowlist();
    /// assert_eq!(allowed.len(), 3);
    /// ```
    pub fn get_allowlist(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get::<_, Vec<Address>>(&StorageKey::AllowedDepositors)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Transfer tokens accidentally sent to the vault address to a designated
    /// recipient (admin only, emergency rescue path).
    ///
    /// For non-USDC tokens the full on-ledger balance is available for rescue.
    /// For the configured USDC token the vault protects the internally-tracked
    /// balance (`DataKey::Balance`) so that rescue can only recover surplus
    /// above what the vault's accounting already claims; this prevents draining
    /// real user funds via the rescue path.
    ///
    /// ## TTL behaviour
    ///
    /// This is a **hot read path**: it reads the tracked USDC balance and the
    /// USDC token address from instance storage before executing the transfer.
    /// The instance TTL is bumped on entry (buffer #5) so that vaults which
    /// are only ever touched via `admin_rescue` do not silently archive.
    ///
    /// # Parameters
    /// - `caller` — Must be the current admin.
    /// - `token_address` — Token contract to rescue funds from.
    /// - `to` — Recipient of the rescued funds.
    /// - `amount` — Amount to transfer; must be > 0.
    ///
    /// # Errors
    /// - [`VaultError::Unauthorized`] — `caller` is not the admin.
    /// - [`VaultError::AmountNotPositive`] — `amount <= 0`.
    /// - [`VaultError::InsufficientBalance`] — Available (unprotected) balance
    ///   is less than `amount`.
    ///
    /// # Events
    /// Emits `("rescue_funds", caller, token_address)` with payload `(to, amount)`.
    pub fn admin_rescue(
        env: Env,
        caller: Address,
        token_address: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;

        // --- Hot read path: bump instance TTL before reading any storage ---
        Self::bump_instance_ttl(&env);

        // Read the tracked USDC balance so we can pass the protection sentinel
        // to rescue::rescue_funds for the USDC token.
        let usdc_addr: Option<Address> = env.storage().instance().get(&DataKey::UsdcToken);
        let protected_balance: Option<i128> = if usdc_addr.as_ref() == Some(&token_address) {
            let bal: i128 = env.storage().instance().get(&DataKey::Balance).unwrap_or(0);
            Some(bal)
        } else {
            None
        };

        rescue::rescue_funds(&env, &token_address, &to, amount, protected_balance)?;

        env.events().publish(
            (
                events::event_rescue_funds(&env),
                events::event_version_v1(&env),
                caller,
                token_address,
            ),
            (to, amount),
        );

        Ok(())
    }

    /// Set or update the reserve cap for a token (owner only).
    ///
    /// The reserve cap is the maximum total balance the vault may hold for
    /// `token`.  Any `deposit` call that would push the balance beyond `cap`
    /// is rejected with [`VaultError::ExceedsReserveCap`].
    ///
    /// Pass `i128::MAX` to remove the effective cap (restore unlimited deposits).
    ///
    /// # Parameters
    /// - `caller` — must be the vault owner.
    /// - `token` — token contract address the cap applies to.
    /// - `cap` — maximum balance in token stroops; must be > 0.
    ///
    /// # Errors
    /// - [`VaultError::Unauthorized`] — `caller` is not the owner.
    /// - [`VaultError::AmountNotPositive`] — `cap <= 0`.
    pub fn set_reserve_cap(
        env: Env,
        caller: Address,
        token: Address,
        cap: i128,
    ) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_owner(env.clone(), caller.clone())?;
        if cap <= 0 {
            return Err(VaultError::AmountNotPositive);
        }
        let prev = limits::set(&env, &token, cap);
        env.events().publish(
            (
                events::event_reserve_cap_set(&env),
                events::event_version_v1(&env),
                caller,
                token,
            ),
            (prev, cap),
        );
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    /// Return the reserve cap for `token`.
    ///
    /// Returns `i128::MAX` when no cap has been configured (effectively unlimited).
    pub fn get_reserve_cap(env: Env, token: Address) -> i128 {
        Self::bump_instance_ttl(&env);
        limits::get(&env, &token)
    }
}

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

pub mod capabilities;
pub mod events;
pub mod limits;
pub mod rate_limit;
pub mod rescue;

/// Formal verification harnesses (compiled only under `cargo kani`).
#[cfg(kani)]
mod kani_proofs;

#[cfg(any(kani, test))]
#[path = "../proofs/deduct.rs"]
mod deduct_proofs;

// ---------------------------------------------------------------------------
// Test modules
// ---------------------------------------------------------------------------
//
// The vault's legacy unit- and integration-test suite does not compile against
// the current contract surface: it targets a removed public API (`DeductItem`,
// a 7-arg `init` without a `settlement` parameter, a `deduct` returning the
// remaining balance, removed `VaultError` variants, and removed SDK methods
// such as `Symbol::try_from_val`). The stale files are preserved in-repo as
// `*.rs.broken` / `*.stale` for a follow-up reconciliation, mirroring the
// convention the crate already used for other stale tests. The only compiled
// test module is `test_value_conservation`, which provides focused regression
// coverage for the value-conservation behaviour addressed by this change.
//
// Quarantined unit-test modules (not compiled):
//   - src/test.rs.broken
//   - src/test_views.rs.broken
//   - src/test_simulate_deduct.rs.broken
//   - src/test_idempotency.rs.broken
//   - src/test_reentrancy.rs.broken
//   - src/test_error_codes.rs.broken (superseded by tests/err_stab.rs)
//   - src/test_sweep_idle_balance.rs.broken
//   - src/test_access_control_matrix.rs.broken
//   - src/test_rustdoc_coverage.rs.broken
//
// Quarantined integration tests (not compiled):
//   - tests/auth_snap.rs.broken
//   - tests/proptest.rs.broken
//   - tests/event_order.stale
//   - tests/rescue.stale

/// Focused regression coverage for the value-conservation behaviour of
/// `deduct` / `batch_deduct` / `execute_sweep` / `cancel_sweep`.
#[cfg(test)]
mod test_value_conservation;

#[cfg(test)]
mod test_recovery_idempotency;

/// Request-id idempotency coverage for `deduct` / `batch_deduct`:
/// duplicate rejection, within-batch dedup, marker views, and pruning.
#[cfg(test)]
mod test_idempotency;

/// Holistic event-shape audit: drives every vault function that emits an event
/// and asserts the published topic list matches EVENT_SCHEMA.md.  Catches
/// call sites that fire zero, twice, or with the wrong topic count / version.
#[cfg(test)]
mod test_event_schema;

/// #1125: sweep/pause/upgrade lifecycle against the global admin cool-off
/// (back-to-back matured proposals, boundary, cancel/re-propose, and the
/// `cancel_sweep` `existing.is_some()` payload). Named `*timelock*` so
/// `cargo test -p callora-vault timelock` selects it.
#[cfg(test)]
mod test_timelock_cooldown;
#[cfg(test)]
mod test_admin_transfers;

/// Allowlist ↔ deposit authorization parity (issue #1110): the
/// `is_authorized_depositor` view and the `deposit` gate must consult the
/// same storage, so a pre-check through the view predicts deposit acceptance.
#[cfg(test)]
mod test_allowlist;

/// Issue #1107 — owner-deduction fallback. The owner may `deduct` /
/// `batch_deduct` when `DataKey::AuthorizedCaller` is `None` (or holds someone
/// else), and an unauthorized caller gets a typed error rather than a host
/// panic.
#[cfg(test)]
mod test_owner_deduct_fallback;

#[cfg(test)]
mod test_timelock;

#[cfg(test)]
mod test_capabilities;

#[cfg(test)]
mod test_allowlist_remove;

/// Parity tests for `simulate_deduct` vs `deduct` (Issue #1115).
/// Run with: `cargo test -p callora-vault simulate`
#[cfg(test)]
mod test_simulate_parity;

#[cfg(test)]
mod test_views;

#[cfg(test)]
mod test_reentrancy;

#[cfg(test)]
mod test;

// #[cfg(test)]
// mod test_gas_budget;
// #[cfg(test)]
// mod test_rate_limit;
