#![no_std]
pub mod admin;
pub mod archive;
pub mod batch;
pub mod errors;
pub mod events;

pub mod freeze;
pub mod limits;
pub mod migrate;
pub mod pagination;
pub mod price_registry;
pub mod replay_guard;
pub mod timelock;
mod types;

use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env, Symbol, Vec};

/// Maximum number of items allowed in a single `batch_receive_payment` call.
pub const MAX_BATCH_SIZE: u32 = 50;

/// Maximum number of developer balances returned per page in paginated queries.
pub const MAX_DEVELOPER_BALANCES_PAGE_SIZE: u32 = 100;

/// Number of developer addresses stored per persistent index page.
///
/// Each `StorageKey::IndexPage(n)` entry holds at most this many addresses.
/// Chosen to keep each persistent entry well within Soroban's 64 KB entry
/// limit while minimising the number of round-trips for sequential scans.
pub const INDEX_PAGE_SIZE: u32 = 50;

/// Maximum byte length of an admin broadcast message.
///
/// Keeps per-call resource consumption (event payload size, transaction size)
/// bounded and predictable for a public admin entry point.
pub const MAX_BROADCAST_MESSAGE_LEN: u32 = 1024;

pub use errors::SettlementError;
pub use migrate::{STORAGE_VERSION_V1, STORAGE_VERSION_V2};
pub use timelock::{
    PendingDeveloperMigration, PendingUpgrade, DEVELOPER_MIGRATION_TIMELOCK_SECONDS,
    UPGRADE_TIMELOCK_SECONDS,
};
pub use types::*;

#[contract]
pub struct CalloraSettlement;

#[contractimpl]
impl CalloraSettlement {
    /// Initialize the settlement contract. Can only be called once.
    ///
    /// # Arguments
    /// * `admin` - Address permitted to call admin-only entrypoints; must authorize.
    /// * `vault_address` - Vault contract address permitted to call `receive_payment`.
    ///
    /// # Panics
    /// * `"Already initialized"` - `init` called more than once.
    /// * `"invalid config: admin and vault_address must be distinct"`
    /// * `"invalid config: admin cannot be the contract itself"`
    /// * `"invalid config: vault_address cannot be the contract itself"`
    pub fn init(env: Env, admin: Address, vault_address: Address) {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            env.panic_with_error(SettlementError::AlreadyInitialized);
        }
        if admin == vault_address {
            env.panic_with_error(SettlementError::InvalidConfigDistinct);
        }
        let contract_address = env.current_contract_address();
        if admin == contract_address {
            env.panic_with_error(SettlementError::InvalidConfigAdminContract);
        }
        if vault_address == contract_address {
            env.panic_with_error(SettlementError::InvalidConfigVaultContract);
        }

        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::Vault, &vault_address);
        let pool = GlobalPool {
            total_balance: 0,
            last_updated: env.ledger().timestamp(),
        };
        inst.set(&StorageKey::GlobalPool, &pool);
        inst.set(&StorageKey::TotalReceived, &0i128);

        events::emit_initialized(&env, &admin, &vault_address, &pool);
    }

    /// Record a vault-originated deduction against the contract's cumulative
    /// received amount.
    ///
    /// This entrypoint is intended for accounting-only updates and does not
    /// credit any developer or pool balance. The vault must authorize the call.
    ///
    /// # Validation and events
    /// Amounts must be positive and each request ID may be recorded only once.
    /// Request markers use persistent storage with the standard persistent TTL.
    /// Emits `deduction_recorded` with the amount and request ID on success.
    ///
    /// # Arithmetic safety
    /// The cumulative total is incremented via `checked_add`. An overflow
    /// panics with [`SettlementError::PoolOverflow`] rather than wrapping
    /// silently.
    pub fn record_deduction(env: Env, amount: i128, request_id: u64) {
        let vault = Self::get_vault(env.clone()).unwrap();
        vault.require_auth();
        if amount <= 0 {
            env.panic_with_error(SettlementError::AmountNotPositive);
        }
        let request_key = StorageKey::DeductionRequest(request_id);
        if env.storage().persistent().has(&request_key) {
            env.panic_with_error(SettlementError::DuplicateRequestId);
        }
        let total = env
            .storage()
            .instance()
            .get::<_, i128>(&StorageKey::TotalReceived)
            .unwrap_or(0);
        let new_total = total
            .checked_add(amount)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::PoolOverflow));
        env.storage()
            .instance()
            .set(&StorageKey::TotalReceived, &new_total);
        env.storage().persistent().set(&request_key, &true);
        env.storage().persistent().extend_ttl(
            &request_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
        events::emit_deduction_recorded(&env, DeductionRecordedEvent { amount, request_id });
    }

    /// Receive payment from vault and credit to pool or developer balance.
    ///
    /// # Arguments
    /// * `caller` - Must be authorized vault address or admin
    /// * `amount` - Payment amount in token micro-units; must be > 0
    /// * `to_pool` - If true, credit global pool; if false, credit a specific developer
    /// * `developer` - Required when `to_pool=false`; ignored when `to_pool=true`
    /// * `token` - The token contract address for this payment
    ///
    /// # Access Control
    /// Only the registered vault address or admin can call this function.
    ///
    /// # Events
    /// Always emits `payment_received`. Also emits `balance_credited` and `deposit`
    /// when `to_pool=false`.
    ///
    /// # Arithmetic Safety
    /// Credits use checked arithmetic:
    /// - Pool credits panic with `PoolOverflow` on `i128` overflow.
    /// - Developer credits panic with `DeveloperOverflow` on `i128` overflow.
    pub fn receive_payment(
        env: Env,
        caller: Address,
        amount: i128,
        to_pool: bool,
        developer: Option<Address>,
        token: Address,
        ledger_seq: u32,
    ) {
        caller.require_auth();
        Self::require_authorized_caller(env.clone(), caller.clone());
        Self::require_supported_token(&env, &token);
        if amount <= 0 {
            env.panic_with_error(SettlementError::AmountNotPositive);
        }

        // Replay guard: reject duplicate / out-of-order settlement claims.
        if to_pool {
            replay_guard::check_pool(&env, ledger_seq).unwrap_or_else(|e| env.panic_with_error(e));
        } else {
            let dev = developer
                .clone()
                .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperRequired));
            replay_guard::check_developer(&env, &dev, ledger_seq)
                .unwrap_or_else(|e| env.panic_with_error(e));
        }

        let inst = env.storage().instance();
        if to_pool {
            if developer.is_some() {
                env.panic_with_error(SettlementError::DeveloperMustBeNone);
            }
            let mut global_pool = Self::get_global_pool(env.clone()).unwrap();
            global_pool.total_balance = global_pool
                .total_balance
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(SettlementError::PoolOverflow));
            global_pool.last_updated = env.ledger().timestamp();
            inst.set(&StorageKey::GlobalPool, &global_pool);
            events::emit_payment_received(
                &env,
                &caller,
                PaymentReceivedEvent {
                    from_vault: caller.clone(),
                    amount,
                    to_pool: true,
                    developer: None,
                    token: token.clone(),
                },
            );
        } else {
            let dev_address = developer
                .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperRequired));

            // Per-token balance key: (developer, token)
            let balance_key = StorageKey::DeveloperBalance(dev_address.clone(), token.clone());

            // Read current balance from persistent storage
            let current_balance: i128 = env
                .storage()
                .persistent()
                .get(&balance_key)
                .unwrap_or(0i128);
            let new_balance = current_balance
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));

            // Write to persistent storage with TTL extension
            env.storage().persistent().set(&balance_key, &new_balance);

            // Keep the developer balance alive for ~120 days (PERSISTENT_BUMP_AMOUNT;
            // the old "1 year" comment was wrong — 50_000 ledgers is ~2.9 days, #1131).
            env.storage().persistent().extend_ttl(
                &balance_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );

            // Add developer to paged persistent index (O(1) membership check).
            Self::index_insert(&env, dev_address.clone());

            events::emit_payment_received(
                &env,
                &caller,
                PaymentReceivedEvent {
                    from_vault: caller.clone(),
                    amount,
                    to_pool: false,
                    developer: Some(dev_address.clone()),
                    token: token.clone(),
                },
            );
            events::emit_balance_credited(
                &env,
                &dev_address,
                BalanceCreditedEvent {
                    developer: dev_address.clone(),
                    amount,
                    new_balance,
                    token: token.clone(),
                },
            );

            events::emit_deposit(
                &env,
                &dev_address, // or `&dev` inside the batch loop
                DepositEvent {
                    developer: dev_address.clone(), // or dev.clone()
                    token: token.clone(),
                    amount,
                },
            );
        }
    }

    /// Atomically credit multiple developer balances in a single call.
    ///
    /// # Arguments
    /// * `caller` - Must be the registered vault address or admin
    /// * `items` - Vec of `(developer_address, amount)` pairs; 1â€“[`MAX_BATCH_SIZE`] entries
    /// * `token` - The token contract address for this batch payment
    ///
    /// # Validation
    /// All amounts must be `> 0`. Empty and oversized batches are rejected before any state change.
    /// The contract returns typed errors for empty batches (`BatchEmpty`), oversized batches
    /// (`BatchTooLarge`), and non-positive amounts (`AmountNotPositive`).
    ///
    /// # Atomicity
    /// All validation runs before any state is written. A failure on any item leaves the
    /// contract state unchanged.
    ///
    /// # Events
    /// Emits `balance_credited` and `deposit` for each item in the batch.
    pub fn batch_receive_payment(
        env: Env,
        caller: Address,
        items: Vec<(Address, i128)>,
        token: Address,
        ledger_seq: u32,
    ) {
        caller.require_auth();
        Self::require_authorized_caller(env.clone(), caller.clone());

        let n = items.len();
        Self::require_supported_token(&env, &token);
        if n == 0 {
            env.panic_with_error(SettlementError::BatchEmpty);
        }
        if n > MAX_BATCH_SIZE {
            env.panic_with_error(SettlementError::BatchTooLarge);
        }

        // Validate all amounts before touching state.
        for item in items.iter() {
            let (_, amount) = item;
            if amount <= 0 {
                env.panic_with_error(SettlementError::AmountNotPositive);
            }
        }

        // Replay guard: validate ALL developer HWMs before any state change.
        for item in items.iter() {
            let (dev, _) = item;
            replay_guard::check_developer(&env, &dev, ledger_seq)
                .unwrap_or_else(|e| env.panic_with_error(e));
        }

        for item in items.iter() {
            let (dev, amount) = item;
            let balance_key = StorageKey::DeveloperBalance(dev.clone(), token.clone());
            let current: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
            let new_balance = current
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));
            env.storage().persistent().set(&balance_key, &new_balance);
            env.storage().persistent().extend_ttl(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            // Add to paged persistent index (O(1) membership check).
            Self::index_insert(&env, dev.clone());
            events::emit_balance_credited(
                &env,
                &dev,
                BalanceCreditedEvent {
                    developer: dev.clone(),
                    amount,
                    new_balance,
                    token: token.clone(),
                },
            );
            events::emit_deposit(
                &env,
                &dev,
                DepositEvent {
                    developer: dev.clone(),
                    token: token.clone(),
                    amount,
                },
            );
        }
    }

    /// Get current admin address
    pub fn get_admin(env: Env) -> Result<Address, SettlementError> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(SettlementError::NotInitialized)
    }

    /// Set the minimum balance for a developer (admin only).
    ///
    /// A withdrawal that would leave the developer's balance below this
    /// threshold is rejected with [`SettlementError::MinBalanceViolation`].
    /// Setting `min_balance` to `0` removes the restriction.
    ///
    /// Emits `developer_min_balance_changed`.
    pub fn set_developer_min_balance(
        env: Env,
        caller: Address,
        developer: Address,
        min_balance: i128,
    ) {
        limits::set_developer_min_balance(&env, caller, developer, min_balance);
    }

    /// Get the minimum balance for a developer.
    ///
    /// Returns `0` if no minimum has been configured (no restriction).
    pub fn get_developer_min_balance(env: Env, developer: Address) -> i128 {
        limits::get_developer_min_balance(&env, developer)
    }
    /// Return the contract crate version as a Soroban string.
    ///
    /// The value is sourced from the package manifest so the on-chain version
    /// stays aligned with the compiled artifact.
    pub fn version(_env: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&_env, env!("CARGO_PKG_VERSION"))
    }

    /// Get registered vault address
    pub fn get_vault(env: Env) -> Result<Address, SettlementError> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage()
            .instance()
            .get(&StorageKey::Vault)
            .ok_or(SettlementError::NotInitialized)
    }

    /// Get global pool information
    pub fn get_global_pool(env: Env) -> Result<GlobalPool, SettlementError> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage()
            .instance()
            .get::<_, GlobalPool>(&StorageKey::GlobalPool)
            .ok_or(SettlementError::NotInitialized)
    }

    /// Return the cumulative total of all funds received via `receive_payment` and
    /// `batch_receive_payment`, regardless of routing (pool or developer). Returns
    /// `0` before any payments.
    pub fn get_total_received(env: Env) -> i128 {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage()
            .instance()
            .get(&StorageKey::TotalReceived)
            .unwrap_or(0)
    }

    /// Get developer balance for a specific token.
    ///
    /// Performs a direct O(1) persistent storage lookup for the specified
    /// developer's balance denominated in `token`. Bumps instance and persistent TTL on read.
    pub fn get_developer_balance(
        env: Env,
        developer: Address,
        token: Address,
    ) -> Result<i128, SettlementError> {
        if !env.storage().instance().has(&StorageKey::Admin) {
            return Err(SettlementError::NotInitialized);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        let key = StorageKey::DeveloperBalance(developer, token);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        Ok(env.storage().persistent().get(&key).unwrap_or(0))
    }

    /// Propose moving a developer's current balance to a replacement address.
    ///
    /// The current admin must authorize this state change. If the admin is a
    /// Stellar multisig account, `require_auth` enforces that account's signer
    /// thresholds. The proposal snapshots the source balance and becomes
    /// executable after [`DEVELOPER_MIGRATION_TIMELOCK_SECONDS`]. Re-proposing
    /// for the same source replaces the prior proposal and restarts the delay.
    pub fn propose_balance_migration(env: Env, caller: Address, from: Address, to: Address) {
        admin::propose_balance_migration(&env, &caller, &from, &to);
    }

    /// Execute a matured developer balance migration proposal.
    ///
    /// The current admin must authorize execution independently of proposal.
    /// Exactly the amount approved at proposal time is moved; credits received
    /// afterward remain at `from`.
    pub fn execute_balance_migration(env: Env, caller: Address, from: Address) {
        admin::execute_balance_migration(&env, &caller, &from);
    }

    /// Return the pending migration for `from`, if one exists.
    pub fn get_balance_migration(env: Env, from: Address) -> Option<PendingDeveloperMigration> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        timelock::get_pending_migration(&env, &from)
    }

    /// Configure the USDC token contract address.
    ///
    /// Only the current admin may set the on-chain USDC token address that this
    /// contract will use to execute withdrawals.
    pub fn set_usdc_token(env: Env, caller: Address, usdc_address: Address) {
        caller.require_auth();
        let current_admin = Self::get_admin(env.clone()).unwrap();
        if caller != current_admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        if usdc_address == env.current_contract_address() {
            env.panic_with_error(SettlementError::InvalidUsdcToken);
        }
        env.storage()
            .instance()
            .set(&StorageKey::Usdc, &usdc_address);
        Self::add_supported_token_internal(&env, &caller, &usdc_address);
    }

    /// Enable a token for settlement payments. Admin only.
    pub fn add_supported_token(env: Env, caller: Address, token: Address) {
        caller.require_auth();
        let current_admin = Self::get_admin(env.clone()).unwrap();
        if caller != current_admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        Self::add_supported_token_internal(&env, &caller, &token);
    }

    /// Disable a token for future settlement payments. Existing balances are retained.
    pub fn remove_supported_token(env: Env, caller: Address, token: Address) {
        caller.require_auth();
        let current_admin = Self::get_admin(env.clone()).unwrap();
        if caller != current_admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        let key = StorageKey::SupportedToken(token.clone());
        if env.storage().persistent().has(&key) {
            env.storage().persistent().remove(&key);
            events::emit_supported_token_removed(&env, &caller, &token);
        }
    }

    /// Check whether a token is registered for settlement payments.
    pub fn is_supported_token(env: Env, token: Address) -> bool {
        let key = StorageKey::SupportedToken(token);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            true
        } else {
            false
        }
    }

    fn get_usdc_token(env: Env) -> Result<Address, SettlementError> {
        env.storage()
            .instance()
            .get(&StorageKey::Usdc)
            .ok_or(SettlementError::UsdcTokenNotConfigured)
    }

    /// Withdraw developer balance as USDC to a designated recipient.
    ///
    /// Requires the developer to authorize the request and the claim to pass
    /// the shared claim validation (see `validate_claim`) for the
    /// configured USDC token.
    ///
    /// # Arguments
    /// * `developer` - Address of the developer withdrawing their balance.
    /// * `amount` - Amount to withdraw in USDC micro-units.
    /// * `to` - Optional recipient address; if `None`, defaults to `developer`.
    ///
    /// # Errors
    /// - `DeveloperFrozen` if the developer's withdrawals are frozen.
    /// - `AmountNotPositive` if amount is <= 0.
    /// - `ClaimWindowClosed` if a developer claim window exists and the current
    ///   ledger timestamp is outside that inclusive window.
    /// - `UsdcTokenNotConfigured` if USDC token not set.
    /// - `InsufficientDeveloperBalance` if developer balance < amount.
    /// - `MinBalanceViolation` if the withdrawal would leave the developer
    ///   below their configured minimum balance.
    /// - `DailyWithdrawCapExceeded` if daily cap is exceeded.
    /// - `DeveloperBalanceUnderflow` if subtraction underflows.
    /// - `InsufficientContractBalance` if contract has insufficient USDC.
    /// - Panics if `to` is the contract's own address.
    pub fn withdraw_developer_balance(
        env: Env,
        developer: Address,
        amount: i128,
        to: Option<Address>,
    ) -> Result<(), SettlementError> {
        developer.require_auth();
        Self::withdraw_developer_balance_inner(&env, developer, amount, to)
    }

    /// Body of [`withdraw_developer_balance`] after authorization (#1135).
    ///
    /// Shared by the single and batch withdrawal entrypoints so both enforce
    /// exactly the same checks (freeze, amount, claim window, balance,
    /// minimum balance, daily cap, contract liquidity), the same CEI order
    /// and the same events. Callers must have authorized `developer` first.
    fn withdraw_developer_balance_inner(
        env: &Env,
        developer: Address,
        amount: i128,
        to: Option<Address>,
    ) -> Result<(), SettlementError> {
        let env = env.clone();
        let sim = Self::validate_claim(&env, &developer, amount, to)?;
        Self::bump_claim_storage_ttl(&env, &developer);

        let contract_address = env.current_contract_address();
        let usdc = token::Client::new(&env, &sim.token);

        // --- EFFECTS (persist all state mutations before the external call) ---
        // CEI: write the reduced balance and updated daily-withdrawal counter
        // here so that any re-entrant call via the token contract observes the
        // already-reduced balance and cannot withdraw a second time.
        let balance_key = StorageKey::DeveloperBalance(sim.developer.clone(), sim.token.clone());
        env.storage()
            .persistent()
            .set(&balance_key, &sim.remaining_balance);
        env.storage().persistent().extend_ttl(
            &balance_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        let today = env.ledger().timestamp() / 86400;
        let today_key = StorageKey::WithdrawalToday(sim.developer.clone());
        env.storage().persistent().set(
            &today_key,
            &DailyWithdrawState {
                day: today,
                amount: sim.withdrawn_today_after,
            },
        );
        env.storage().persistent().extend_ttl(
            &today_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        // --- INTERACTION (external token transfer happens last) ---
        usdc.transfer(&contract_address, &sim.recipient, &sim.amount);

        events::emit_developer_withdraw(
            &env,
            &sim.developer.clone(),
            DeveloperWithdrawEvent {
                developer: sim.developer,
                amount: sim.amount,
                remaining_balance: sim.remaining_balance,
                to: sim.recipient,
                token: sim.token,
            },
        );

        Ok(())
    }

    /// Extend the TTLs that a real claim historically maintained: the
    /// instance, the developer's claim-window entry (when configured), and
    /// the developer's minimum-balance entry (when configured).
    ///
    /// Validation itself ([`Self::validate_claim`]) is read-only so that
    /// `simulate_claim` never mutates storage; only the state-changing
    /// `withdraw_developer_balance` path performs this maintenance.
    fn bump_claim_storage_ttl(env: &Env, developer: &Address) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        let window_key = StorageKey::DeveloperClaimWindow(developer.clone());
        if env.storage().persistent().has(&window_key) {
            env.storage().persistent().extend_ttl(
                &window_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        let min_key = StorageKey::DeveloperMinBalance(developer.clone());
        if env.storage().persistent().has(&min_key) {
            env.storage().persistent().extend_ttl(
                &min_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
    }

    /// Simulate a developer claim without side effects.
    ///
    /// This read-only view previews the outcome of `withdraw_developer_balance`
    /// for the configured USDC token by running the exact shared claim
    /// validation. It intentionally does not require developer authorization
    /// and does not transfer tokens, mutate balances, update daily withdrawal
    /// counters, extend TTLs, or emit events.
    ///
    /// The view returns the same typed errors as a real claim for developer
    /// freeze state, positive amount, claim window, developer balance,
    /// per-developer minimum balance, daily cap, USDC configuration, and
    /// contract token-liquidity failures. If `to` is `None`, the simulated
    /// recipient is the developer address.
    pub fn simulate_claim(
        env: Env,
        developer: Address,
        amount: i128,
        to: Option<Address>,
    ) -> Result<ClaimSimulation, SettlementError> {
        Self::validate_claim(&env, &developer, amount, to)
    }

    /// Shared claim validation used by both `withdraw_developer_balance` and
    /// `simulate_claim` so the simulation mirrors a real claim exactly.
    ///
    /// Checks, in order: developer freeze, positive amount, recipient
    /// (panics on the contract's own address), open claim window,
    /// configured USDC token, developer balance vs. amount,
    /// per-developer minimum balance, daily withdrawal cap, and contract
    /// token liquidity.
    ///
    /// This function is read-only: it never extends TTLs or writes storage.
    /// The state-changing caller (`withdraw_developer_balance`) performs TTL
    /// maintenance itself via `bump_claim_storage_ttl`.
    fn validate_claim(
        env: &Env,
        developer: &Address,
        amount: i128,
        to: Option<Address>,
    ) -> Result<ClaimSimulation, SettlementError> {
        if freeze::is_developer_frozen(env.clone(), developer.clone()) {
            return Err(SettlementError::DeveloperFrozen);
        }
        if amount <= 0 {
            return Err(SettlementError::AmountNotPositive);
        }

        let recipient = to.unwrap_or_else(|| developer.clone());
        let contract_address = env.current_contract_address();
        if recipient == contract_address {
            env.panic_with_error(SettlementError::InvalidRecipient);
        }

        Self::require_claim_window_open(env, developer)?;

        let usdc_address = Self::get_usdc_token(env.clone())?;
        let current_balance: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::DeveloperBalance(
                developer.clone(),
                usdc_address.clone(),
            ))
            .unwrap_or(0);
        let remaining = current_balance
            .checked_sub(amount)
            .ok_or(SettlementError::InsufficientDeveloperBalance)?;
        limits::check_min_balance(env, developer, remaining)?;
        if amount > current_balance {
            return Err(SettlementError::InsufficientDeveloperBalance);
        }

        let today = env.ledger().timestamp() / 86400;
        let daily = env
            .storage()
            .persistent()
            .get::<_, DailyWithdrawState>(&StorageKey::WithdrawalToday(developer.clone()))
            .unwrap_or(DailyWithdrawState {
                day: today,
                amount: 0,
            });
        let withdrawn_today = if daily.day == today { daily.amount } else { 0 };
        let cap: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::DailyWithdrawCap(developer.clone()))
            .unwrap_or(0);
        let withdrawn_today_after = withdrawn_today
            .checked_add(amount)
            .ok_or(SettlementError::DailyWithdrawCapExceeded)?;
        if cap > 0 && withdrawn_today_after > cap {
            return Err(SettlementError::DailyWithdrawCapExceeded);
        }

        let remaining_balance = current_balance
            .checked_sub(amount)
            .ok_or(SettlementError::DeveloperBalanceUnderflow)?;
        let contract_balance = token::Client::new(env, &usdc_address).balance(&contract_address);
        if contract_balance < amount {
            return Err(SettlementError::InsufficientContractBalance);
        }

        Ok(ClaimSimulation {
            developer: developer.clone(),
            amount,
            recipient,
            token: usdc_address,
            current_balance,
            remaining_balance,
            contract_balance,
            daily_withdraw_cap: cap,
            withdrawn_today,
            withdrawn_today_after,
        })
    }

    /// Configure the inclusive claim window for a developer.
    ///
    /// A configured window restricts `withdraw_developer_balance` so the
    /// developer can claim only when the current ledger timestamp is between
    /// `start_ts` and `end_ts`, inclusive. Developers with no configured
    /// window remain claimable at any time.
    ///
    /// # Access Control
    /// Only the current admin can call this function.
    ///
    /// # Errors
    /// - `Unauthorized` if caller is not the current admin.
    /// - `InvalidClaimWindow` if `end_ts < start_ts`.
    ///
    /// # Events
    /// Emits `claim_window_changed` with `enabled = true`.
    pub fn set_developer_claim_window(
        env: Env,
        caller: Address,
        developer: Address,
        start_ts: u64,
        end_ts: u64,
    ) -> Result<(), SettlementError> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            return Err(SettlementError::Unauthorized);
        }
        if end_ts < start_ts {
            return Err(SettlementError::InvalidClaimWindow);
        }

        let window_key = StorageKey::DeveloperClaimWindow(developer.clone());
        env.storage()
            .persistent()
            .set(&window_key, &DeveloperClaimWindow { start_ts, end_ts });
        env.storage().persistent().extend_ttl(
            &window_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        events::emit_developer_claim_window_changed(
            &env,
            &developer.clone(),
            DeveloperClaimWindowChanged {
                developer,
                start_ts,
                end_ts,
                enabled: true,
            },
        );

        Ok(())
    }

    /// Clear a developer's claim window and restore unrestricted claiming.
    ///
    /// # Access Control
    /// Only the current admin can call this function.
    ///
    /// # Events
    /// Emits `claim_window_changed` with `enabled = false`.
    pub fn clear_developer_claim_window(
        env: Env,
        caller: Address,
        developer: Address,
    ) -> Result<(), SettlementError> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            return Err(SettlementError::Unauthorized);
        }

        env.storage()
            .persistent()
            .remove(&StorageKey::DeveloperClaimWindow(developer.clone()));

        events::emit_developer_claim_window_changed(
            &env,
            &developer.clone(),
            DeveloperClaimWindowChanged {
                developer,
                start_ts: 0,
                end_ts: 0,
                enabled: false,
            },
        );

        Ok(())
    }

    /// Return the configured claim window for a developer, or `None` when
    /// claims are unrestricted. Bumps instance and persistent TTL on read.
    pub fn get_developer_claim_window(
        env: Env,
        developer: Address,
    ) -> Option<DeveloperClaimWindow> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        let key = StorageKey::DeveloperClaimWindow(developer);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        env.storage().persistent().get(&key)
    }

    /// Abort with `ClaimWindowClosed` when a developer has a configured claim
    /// window and the current ledger timestamp falls outside its inclusive
    /// `[start_ts, end_ts]` range. A developer with no configured window may
    /// claim at any time.
    ///
    /// This check is read-only: it never extends TTLs, so callers that need
    /// TTL maintenance (e.g. `withdraw_developer_balance`) must extend the
    /// [`StorageKey::DeveloperClaimWindow`] entry explicitly.
    fn require_claim_window_open(env: &Env, developer: &Address) -> Result<(), SettlementError> {
        let key = StorageKey::DeveloperClaimWindow(developer.clone());
        let window: Option<DeveloperClaimWindow> = env.storage().persistent().get(&key);
        if let Some(window) = window {
            let now = env.ledger().timestamp();
            if now < window.start_ts || now > window.end_ts {
                return Err(SettlementError::ClaimWindowClosed);
            }
        }
        Ok(())
    }

    /// Set the daily withdrawal cap for a developer (admin only).
    ///
    /// A cap of `0` means unlimited (no daily limit enforced).
    /// Negative caps are rejected with `AmountNotPositive`.
    ///
    /// # Errors
    /// Returns `Unauthorized` if the caller is not the admin, or
    /// `AmountNotPositive` if `cap` is negative.
    ///
    /// # Events
    /// Emits `daily_withdraw_cap_changed` with the developer and new cap.
    pub fn set_daily_withdraw_cap(
        env: Env,
        caller: Address,
        developer: Address,
        cap: i128,
    ) -> Result<(), SettlementError> {
        caller.require_auth();
        let current_admin = Self::get_admin(env.clone())?;
        if caller != current_admin {
            return Err(SettlementError::Unauthorized);
        }
        if cap < 0 {
            return Err(SettlementError::AmountNotPositive);
        }
        let cap_key = StorageKey::DailyWithdrawCap(developer.clone());
        env.storage().persistent().set(&cap_key, &cap);
        env.storage().persistent().extend_ttl(
            &cap_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        events::emit_daily_withdraw_cap_changed(
            &env,
            &caller,
            DailyWithdrawCapChanged {
                developer,
                new_cap: cap,
            },
        );
        Ok(())
    }

    /// Get the daily withdrawal cap for a developer. Returns `0` (unlimited)
    /// if no cap has been set. Bumps instance and persistent TTL on read.
    pub fn get_daily_withdraw_cap(env: Env, developer: Address) -> i128 {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        let key = StorageKey::DailyWithdrawCap(developer);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        env.storage().persistent().get(&key).unwrap_or(0)
    }

    /// Get the amount a developer has already withdrawn today (UTC epoch day).
    /// Returns `0` if no withdrawal has been made today. Bumps instance and persistent TTL on read.
    pub fn get_withdrawal_today(env: Env, developer: Address) -> i128 {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        let key = StorageKey::WithdrawalToday(developer);
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        let state: Option<DailyWithdrawState> = env.storage().persistent().get(&key);
        match state {
            Some(s) if s.day == env.ledger().timestamp() / 86400 => s.amount,
            _ => 0,
        }
    }

    /// Configure the minimum-balance advisory threshold for a developer.
    ///
    /// This is a compatibility wrapper around [`Self::set_developer_min_balance`]
    /// and uses the same admin-only authorization and storage semantics.
    pub fn set_minimum_balance(env: Env, caller: Address, developer: Address, min_balance: i128) {
        limits::set_developer_min_balance(&env, caller, developer, min_balance);
    }

    /// Return the configured minimum-balance advisory threshold for a developer.
    ///
    /// Returns `0` when no minimum balance has been configured.
    pub fn get_minimum_balance(env: Env, developer: Address) -> i128 {
        limits::get_developer_min_balance(&env, developer)
    }

    /// Admin-only escape hatch to manually credit a developer balance for a
    /// specific token.
    ///
    /// Note: The admin can invoke this entrypoint directly to credit developer
    /// balances.
    ///
    /// This function is designed for operational edge cases where a developer
    /// must be credited outside the normal `receive_payment` flow (e.g.,
    /// off-chain payment reconciliation, dispute resolution). It does **not**
    /// move on-ledger tokens and is treated as an audited administrative inflow.
    ///
    /// # Panics
    /// * `Unauthorized` â€” caller is not admin.
    /// * `AmountNotPositive` â€” amount is zero or negative.
    /// * `DeveloperOverflow` â€” i128 overflow on developer balance.
    ///
    /// # Events
    /// Emits `developer_force_credited`.
    pub fn force_credit_developer(
        env: Env,
        caller: Address,
        developer: Address,
        amount: i128,
        token: Address,
        reason: Symbol,
    ) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        if amount <= 0 {
            env.panic_with_error(SettlementError::AmountNotPositive);
        }

        let balance_key = StorageKey::DeveloperBalance(developer.clone(), token.clone());
        let current_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key)
            .unwrap_or(0i128);
        let new_balance = current_balance
            .checked_add(amount)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));

        env.storage().persistent().set(&balance_key, &new_balance);
        env.storage().persistent().extend_ttl(
            &balance_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        // Add to paged persistent index (O(1) membership check).
        Self::index_insert(&env, developer.clone());

        events::emit_developer_force_credited(
            &env,
            &developer.clone(),
            DeveloperForceCreditedEvent {
                developer,
                amount,
                reason,
                new_balance,
                token,
            },
        );
    }

    /// Get all developer balances for a specific token (admin only).
    ///
    /// Iterates the full paged developer index. For deployments with many
    /// developers, prefer `get_developer_balances_cursor` for bounded,
    /// paginated access. Bumps instance and persistent TTL for retrieved entries.
    pub fn get_all_developer_balances(
        env: Env,
        caller: Address,
        token: Address,
    ) -> Vec<DeveloperBalance> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap_or_else(|e| env.panic_with_error(e));
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        let mut result = Vec::new(&env);
        Self::iter_index(&env, |address| {
            let key = StorageKey::DeveloperBalance(address.clone(), token.clone());
            if env.storage().persistent().has(&key) {
                env.storage().persistent().extend_ttl(
                    &key,
                    PERSISTENT_BUMP_THRESHOLD,
                    PERSISTENT_BUMP_AMOUNT,
                );
            }
            let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
            result.push_back(DeveloperBalance {
                address,
                token: token.clone(),
                balance,
            });
        });
        result
    }

    /// Get a start/limit-paginated slice of developer balances for a token
    /// (admin only). `limit` is capped at [`MAX_DEVELOPER_BALANCES_PAGE_SIZE`].
    /// Bumps instance and persistent TTL for retrieved entries.
    ///
    /// `start` and `limit` are global offsets across all index pages.
    pub fn get_developer_balances_page(
        env: Env,
        caller: Address,
        start: u32,
        limit: u32,
        token: Address,
    ) -> Vec<DeveloperBalance> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        if limit == 0 {
            return Vec::new(&env);
        }
        let effective_limit = limit.min(MAX_DEVELOPER_BALANCES_PAGE_SIZE);

        let mut result = Vec::new(&env);
        let mut global_cursor: u32 = 0;
        let end = start.saturating_add(effective_limit);

        Self::iter_index(&env, |address| {
            if result.len() >= effective_limit {
                return;
            }
            if global_cursor >= end {
                global_cursor = global_cursor.saturating_add(1);
                return;
            }
            if global_cursor >= start {
                let key = StorageKey::DeveloperBalance(address.clone(), token.clone());
                if env.storage().persistent().has(&key) {
                    env.storage().persistent().extend_ttl(
                        &key,
                        PERSISTENT_BUMP_THRESHOLD,
                        PERSISTENT_BUMP_AMOUNT,
                    );
                }
                let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);
                result.push_back(DeveloperBalance {
                    address,
                    token: token.clone(),
                    balance,
                });
            }
            global_cursor = global_cursor.saturating_add(1);
        });
        result
    }

    /// Cursor-based paginated developer balances for a specific token (admin only).
    ///
    /// Returns up to `limit` developer balance records starting **after** the
    /// supplied `cursor` address (exclusive), or from the beginning of the
    /// paged index when `cursor` is `None`.
    pub fn get_developer_balances_cursor(
        env: Env,
        caller: Address,
        cursor: Option<Address>,
        limit: u32,
        token: Address,
    ) -> (Vec<DeveloperBalance>, Option<Address>) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap_or_else(|e| env.panic_with_error(e));
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }

        pagination::get_page(&env, cursor, limit, &token)
    }

    /// Return the pending admin address, or `None` if no two-step admin transfer is in progress.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage().instance().get(&StorageKey::PendingAdmin)
    }

    /// Nominate a new admin (admin only). The nominee must call `accept_admin`
    /// to finalize the transfer.
    ///
    /// # Events
    /// Emits `admin_nominated` with `(current_admin, new_admin)`.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);
        events::emit_admin_nominated(&env, &admin, &new_admin);
    }

    /// Finalize a pending admin transfer. Must be called by the nominated admin.
    ///
    /// # Panics
    /// * `"no admin transfer pending"` â€” `set_admin` was not called first.
    ///
    /// # Events
    /// Emits `admin_accepted` with `(old_admin, new_admin)`.
    pub fn accept_admin(env: Env) {
        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .unwrap_or_else(|| panic!("no admin transfer pending"));
        pending.require_auth();
        let old_admin = Self::get_admin(env.clone()).unwrap();
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &pending);
        inst.remove(&StorageKey::PendingAdmin);
        events::emit_admin_accepted(&env, &old_admin, &pending);
    }

    /// Cancel a pending admin transfer (admin only).
    ///
    /// # Panics
    /// * `"no admin transfer pending"` â€” no nomination is in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled`.
    pub fn cancel_admin_transfer(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        if !env.storage().instance().has(&StorageKey::PendingAdmin) {
            env.panic_with_error(SettlementError::NoAdminTransferPending);
        }
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        events::emit_admin_cancelled(&env, &admin);
    }

    /// Propose a new vault address (admin only). The proposed vault (or the
    /// admin) must call `accept_vault` to finalize.
    ///
    /// # Events
    /// Emits `vault_proposed` with [`VaultProposedEvent`].
    pub fn propose_vault(env: Env, caller: Address, new_vault: Address) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        if new_vault == env.current_contract_address() {
            env.panic_with_error(SettlementError::InvalidVault);
        }
        let current_vault = Self::get_vault(env.clone()).unwrap();
        env.storage()
            .instance()
            .set(&StorageKey::PendingVault, &new_vault);
        events::emit_vault_proposed(
            &env,
            &admin,
            VaultProposedEvent {
                current_vault,
                proposed_vault: new_vault,
            },
        );
    }

    /// Alias for `propose_vault` (admin only).
    pub fn set_vault(env: Env, caller: Address, new_vault: Address) {
        Self::propose_vault(env, caller, new_vault);
    }

    /// Finalize a pending vault rotation. Must be called by the pending vault
    /// itself — proving the new vault address can authorize — to prevent a
    /// single admin key from completing the rotation alone.
    ///
    /// # Errors
    /// * [`SettlementError::NoVaultRotationPending`] — `propose_vault` was not
    ///   called first.
    /// * [`SettlementError::Unauthorized`] — caller is not the pending vault.
    ///
    /// # Events
    /// Emits `vault_accepted` with [`VaultAcceptedEvent`].
    pub fn accept_vault(env: Env, caller: Address) {
        caller.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingVault)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::NoVaultRotationPending));
        // Only the pending vault may accept — not the admin.  This closes the
        // one-actor bypass: the admin can propose but cannot unilaterally
        // complete the two-step rotation.
        if caller != pending {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        let old_vault = Self::get_vault(env.clone()).unwrap();
        let inst = env.storage().instance();
        inst.set(&StorageKey::Vault, &pending);
        inst.remove(&StorageKey::PendingVault);
        events::emit_vault_accepted(
            &env,
            &pending,
            VaultAcceptedEvent {
                old_vault,
                new_vault: pending.clone(),
                accepted_by: caller,
            },
        );
    }

    /// Broadcast an operator/emergency message (admin only).
    pub fn broadcast(env: Env, caller: Address, severity: Severity, message: soroban_sdk::String) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        if message.len() > MAX_BROADCAST_MESSAGE_LEN {
            env.panic_with_error(SettlementError::BroadcastMessageTooLong);
        }
        events::emit_admin_broadcast(&env, &caller, AdminBroadcast { severity, message });
    }

    /// Propose a timelocked WASM upgrade (admin only).
    ///
    /// Records a [`PendingUpgrade`] snapshot containing `new_wasm_hash` and an
    /// execution deadline of `now + UPGRADE_TIMELOCK_SECONDS` (48 h). A second
    /// call replaces any existing proposal and restarts the delay.
    ///
    /// # Arguments
    /// * `caller` - Must be the current admin; must authorize.
    /// * `new_wasm_hash` - 32-byte WASM hash to install on execution. Must not
    ///   be all-zero bytes.
    ///
    /// # Errors
    /// * `Unauthorized` — caller is not the admin.
    /// * `ZeroWasmHash` — all-zero `new_wasm_hash` is rejected.
    /// * `TimelockOverflow` — `proposed_at + delay` overflows `u64`.
    ///
    /// # Events
    /// Emits `upgrade_proposed` with the hash, `proposed_at`, and `execute_after`.
    pub fn propose_upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        // Reject zero hash (32 zero bytes) as an obviously invalid WASM address.
        if new_wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            env.panic_with_error(SettlementError::ZeroWasmHash);
        }
        let proposed_at = env.ledger().timestamp();
        let execute_after = proposed_at
            .checked_add(UPGRADE_TIMELOCK_SECONDS)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::TimelockOverflow));
        let proposal = timelock::PendingUpgrade {
            wasm_hash: new_wasm_hash.clone(),
            proposed_at,
            execute_after,
        };
        timelock::set_pending_upgrade(&env, &proposal);
        events::emit_upgrade_proposed(
            &env,
            &caller,
            UpgradeProposedEvent {
                wasm_hash: new_wasm_hash,
                proposed_at,
                execute_after,
            },
        );
    }

    /// Execute a matured WASM upgrade proposal (admin only).
    ///
    /// The admin must authorize independently of `propose_upgrade`. Exactly the
    /// WASM hash recorded at proposal time is installed; the proposal is cleared
    /// atomically to prevent replay.
    ///
    /// # Errors
    /// * `Unauthorized` — caller is not the admin.
    /// * `NoUpgradePending` — no proposal exists.
    /// * `UpgradeTimelockNotExpired` — `execute_after` has not been reached.
    ///
    /// # Events
    /// Emits `upgraded` with the installed WASM hash (existing event topic,
    /// kept for consistency with `get_version` / off-chain indexers).
    pub fn execute_upgrade(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        let proposal = timelock::get_pending_upgrade(&env)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::NoUpgradePending));
        let now = env.ledger().timestamp();
        if now < proposal.execute_after {
            env.panic_with_error(SettlementError::UpgradeTimelockNotExpired);
        }
        // Consume the proposal before calling the deployer to prevent re-entry.
        timelock::clear_pending_upgrade(&env);
        env.deployer()
            .update_current_contract_wasm(proposal.wasm_hash.clone());
        env.storage()
            .instance()
            .set(&StorageKey::ContractVersion, &proposal.wasm_hash);
        events::emit_upgraded(&env, &caller, &proposal.wasm_hash);
    }

    /// Cancel a pending upgrade proposal (admin only).
    ///
    /// # Errors
    /// * `Unauthorized` — caller is not the admin.
    /// * `NoUpgradePending` — no proposal exists.
    ///
    /// # Events
    /// Emits `upgrade_cancelled` with the cancelled WASM hash and current timestamp.
    pub fn cancel_upgrade(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        let proposal = timelock::get_pending_upgrade(&env)
            .unwrap_or_else(|| env.panic_with_error(SettlementError::NoUpgradePending));
        timelock::clear_pending_upgrade(&env);
        events::emit_upgrade_cancelled(
            &env,
            &caller,
            UpgradeCancelledEvent {
                wasm_hash: proposal.wasm_hash,
                cancelled_at: env.ledger().timestamp(),
            },
        );
    }

    /// Return the pending upgrade proposal, or `None` if none is in progress.
    ///
    /// Read-only; no auth required. Bumps instance TTL on call.
    pub fn get_pending_upgrade(env: Env) -> Option<PendingUpgrade> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        timelock::get_pending_upgrade(&env)
    }

    /// Deprecated: alias for `propose_upgrade`.
    ///
    /// Retained for interface compatibility. Callers should migrate to
    /// `propose_upgrade` + `execute_upgrade` for timelocked upgrades.
    ///
    /// # Events
    /// Emits `upgrade_proposed` (not `upgraded`). The upgrade is **not**
    /// applied immediately; call `execute_upgrade` after the delay.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        Self::propose_upgrade(env, caller, new_wasm_hash);
    }

    /// Return the WASM hash installed by the most recent `upgrade` call, or
    /// `None` if the contract has never been upgraded.
    pub fn get_version(env: Env) -> Option<BytesN<32>> {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        env.storage().instance().get(&StorageKey::ContractVersion)
    }

    /// Migrate a single developer's V1 balance to V2 (admin only).
    pub fn migrate_developer_balance(
        env: Env,
        caller: Address,
        developer: Address,
    ) -> Result<(), SettlementError> {
        migrate::migrate_single_developer(&env, &caller, &developer)
    }

    /// Migrate a single developer's V1 balance to V2 (admin only). Alias of
    /// `migrate_developer_balance`.
    pub fn migrate_single_dev_v2(
        env: Env,
        caller: Address,
        developer: Address,
    ) -> Result<(), SettlementError> {
        migrate::migrate_single_developer(&env, &caller, &developer)
    }

    /// One-shot V1 -> V2 storage migration (admin only). See [`migrate`] module docs.
    pub fn migrate_v1_to_v2(env: Env, caller: Address) {
        migrate::migrate_v1_to_v2(&env, &caller);
    }

    /// Paginated V1 -> V2 storage migration (admin only). See [`migrate`] module docs.
    pub fn migrate_v1_to_v2_page(
        env: Env,
        caller: Address,
        offset: u32,
        batch_size: u32,
    ) -> (u32, bool) {
        migrate::migrate_v1_to_v2_page(&env, &caller, offset, batch_size)
    }

    /// Return the current storage-layout version (1 = legacy single-token, 2 = per-token).
    pub fn migration_storage_version(env: Env) -> u32 {
        migrate::storage_version(&env)
    }

    /// Migrate the legacy flat `DeveloperIndex` (instance storage) to the new
    /// paged persistent index.  Processes up to `batch_size` (capped at
    /// [`MAX_BATCH_SIZE`]) addresses per call starting at `offset`.
    ///
    /// Call repeatedly, passing the returned `next_offset` back in, until
    /// `is_complete == true`.  Re-running after completion is a safe no-op.
    ///
    /// Admin signature required.
    pub fn migrate_index_to_pages(
        env: Env,
        caller: Address,
        offset: u32,
        batch_size: u32,
    ) -> (u32, bool) {
        migrate::migrate_index_to_pages(&env, &caller, offset, batch_size)
    }

    /// Cursor-based batch developer withdrawal (#1135).
    ///
    /// Processes `developers[cursor .. min(cursor + limit, len)]`, withdrawing
    /// `amounts[i]` from each developer's balance to that developer's own
    /// address, and returns `(next_cursor, done)`. Passing `next_cursor` back
    /// resumes exactly where this call stopped; `done` is `true` once
    /// `next_cursor == developers.len()`.
    ///
    /// # Authorization
    /// Every developer in the processed window must authorize the call
    /// (`developer.require_auth()`), exactly as for
    /// [`withdraw_developer_balance`] — an operator can batch payouts but can
    /// never move a developer's funds without that developer's signature.
    /// Developers outside the window are neither authorized nor touched.
    ///
    /// # Atomicity
    /// Each item reuses the single-withdrawal internals. If any item in the
    /// window fails (frozen, closed claim window, insufficient balance, cap,
    /// liquidity…), the error is returned and Soroban rolls back the whole
    /// invocation, so a window is applied all-or-nothing and the caller can
    /// retry the same cursor after fixing the offending item.
    ///
    /// # Errors
    /// - `LengthMismatch` if `developers.len() != amounts.len()`.
    /// - `BatchEmpty` if `developers` is empty.
    /// - `BatchTooLarge` if `developers.len() > MAX_BATCH_SIZE`.
    /// - `InvalidCursor` if `cursor > developers.len()` or `limit == 0`.
    /// - Any error of [`withdraw_developer_balance`] for an item in the window.
    ///
    /// `limit` is capped at [`MAX_BATCH_SIZE`].
    pub fn batch_withdraw_balance_cursor(
        env: Env,
        developers: Vec<Address>,
        amounts: Vec<i128>,
        cursor: u32,
        limit: u32,
    ) -> Result<(u32, bool), SettlementError> {
        let count = developers.len();
        if count != amounts.len() {
            return Err(SettlementError::LengthMismatch);
        }
        if count == 0 {
            return Err(SettlementError::BatchEmpty);
        }
        if count > MAX_BATCH_SIZE {
            return Err(SettlementError::BatchTooLarge);
        }
        if cursor > count || limit == 0 {
            return Err(SettlementError::InvalidCursor);
        }

        let end = cursor.saturating_add(limit.min(MAX_BATCH_SIZE)).min(count);
        for i in cursor..end {
            let developer = developers.get_unchecked(i);
            let amount = amounts.get_unchecked(i);
            developer.require_auth();
            Self::withdraw_developer_balance_inner(&env, developer, amount, None)?;
        }

        Ok((end, end == count))
    }

    /// Execute a batch of settlement operations and return one outcome per input.
    ///
    /// The entrypoint delegates to the settlement batch helper, which validates
    /// each input and reports the per-item outcome in the returned vector.
    pub fn batch_settle(
        env: Env,
        settlements: soroban_sdk::Vec<batch::SettleInput>,
    ) -> soroban_sdk::Vec<batch::SettleOutcome> {
        batch::batch_settle(&env, settlements)
    }

    /// Freeze a developer's withdrawals.
    ///
    /// Only the admin may call. Sets the developer's `FrozenDeveloper` flag
    /// to `true`, blocking `withdraw_developer_balance` for that developer.
    ///
    /// # Arguments
    /// * `caller` - Must be the admin; must authorize.
    /// * `developer` - The developer address to freeze.
    /// * `reason` - Opaque label emitted in the event for off-chain indexers.
    ///
    /// # Errors
    /// * [`SettlementError::FreezeUnauthorized`] â€” caller is not the admin.
    /// * [`SettlementError::DeveloperFrozen`] â€” developer is already frozen.
    pub fn freeze_developer(
        env: Env,
        caller: Address,
        developer: Address,
        reason: Symbol,
    ) -> Result<(), SettlementError> {
        freeze::freeze_developer(env, caller, developer, reason)
    }

    /// Unfreeze a developer's withdrawals.
    ///
    /// Only the admin may call.
    ///
    /// # Arguments
    /// * `caller` - Must be the admin; must authorize.
    /// * `developer` - The developer address to unfreeze.
    ///
    /// # Errors
    /// * [`SettlementError::FreezeUnauthorized`] â€” caller is not the admin.
    /// * [`SettlementError::DeveloperNotFrozen`] â€” developer is not frozen.
    pub fn unfreeze_developer(
        env: Env,
        caller: Address,
        developer: Address,
    ) -> Result<(), SettlementError> {
        freeze::unfreeze_developer(env, caller, developer)
    }

    /// Return `true` if the developer's withdrawals are currently frozen.
    ///
    /// Read-only; no auth required.
    pub fn is_developer_frozen(env: Env, developer: Address) -> bool {
        freeze::is_developer_frozen(env, developer)
    }

    pub fn set_price(
        env: Env,
        caller: Address,
        offering_id: soroban_sdk::String,
        price: soroban_sdk::String,
    ) {
        price_registry::set_price(&env, caller, offering_id, price);
    }

    pub fn remove_price(env: Env, caller: Address, offering_id: soroban_sdk::String) {
        price_registry::remove_price(&env, caller, offering_id);
    }

    pub fn get_price(env: Env, offering_id: soroban_sdk::String) -> Option<soroban_sdk::String> {
        price_registry::get_price(&env, offering_id)
    }

    // â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â” Internal helpers â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”â”

    fn require_supported_token(env: &Env, token: &Address) {
        let key = StorageKey::SupportedToken(token.clone());
        if !env.storage().persistent().has(&key) {
            env.panic_with_error(SettlementError::UnsupportedToken);
        }
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    fn add_supported_token_internal(env: &Env, caller: &Address, token: &Address) {
        let key = StorageKey::SupportedToken(token.clone());
        if !env.storage().persistent().has(&key) {
            env.storage().persistent().set(&key, &true);
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            events::emit_supported_token_added(env, caller, token);
        }
    }

    /// Abort with `Unauthorized` unless `caller` is the registered vault or admin.
    ///
    /// For details on the caller authorization model, dual-caller access controls,
    /// and security implications, see [`docs/ACCESS_CONTROL.md`](../../docs/ACCESS_CONTROL.md).
    fn require_authorized_caller(env: Env, caller: Address) {
        let vault = Self::get_vault(env.clone()).unwrap();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != vault && caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
    }

    /// Register `addr` in the paged persistent developer index.
    ///
    /// Uses a per-developer membership flag (`StorageKey::DeveloperMember`) as
    /// an O(1) duplicate guard, so no full-index scan is required.  If the
    /// developer is already registered this is a cheap no-op (one persistent
    /// read).
    ///
    /// New addresses are appended to the last page.  When the last page is
    /// full a new page is allocated and the `IndexPageCount` counter in
    /// instance storage is incremented by 1.  Each page therefore stays at a
    /// bounded, fixed size (`INDEX_PAGE_SIZE`), preventing any single storage
    /// entry from growing unboundedly.
    pub(crate) fn index_insert(env: &Env, addr: Address) {
        let member_key = StorageKey::DeveloperMember(addr.clone());
        if env.storage().persistent().has(&member_key) {
            // Already registered — O(1) early exit.
            return;
        }

        // Mark as registered.
        env.storage().persistent().set(&member_key, &true);
        env.storage().persistent().extend_ttl(
            &member_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        // Determine current page count (0 means no pages yet).
        let page_count: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::IndexPageCount)
            .unwrap_or(0u32);

        // Load the last page (or create the first one).
        let last_page_idx = if page_count == 0 { 0u32 } else { page_count - 1 };
        let last_page_key = StorageKey::IndexPage(last_page_idx);
        let mut page: Vec<Address> = env
            .storage()
            .persistent()
            .get(&last_page_key)
            .unwrap_or_else(|| Vec::new(env));

        if page.len() >= INDEX_PAGE_SIZE || (page_count == 0 && page.is_empty()) {
            if page_count > 0 && page.len() >= INDEX_PAGE_SIZE {
                // Current last page is full — start a new one.
                let new_page_idx = page_count;
                let new_page_key = StorageKey::IndexPage(new_page_idx);
                let mut new_page = Vec::new(env);
                new_page.push_back(addr);
                env.storage().persistent().set(&new_page_key, &new_page);
                env.storage().persistent().extend_ttl(
                    &new_page_key,
                    PERSISTENT_BUMP_THRESHOLD,
                    PERSISTENT_BUMP_AMOUNT,
                );
                env.storage()
                    .instance()
                    .set(&StorageKey::IndexPageCount, &(page_count + 1));
                return;
            }
            // page_count == 0: first developer ever — initialise page 0.
            page.push_back(addr);
            env.storage().persistent().set(&last_page_key, &page);
            env.storage().persistent().extend_ttl(
                &last_page_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            env.storage()
                .instance()
                .set(&StorageKey::IndexPageCount, &1u32);
        } else {
            // Append to the existing last page.
            page.push_back(addr);
            env.storage().persistent().set(&last_page_key, &page);
            env.storage().persistent().extend_ttl(
                &last_page_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            if page_count == 0 {
                env.storage()
                    .instance()
                    .set(&StorageKey::IndexPageCount, &1u32);
            }
        }
    }

    /// Iterate all paged index entries, calling `f` with each `Address`.
    ///
    /// Reads only the persistent pages that actually exist; instance storage
    /// is touched only for the single `IndexPageCount` read.
    pub(crate) fn iter_index<F: FnMut(Address)>(env: &Env, mut f: F) {
        let page_count: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::IndexPageCount)
            .unwrap_or(0u32);
        for p in 0..page_count {
            let key = StorageKey::IndexPage(p);
            let page: Vec<Address> = env
                .storage()
                .persistent()
                .get(&key)
                .unwrap_or_else(|| Vec::new(env));
            for addr in page.iter() {
                f(addr);
            }
        }
    }
}
#[cfg(test)]
mod test_freeze;
#[cfg(test)]
mod test_reentrancy;

/// #1135: cursor-based batch developer withdrawals.
#[cfg(test)]
mod test_batch_withdraw;
#[cfg(test)]
// Legacy suites targeting the pre-nonce payment API are intentionally not
// compiled; current authorization behavior is covered by contracts/tests.
#[cfg(test)]
mod test_admin_migration;
#[cfg(test)]
mod test_error_codes;
#[cfg(test)]
mod test_events;
#[cfg(test)]
mod test_invariant;
#[cfg(test)]
mod test_multi_asset;
#[cfg(test)]
mod test_overflow_safe_math;
#[cfg(test)]
mod test_record_deduction;
#[cfg(test)]
mod test_simulate_claim;
#[cfg(test)]
mod test_ttl_bump;
#[cfg(test)]
mod test_upgrade_timelock;
#[cfg(test)]
mod test_views;
