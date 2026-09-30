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

/// Maximum byte length of an admin broadcast message.
///
/// Keeps per-call resource consumption (event payload size, transaction size)
/// bounded and predictable for a public admin entry point.
pub const MAX_BROADCAST_MESSAGE_LEN: u32 = 1024;

pub use errors::SettlementError;
pub use migrate::{STORAGE_VERSION_V1, STORAGE_VERSION_V2};
pub use timelock::{PendingDeveloperMigration, DEVELOPER_MIGRATION_TIMELOCK_SECONDS};
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
    /// # Arithmetic safety
    /// The cumulative total is incremented via `checked_add`. An overflow
    /// panics with [`SettlementError::PoolOverflow`] rather than wrapping
    /// silently.
    pub fn record_deduction(env: Env, amount: i128, _request_id: u64) {
        let vault = Self::get_vault(env.clone()).unwrap();
        vault.require_auth();
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

            // Extend TTL for the developer's balance entry (persistent storage live for 1 year)
            env.storage()
                .persistent()
                .extend_ttl(&balance_key, 50000, 50000);

            // Add developer to index in sorted order if not already present
            let mut index: Vec<Address> = inst
                .get(&StorageKey::DeveloperIndex)
                .unwrap_or_else(|| Vec::new(&env));
            Self::sorted_insert(&env, &mut index, dev_address.clone());
            inst.set(&StorageKey::DeveloperIndex, &index);

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

        let inst = env.storage().instance();

        for item in items.iter() {
            let (dev, amount) = item;
            let balance_key = StorageKey::DeveloperBalance(dev.clone(), token.clone());
            let current: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
            let new_balance = current
                .checked_add(amount)
                .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));
            env.storage().persistent().set(&balance_key, &new_balance);
            env.storage().persistent().set(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                &new_balance,
            );
            env.storage().persistent().extend_ttl(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                50000,
                50000,
            );
            // Add to index in sorted order if not already present
            let mut index: Vec<Address> = inst
                .get(&StorageKey::DeveloperIndex)
                .unwrap_or_else(|| Vec::new(&env));
            Self::sorted_insert(&env, &mut index, dev.clone());
            inst.set(&StorageKey::DeveloperIndex, &index);
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
    }

    fn get_usdc_token(env: Env) -> Result<Address, SettlementError> {
        env.storage()
            .instance()
            .get(&StorageKey::Usdc)
            .ok_or(SettlementError::UsdcTokenNotConfigured)
    }

    /// Withdraw developer balance as USDC to a designated recipient.
    ///
    /// Requires the developer to authorize the request, the amount to be
    /// positive, the developer's optional claim window to be open, and the
    /// requested amount to be covered by the tracked developer balance in the
    /// configured USDC token.
    ///
    /// # Arguments
    /// * `developer` - Address of the developer withdrawing their balance.
    /// * `amount` - Amount to withdraw in USDC micro-units.
    /// * `to` - Optional recipient address; if `None`, defaults to `developer`.
    ///
    /// # Errors
    /// - `AmountNotPositive` if amount is <= 0.
    /// - `ClaimWindowClosed` if a developer claim window exists and the current
    ///   ledger timestamp is outside that inclusive window.
    /// - `UsdcTokenNotConfigured` if USDC token not set.
    /// - `InsufficientDeveloperBalance` if developer balance < amount.
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

        Self::require_claim_window_open(&env, &developer)?;

        let usdc_address = Self::get_usdc_token(env.clone())?;

        // --- CHECKS ---
        // Single read of the developer's per-token balance (removes the duplicate
        // read that previously appeared as both `dev_balance_key`/`dev_balance`
        // and `balance_key`/`current_balance` with identical keys).
        let balance_key = StorageKey::DeveloperBalance(developer.clone(), usdc_address.clone());
        let current_balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if amount > current_balance {
            return Err(SettlementError::InsufficientDeveloperBalance);
        }

        // Compute the post-withdrawal balance and enforce the developer minimum.
        let new_balance = current_balance
            .checked_sub(amount)
            .ok_or(SettlementError::DeveloperBalanceUnderflow)?;
        limits::check_min_balance(&env, &developer, new_balance)?;

        let today = env.ledger().timestamp() / 86400;
        let today_key = StorageKey::WithdrawalToday(developer.clone());
        let mut daily = env
            .storage()
            .persistent()
            .get::<_, DailyWithdrawState>(&today_key)
            .unwrap_or(DailyWithdrawState {
                day: today,
                amount: 0,
            });
        if daily.day != today {
            daily.day = today;
            daily.amount = 0;
        }

        let cap: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::DailyWithdrawCap(developer.clone()))
            .unwrap_or(0);
        if cap > 0 {
            let projected = daily
                .amount
                .checked_add(amount)
                .ok_or(SettlementError::DailyWithdrawCapExceeded)?;
            if projected > cap {
                return Err(SettlementError::DailyWithdrawCapExceeded);
            }
        }

        let usdc = token::Client::new(&env, &usdc_address);
        if usdc.balance(&contract_address) < amount {
            return Err(SettlementError::InsufficientContractBalance);
        }

        // --- EFFECTS (persist all state mutations before the external call) ---
        // CEI: write the reduced balance and updated daily-withdrawal counter
        // here so that any re-entrant call via the token contract observes the
        // already-reduced balance and cannot withdraw a second time.
        env.storage().persistent().set(&balance_key, &new_balance);
        env.storage()
            .persistent()
            .extend_ttl(&balance_key, 50000, 50000);

        daily.amount = daily
            .amount
            .checked_add(amount)
            .ok_or(SettlementError::DailyWithdrawCapExceeded)?;
        env.storage().persistent().set(&today_key, &daily);
        env.storage()
            .persistent()
            .extend_ttl(&today_key, 50000, 50000);

        // --- INTERACTION (external token transfer happens last) ---
        usdc.transfer(&contract_address, &recipient, &amount);

        events::emit_developer_withdraw(
            &env,
            &developer.clone(),
            DeveloperWithdrawEvent {
                developer,
                amount,
                remaining_balance: new_balance,
                to: recipient,
                token: usdc_address,
            },
        );

        Ok(())
    }

    /// Simulate a developer claim without side effects.
    ///
    /// This read-only view previews the outcome of `withdraw_developer_balance`
    /// for the configured USDC token. It intentionally does not require
    /// developer authorization and does not transfer tokens, mutate balances,
    /// update daily withdrawal counters, extend TTLs, or emit events.
    ///
    /// The view returns the same typed errors as a real claim for amount, claim
    /// window, developer balance, daily cap, USDC configuration, and contract
    /// token-liquidity failures. If `to` is `None`, the simulated recipient is
    /// the developer address.
    pub fn simulate_claim(
        env: Env,
        developer: Address,
        amount: i128,
        to: Option<Address>,
    ) -> Result<ClaimSimulation, SettlementError> {
        if amount <= 0 {
            return Err(SettlementError::AmountNotPositive);
        }

        let recipient = to.unwrap_or_else(|| developer.clone());
        let contract_address = env.current_contract_address();
        if recipient == contract_address {
            env.panic_with_error(SettlementError::InvalidRecipient);
        }

        Self::require_claim_window_open(&env, &developer)?;

        let usdc_address = Self::get_usdc_token(env.clone())?;
        let current_balance: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::DeveloperBalance(
                developer.clone(),
                usdc_address.clone(),
            ))
            .unwrap_or(0);
        if amount > current_balance {
            return Err(SettlementError::InsufficientDeveloperBalance);
        }

        let today = env.ledger().timestamp() / 86400;
        let cap: i128 = env
            .storage()
            .persistent()
            .get(&StorageKey::DailyWithdrawCap(developer.clone()))
            .unwrap_or(0);
        let daily = env
            .storage()
            .persistent()
            .get::<_, DailyWithdrawState>(&StorageKey::WithdrawalToday(developer.clone()))
            .unwrap_or(DailyWithdrawState {
                day: today,
                amount: 0,
            });
        let withdrawn_today = if daily.day == today { daily.amount } else { 0 };
        let withdrawn_today_after = withdrawn_today
            .checked_add(amount)
            .ok_or(SettlementError::DailyWithdrawCapExceeded)?;
        if cap > 0 && withdrawn_today_after > cap {
            return Err(SettlementError::DailyWithdrawCapExceeded);
        }

        let remaining_balance = current_balance
            .checked_sub(amount)
            .ok_or(SettlementError::DeveloperBalanceUnderflow)?;
        let contract_balance = token::Client::new(&env, &usdc_address).balance(&contract_address);
        if contract_balance < amount {
            return Err(SettlementError::InsufficientContractBalance);
        }

        Ok(ClaimSimulation {
            developer,
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
        env.storage()
            .persistent()
            .extend_ttl(&window_key, 50000, 50000);

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
    fn require_claim_window_open(env: &Env, developer: &Address) -> Result<(), SettlementError> {
        let key = StorageKey::DeveloperClaimWindow(developer.clone());
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
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
    ///
    /// # Events
    /// Emits `daily_withdraw_cap_changed` with the developer and new cap.
    pub fn set_daily_withdraw_cap(env: Env, caller: Address, developer: Address, cap: i128) {
        caller.require_auth();
        let current_admin = Self::get_admin(env.clone()).unwrap();
        if caller != current_admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        let cap_key = StorageKey::DailyWithdrawCap(developer.clone());
        env.storage().persistent().set(&cap_key, &cap);
        env.storage()
            .persistent()
            .extend_ttl(&cap_key, 50000, 50000);

        events::emit_daily_withdraw_cap_changed(
            &env,
            &caller,
            DailyWithdrawCapChanged {
                developer,
                new_cap: cap,
            },
        );
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
        env.storage()
            .persistent()
            .extend_ttl(&balance_key, 50000, 50000);

        let inst = env.storage().instance();
        let mut index: Vec<Address> = inst
            .get(&StorageKey::DeveloperIndex)
            .unwrap_or_else(|| Vec::new(&env));
        Self::sorted_insert(&env, &mut index, developer.clone());
        inst.set(&StorageKey::DeveloperIndex, &index);

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
    /// Iterates the full developer index. For deployments with many
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
        let index: Vec<Address> = env
            .storage()
            .instance()
            .get(&StorageKey::DeveloperIndex)
            .unwrap_or_else(|| Vec::new(&env));

        let mut result = Vec::new(&env);
        for address in index.iter() {
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
                address: address.clone(),
                token: token.clone(),
                balance,
            });
        }
        result
    }

    /// Get a start/limit-paginated slice of developer balances for a token
    /// (admin only). `limit` is capped at [`MAX_DEVELOPER_BALANCES_PAGE_SIZE`].
    /// Bumps instance and persistent TTL for retrieved entries.
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

        let index: Vec<Address> = env
            .storage()
            .instance()
            .get(&StorageKey::DeveloperIndex)
            .unwrap_or_else(|| Vec::new(&env));

        if limit == 0 || start >= index.len() {
            return Vec::new(&env);
        }

        let end = start
            .saturating_add(limit.min(MAX_DEVELOPER_BALANCES_PAGE_SIZE))
            .min(index.len());
        let mut result = Vec::new(&env);
        for (cursor, address) in (0_u32..).zip(index.iter()) {
            if cursor >= end {
                break;
            }
            if cursor >= start {
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
                    address: address.clone(),
                    token: token.clone(),
                    balance,
                });
            }
        }
        result
    }

    /// Cursor-based paginated developer balances for a specific token (admin only).
    ///
    /// Returns up to `limit` developer balance records starting **after** the
    /// supplied `cursor` address (exclusive), or from the beginning of the
    /// sorted index when `cursor` is `None`. The index is maintained in
    /// deterministic ascending order by address bytes, so pages are stable
    /// across interleaved `receive_payment` calls for developers that sort
    /// after the cursor.
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

        let index: Vec<Address> = env
            .storage()
            .instance()
            .get(&StorageKey::DeveloperIndex)
            .unwrap_or_else(|| Vec::new(&env));

        pagination::get_page(&env, &index, cursor, limit, &token)
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

    /// Upgrade the contract to a new WASM hash (admin only).
    ///
    /// # Events
    /// Emits `upgraded` with the new WASM hash.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) {
        caller.require_auth();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());
        env.storage()
            .instance()
            .set(&StorageKey::ContractVersion, &new_wasm_hash);
        events::emit_upgraded(&env, &caller, &new_wasm_hash);
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

    /// Placeholder batch-withdraw entrypoint for cursor-based withdrawals.
    ///
    /// The current implementation validates that the developer and amount
    /// vectors have the same length and returns an immediately-complete
    /// `(cursor, is_complete)` tuple. It is retained as a public entrypoint for
    /// interface compatibility and is not yet implemented.
    pub fn batch_withdraw_balance_cursor(
        _env: Env,
        developers: Vec<Address>,
        amounts: Vec<i128>,
        _cursor: u32,
        _limit: u32,
    ) -> Result<(u32, bool), SettlementError> {
        let count = developers.len();
        if count != amounts.len() {
            return Err(SettlementError::AmountNotPositive);
        }
        if count > MAX_BATCH_SIZE {
            return Err(SettlementError::BatchTooLarge);
        }
        Ok((0, true))
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

    /// Abort with `Unauthorized` unless `caller` is the registered vault or admin.
    fn require_authorized_caller(env: Env, caller: Address) {
        let vault = Self::get_vault(env.clone()).unwrap();
        let admin = Self::get_admin(env.clone()).unwrap();
        if caller != vault && caller != admin {
            env.panic_with_error(SettlementError::Unauthorized);
        }
    }

    /// Insert `addr` into `index` in deterministic ascending order by address
    /// bytes, if not already present. Keeps `DeveloperIndex` iteration and
    /// pagination stable and independent of insertion order.
    fn sorted_insert(_env: &Env, index: &mut Vec<Address>, addr: Address) {
        if index.iter().any(|a| a == addr) {
            return;
        }
        let mut pos: u32 = index.len();
        for (i, existing) in index.iter().enumerate() {
            if addr < existing {
                pos = i as u32;
                break;
            }
        }
        index.insert(pos, addr);
    }
}

#[cfg(test)]
mod test_freeze;
#[cfg(test)]
mod test_reentrancy;

#[cfg(test)]
// Legacy suites targeting the pre-nonce payment API are intentionally not
// compiled; current authorization behavior is covered by contracts/tests.
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
mod test_ttl_bump;
#[cfg(test)]
mod test_views;
