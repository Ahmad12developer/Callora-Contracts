#![no_std]

mod errors;
mod types;

#[cfg(test)]
mod test;
#[cfg(test)]
mod test_ttl_bump;

#[cfg(test)]
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

pub use errors::RefundError;
pub use types::*;

/// Minimum threshold of remaining ledgers before instance storage TTL is extended (~30 days).
pub const INSTANCE_BUMP_THRESHOLD: u32 = 17_280 * 30;

/// Number of ledgers to extend instance storage TTL by (~60 days).
pub const INSTANCE_BUMP_AMOUNT: u32 = 17_280 * 60;

/// Minimum threshold of remaining ledgers before persistent storage TTL is extended.
pub const PERSISTENT_BUMP_THRESHOLD: u32 = 50_000;

/// Number of ledgers to extend persistent storage TTL by.
pub const PERSISTENT_BUMP_AMOUNT: u32 = 50_000;

/// Maximum number of request IDs retained in each requester's index.
/// Older entries remain addressable through `get_refund_request`; the index is
/// bounded so a single requester cannot grow one persistent value forever.
pub const MAX_REQUESTER_REFUNDS: u32 = 100;

/// Maximum page size for requester refund index reads.
pub const MAX_REQUESTER_REFUNDS_PAGE_SIZE: u32 = 50;

#[contract]
pub struct RefundContract;

#[contractimpl]
impl RefundContract {
    /// Initialize the refund contract.
    ///
    /// # Arguments
    /// * `admin` - Address that will hold admin privileges; must authorize.
    /// * `fee_bps` - Fee rate in basis points (0-10000).
    /// * `min_refund_amount` - Minimum refund amount allowed (>= 0).
    ///
    /// # Errors
    /// * `AlreadyInitialized` - Contract already initialized.
    /// * `FeeTooHigh` - `fee_bps` exceeds 10000.
    /// * `InvalidAmount` - `min_refund_amount` is negative.
    pub fn init(
        env: Env,
        admin: Address,
        fee_bps: u32,
        min_refund_amount: i128,
    ) -> Result<(), RefundError> {
        admin.require_auth();

        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(RefundError::AlreadyInitialized);
        }
        if fee_bps > 10_000 {
            return Err(RefundError::FeeTooHigh);
        }
        if min_refund_amount < 0 {
            return Err(RefundError::InvalidAmount);
        }

        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::RefundCounter, &0u64);
        inst.set(&StorageKey::TotalRefunds, &0i128);
        inst.set(
            &StorageKey::RefundConfig,
            &RefundConfig {
                fee_bps,
                min_refund_amount,
            },
        );

        Self::emit_initialized(&env, &admin, fee_bps, min_refund_amount);

        Ok(())
    }

    /// Request a refund.
    ///
    /// # Arguments
    /// * `requester` - Address requesting the refund; must authorize.
    /// * `token` - Token contract address for the refund.
    /// * `amount` - Amount to refund; must be > 0 and >= min_refund_amount.
    /// * `reason` - Reason for the refund request.
    ///
    /// # Returns
    /// The unique request ID assigned to this refund request.
    ///
    /// # Errors
    /// * `NotInitialized` - Contract not initialized.
    /// * `InvalidAmount` - Amount is not positive.
    /// * `AmountTooLow` - Amount is below minimum refund amount.
    /// * `Overflow` - Refund counter would overflow.
    pub fn request_refund(
        env: Env,
        requester: Address,
        token: Address,
        amount: i128,
        reason: Symbol,
    ) -> Result<u64, RefundError> {
        requester.require_auth();
        Self::ensure_initialized(&env)?;
        Self::bump_instance_ttl(&env);

        if amount <= 0 {
            return Err(RefundError::InvalidAmount);
        }

        let config: RefundConfig = env
            .storage()
            .instance()
            .get(&StorageKey::RefundConfig)
            .ok_or(RefundError::NotInitialized)?;
        if amount < config.min_refund_amount {
            return Err(RefundError::AmountTooLow);
        }

        let counter: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::RefundCounter)
            .unwrap_or(0);
        let request_id = counter
            .checked_add(1)
            .ok_or(RefundError::Overflow)?;

        let request = RefundRequest {
            id: request_id,
            requester: requester.clone(),
            token: token.clone(),
            amount,
            reason: reason.clone(),
            status: RefundStatus::Pending,
            created_at: env.ledger().timestamp(),
            processed_at: None,
        };

        let key = StorageKey::PendingRefund(request_id);
        env.storage().persistent().set(&key, &request);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

        let requester_key = StorageKey::RequesterRefunds(requester.clone());
        let mut requester_refunds: soroban_sdk::Vec<u64> = env
            .storage()
            .persistent()
            .get(&requester_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if requester_refunds.len() >= MAX_REQUESTER_REFUNDS {
            requester_refunds.remove(0);
        }
        requester_refunds.push_back(request_id);
        env.storage().persistent().set(&requester_key, &requester_refunds);
        env.storage().persistent().extend_ttl(
            &requester_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );

        env.storage()
            .instance()
            .set(&StorageKey::RefundCounter, &request_id);

        Self::emit_refund_requested(&env, request_id, &requester, &token, amount, &reason);

        Ok(request_id)
    }

    /// Approve a pending refund request (admin only).
    ///
    /// # Arguments
    /// * `admin` - Admin address; must authorize and match stored admin.
    /// * `request_id` - ID of the refund request to approve.
    ///
    /// # Errors
    /// * `NotInitialized` - Contract not initialized.
    /// * `Unauthorized` - Caller is not the admin.
    /// * `NotFound` - Refund request not found.
    /// * `InvalidStatus` - Request is not in Pending status.
    pub fn approve_refund(
        env: Env,
        admin: Address,
        request_id: u64,
    ) -> Result<(), RefundError> {
        admin.require_auth();
        Self::ensure_initialized(&env)?;
        Self::bump_instance_ttl(&env);

        let stored_admin: Address = Self::get_admin_internal(&env)?;
        if admin != stored_admin {
            return Err(RefundError::Unauthorized);
        }

        let key = StorageKey::PendingRefund(request_id);
        let mut request: RefundRequest = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(RefundError::NotFound)?;

        if request.status != RefundStatus::Pending {
            return Err(RefundError::InvalidStatus);
        }

        request.status = RefundStatus::Approved;
        env.storage().persistent().set(&key, &request);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

        Self::emit_refund_processed(
            &env,
            request_id,
            &admin,
            request.amount,
            RefundStatus::Approved,
        );

        Ok(())
    }

    /// Reject a pending refund request (admin only).
    ///
    /// # Arguments
    /// * `admin` - Admin address; must authorize and match stored admin.
    /// * `request_id` - ID of the refund request to reject.
    ///
    /// # Errors
    /// * `NotInitialized` - Contract not initialized.
    /// * `Unauthorized` - Caller is not the admin.
    /// * `NotFound` - Refund request not found.
    /// * `InvalidStatus` - Request is not in Pending status.
    pub fn reject_refund(
        env: Env,
        admin: Address,
        request_id: u64,
    ) -> Result<(), RefundError> {
        admin.require_auth();
        Self::ensure_initialized(&env)?;
        Self::bump_instance_ttl(&env);

        let stored_admin: Address = Self::get_admin_internal(&env)?;
        if admin != stored_admin {
            return Err(RefundError::Unauthorized);
        }

        let key = StorageKey::PendingRefund(request_id);
        let mut request: RefundRequest = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(RefundError::NotFound)?;

        if request.status != RefundStatus::Pending {
            return Err(RefundError::InvalidStatus);
        }

        request.status = RefundStatus::Rejected;
        request.processed_at = Some(env.ledger().timestamp());
        env.storage().persistent().set(&key, &request);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

        Self::emit_refund_processed(
            &env,
            request_id,
            &admin,
            request.amount,
            RefundStatus::Rejected,
        );

        Ok(())
    }

    /// Process an approved refund request (admin only).
    ///
    /// This marks the refund as processed and updates the total refunds counter.
    ///
    /// # Arguments
    /// * `admin` - Admin address; must authorize and match stored admin.
    /// * `request_id` - ID of the refund request to process.
    ///
    /// # Errors
    /// * `NotInitialized` - Contract not initialized.
    /// * `Unauthorized` - Caller is not the admin.
    /// * `NotFound` - Refund request not found.
    /// * `InvalidStatus` - Request is not in Approved status.
    /// * `Overflow` - Total refunds counter would overflow.
    pub fn process_refund(
        env: Env,
        admin: Address,
        request_id: u64,
    ) -> Result<(), RefundError> {
        admin.require_auth();
        Self::ensure_initialized(&env)?;
        Self::bump_instance_ttl(&env);

        let stored_admin: Address = Self::get_admin_internal(&env)?;
        if admin != stored_admin {
            return Err(RefundError::Unauthorized);
        }

        let key = StorageKey::PendingRefund(request_id);
        let mut request: RefundRequest = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(RefundError::NotFound)?;

        if request.status != RefundStatus::Approved {
            return Err(RefundError::InvalidStatus);
        }

        request.status = RefundStatus::Processed;
        request.processed_at = Some(env.ledger().timestamp());
        env.storage().persistent().set(&key, &request);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

        let total: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::TotalRefunds)
            .unwrap_or(0);
        let new_total = total
            .checked_add(request.amount)
            .ok_or(RefundError::Overflow)?;
        env.storage()
            .instance()
            .set(&StorageKey::TotalRefunds, &new_total);

        Self::emit_refund_processed(
            &env,
            request_id,
            &admin,
            request.amount,
            RefundStatus::Processed,
        );

        Ok(())
    }

    /// Update refund configuration (admin only).
    ///
    /// # Arguments
    /// * `admin` - Admin address; must authorize and match stored admin.
    /// * `fee_bps` - New fee rate in basis points (0-10000).
    /// * `min_refund_amount` - New minimum refund amount (>= 0).
    ///
    /// # Errors
    /// * `NotInitialized` - Contract not initialized.
    /// * `Unauthorized` - Caller is not the admin.
    /// * `FeeTooHigh` - `fee_bps` exceeds 10000.
    /// * `InvalidAmount` - `min_refund_amount` is negative.
    pub fn update_config(
        env: Env,
        admin: Address,
        fee_bps: u32,
        min_refund_amount: i128,
    ) -> Result<(), RefundError> {
        admin.require_auth();
        Self::ensure_initialized(&env)?;
        Self::bump_instance_ttl(&env);

        let stored_admin: Address = Self::get_admin_internal(&env)?;
        if admin != stored_admin {
            return Err(RefundError::Unauthorized);
        }
        if fee_bps > 10_000 {
            return Err(RefundError::FeeTooHigh);
        }
        if min_refund_amount < 0 {
            return Err(RefundError::InvalidAmount);
        }

        env.storage().instance().set(
            &StorageKey::RefundConfig,
            &RefundConfig {
                fee_bps,
                min_refund_amount,
            },
        );

        Self::emit_config_updated(&env, &admin, fee_bps, min_refund_amount);

        Ok(())
    }

    /// Get the stored admin address.
    ///
    /// Bumps instance storage TTL on read to prevent premature archival.
    pub fn get_admin(env: Env) -> Result<Address, RefundError> {
        Self::bump_instance_ttl(&env);
        Self::get_admin_internal(&env)
    }

    /// Get the current refund configuration.
    ///
    /// Bumps instance storage TTL on read to prevent premature archival.
    pub fn get_config(env: Env) -> Result<RefundConfig, RefundError> {
        Self::bump_instance_ttl(&env);
        env.storage()
            .instance()
            .get(&StorageKey::RefundConfig)
            .ok_or(RefundError::NotInitialized)
    }

    /// Get a refund request by ID.
    ///
    /// Bumps instance and persistent storage TTL on read to prevent premature archival.
    pub fn get_refund_request(
        env: Env,
        request_id: u64,
    ) -> Result<RefundRequest, RefundError> {
        Self::bump_instance_ttl(&env);

        let key = StorageKey::PendingRefund(request_id);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
        }
        env.storage()
            .persistent()
            .get(&key)
            .ok_or(RefundError::NotFound)
    }

    /// Get total refunds processed.
    ///
    /// Bumps instance storage TTL on read to prevent premature archival.
    pub fn get_total_refunds(env: Env) -> Result<i128, RefundError> {
        Self::bump_instance_ttl(&env);
        Self::ensure_initialized(&env)?;
        Ok(env
            .storage()
            .instance()
            .get(&StorageKey::TotalRefunds)
            .unwrap_or(0))
    }

    /// Get the current refund counter (next request ID).
    ///
    /// Bumps instance storage TTL on read to prevent premature archival.
    pub fn get_refund_counter(env: Env) -> Result<u64, RefundError> {
        Self::bump_instance_ttl(&env);
        Self::ensure_initialized(&env)?;
        Ok(env
            .storage()
            .instance()
            .get(&StorageKey::RefundCounter)
            .unwrap_or(0))
    }

    /// Return a bounded page of this requester's refund IDs in creation order.
    ///
    /// `start` is a zero-based offset into the retained index. The index keeps
    /// at most [`MAX_REQUESTER_REFUNDS`] recent IDs; older requests remain
    /// available through [`Self::get_refund_request`]. Both the index and
    /// returned request records receive TTL bumps.
    pub fn get_refunds_by_requester(
        env: Env,
        requester: Address,
        start: u32,
        limit: u32,
    ) -> Result<soroban_sdk::Vec<u64>, RefundError> {
        Self::bump_instance_ttl(&env);
        Self::ensure_initialized(&env)?;

        let requester_key = StorageKey::RequesterRefunds(requester);
        let index: soroban_sdk::Vec<u64> = env
            .storage()
            .persistent()
            .get(&requester_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if env.storage().persistent().has(&requester_key) {
            env.storage().persistent().extend_ttl(
                &requester_key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }

        let effective_limit = limit.min(MAX_REQUESTER_REFUNDS_PAGE_SIZE);
        let mut page = soroban_sdk::Vec::new(&env);
        let end = start
            .saturating_add(effective_limit)
            .min(index.len());
        for offset in start..end {
            let request_id = index.get(offset).unwrap();
            let key = StorageKey::PendingRefund(request_id);
            if env.storage().persistent().has(&key) {
                env.storage().persistent().extend_ttl(
                    &key,
                    PERSISTENT_BUMP_THRESHOLD,
                    PERSISTENT_BUMP_AMOUNT,
                );
                page.push_back(request_id);
            }
        }
        Ok(page)
    }

    /// Internal helper to get admin without TTL bump (for internal use).
    fn get_admin_internal(env: &Env) -> Result<Address, RefundError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(RefundError::NotInitialized)
    }

    /// Internal helper to ensure contract is initialized.
    fn ensure_initialized(env: &Env) -> Result<(), RefundError> {
        if !env.storage().instance().has(&StorageKey::Admin) {
            return Err(RefundError::NotInitialized);
        }
        Ok(())
    }

    /// Bump instance storage TTL to prevent premature archival on hot read paths.
    pub(crate) fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Publish the `"initialized"` event.
    ///
    /// **What**: Announces that the contract has just been configured with its
    /// first admin and fee policy.
    /// **How**: Called exactly once, from [`Self::init`], immediately after the
    /// admin address and [`RefundConfig`] are persisted to instance storage.
    /// **Why**: Lets off-chain indexers detect contract bootstrap and cache the
    /// initial fee/minimum-amount policy without polling storage.
    ///
    /// Topic: `("initialized",)`. Data: [`InitializedEvent`].
    fn emit_initialized(env: &Env, admin: &Address, fee_bps: u32, min_refund_amount: i128) {
        env.events().publish(
            (Symbol::new(env, "initialized"),),
            InitializedEvent {
                admin: admin.clone(),
                fee_bps,
                min_refund_amount,
            },
        );
    }

    /// Publish the `"refund_requested"` event.
    ///
    /// **What**: Records that a new refund request entered the `Pending` state.
    /// **How**: Called from [`Self::request_refund`] after the request is
    /// assigned an ID and written to persistent storage.
    /// **Why**: Gives indexers and admin tooling a durable feed of pending
    /// requests to drive an approval queue, without re-scanning storage.
    ///
    /// Topic: `("refund_requested",)`. Data: [`RefundRequestedEvent`].
    fn emit_refund_requested(
        env: &Env,
        request_id: u64,
        requester: &Address,
        token: &Address,
        amount: i128,
        reason: &Symbol,
    ) {
        env.events().publish(
            (Symbol::new(env, "refund_requested"),),
            RefundRequestedEvent {
                request_id,
                requester: requester.clone(),
                token: token.clone(),
                amount,
                reason: reason.clone(),
            },
        );
    }

    /// Publish the `"refund_processed"` event.
    ///
    /// **What**: Records a status transition on an existing refund request.
    /// **How**: Called from three admin-only entrypoints, once each, with the
    /// terminal or intermediate status the request just moved to:
    /// [`Self::approve_refund`] (`status = Approved`),
    /// [`Self::reject_refund`] (`status = Rejected`), and
    /// [`Self::process_refund`] (`status = Processed`).
    /// **Why**: A single topic covering all three transitions lets indexers
    /// reconstruct the full request lifecycle by filtering on `request_id` and
    /// reading `status` from the payload, instead of subscribing to three
    /// separate topics.
    ///
    /// Topic: `("refund_processed",)`. Data: [`RefundProcessedEvent`].
    fn emit_refund_processed(
        env: &Env,
        request_id: u64,
        processor: &Address,
        amount: i128,
        status: RefundStatus,
    ) {
        env.events().publish(
            (Symbol::new(env, "refund_processed"),),
            RefundProcessedEvent {
                request_id,
                processor: processor.clone(),
                amount,
                status,
            },
        );
    }

    /// Publish the `"config_updated"` event.
    ///
    /// **What**: Announces a change to the contract-wide fee rate and/or
    /// minimum refund amount.
    /// **How**: Called from [`Self::update_config`] after the new
    /// [`RefundConfig`] passes validation and is persisted.
    /// **Why**: Lets clients quoting refund amounts invalidate cached fee
    /// assumptions as soon as the admin changes policy, rather than after
    /// their next storage read.
    ///
    /// Topic: `("config_updated",)`. Data: [`RefundConfigUpdatedEvent`].
    fn emit_config_updated(env: &Env, admin: &Address, fee_bps: u32, min_refund_amount: i128) {
        env.events().publish(
            (Symbol::new(env, "config_updated"),),
            RefundConfigUpdatedEvent {
                admin: admin.clone(),
                fee_bps,
                min_refund_amount,
            },
        );
    }
}

/// Payload for the `"initialized"` event.
///
/// Published once by [`RefundContract::emit_initialized`] when the contract
/// is first configured.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitializedEvent {
    /// Address granted admin privileges by [`RefundContract::init`].
    pub admin: Address,
    /// Fee rate in basis points (0-10000) applied to future refunds.
    pub fee_bps: u32,
    /// Minimum refund amount accepted by [`RefundContract::request_refund`].
    pub min_refund_amount: i128,
}

/// Payload for the `"refund_requested"` event.
///
/// Published by [`RefundContract::emit_refund_requested`] when a new refund
/// request is created in the `Pending` state.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundRequestedEvent {
    /// ID assigned to this request; use it to correlate with the eventual
    /// `"refund_processed"` event.
    pub request_id: u64,
    /// Address that submitted the refund request.
    pub requester: Address,
    /// Token contract address the refund would be paid in.
    pub token: Address,
    /// Requested refund amount, in the token's smallest unit.
    pub amount: i128,
    /// Caller-supplied reason code for the request.
    pub reason: Symbol,
}

/// Payload for the `"refund_processed"` event.
///
/// Published by [`RefundContract::emit_refund_processed`] on every admin
/// decision on a refund request. `status` distinguishes which transition
/// occurred — see [`RefundStatus`].
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundProcessedEvent {
    /// ID of the refund request this decision applies to.
    pub request_id: u64,
    /// Admin address that made the decision.
    pub processor: Address,
    /// Amount recorded on the original request at decision time.
    pub amount: i128,
    /// New status of the request: `Approved`, `Rejected`, or `Processed`.
    pub status: RefundStatus,
}

/// Payload for the `"config_updated"` event.
///
/// Published by [`RefundContract::emit_config_updated`] whenever the admin
/// changes the fee policy via [`RefundContract::update_config`].
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundConfigUpdatedEvent {
    /// Admin address that made the change.
    pub admin: Address,
    /// New fee rate in basis points (0-10000).
    pub fee_bps: u32,
    /// New minimum refund amount.
    pub min_refund_amount: i128,
}
