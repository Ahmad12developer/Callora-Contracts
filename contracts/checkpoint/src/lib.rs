#![no_std]

#[cfg(test)]
extern crate std;

pub mod errors;
pub mod events;

use callora_storage_migration::StorageMigrationValidator;
use errors::CheckpointError;
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env, Symbol, Vec};

/// Storage-layout version recorded by [`StorageMigrationValidator`] for this
/// contract. Bumped whenever the on-ledger schema changes so that the
/// pre-upgrade validation gate can enforce ordered, single-step migrations.
const STORAGE_MIGRATION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of checkpoint records that can be created in a single
/// [`CalloraCheckpoint::batch_create_checkpoints`] call.
pub const MAX_BATCH_SIZE: u32 = 50;

/// Maximum number of checkpoint records returned per page from
/// [`CalloraCheckpoint::get_checkpoints_range`].
pub const MAX_PAGE_SIZE: u32 = 100;

/// TTL bump constants for persistent storage archival risk mitigation.
/// Soroban archives ledger entries after ~7 days (631 ledgers) of inactivity.
///
/// - `BUMP_AMOUNT`: extend TTL by 3 110 400 ledgers (approx 6 months).
/// - `LIFETIME_THRESHOLD`: minimum TTL before triggering a bump (approx 1 day).
pub const BUMP_AMOUNT: u32 = 3_110_400;
pub const LIFETIME_THRESHOLD: u32 = 17_280;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Persistent and instance storage keys for the checkpoint contract.
///
/// Each variant maps a logical key name to the underlying Soroban storage
/// key, avoiding accidental key collisions with raw strings.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    /// Instance: current admin [`Address`].
    Admin,
    /// Instance: pending admin [`Address`] for two-step rotation.
    PendingAdmin,
    /// Instance: next sequential checkpoint ID to assign (`u64`).
    NextCheckpointId,
    /// Persistent: an individual checkpoint record indexed by `u64` ID.
    Checkpoint(u64),
    /// Instance: cached count of total checkpoints (`u64`), kept in
    /// sync with `NextCheckpointId` for efficient `get_checkpoint_count`.
    CheckpointCount,
    /// Persistent: ordered list of checkpoint IDs for a given subject [`Address`].
    ///
    /// Maintained as an append-only `Vec<u64>` in persistent storage so that
    /// [`CalloraCheckpoint::get_checkpoints_for_subject`] can return a
    /// subject's history in creation order without scanning the full range.
    SubjectIndex(Address),
}

/// A single immutable audit checkpoint recording a balance snapshot at a point in time.
///
/// Once written, a [`CheckpointRecord`] is never updated. It provides an
/// append-only audit trail suitable for compliance, reconciliation, and
/// dispute resolution.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointRecord {
    /// Globally-unique sequential checkpoint identifier.
    pub id: u64,
    /// The address whose balance is being snapshotted (e.g., a developer).
    pub subject: Address,
    /// The token contract address this balance is denominated in (e.g., USDC).
    pub token: Address,
    /// The snapshotted balance in token base units.
    pub balance: i128,
    /// Ledger timestamp (seconds) at which this checkpoint was created.
    pub timestamp: u64,
    /// Free-form metadata tag attached by the admin (e.g.,
    /// "monthly-close", "pre-migration"). Soroban
    /// [`Symbol`] values are protocol-limited to 32 characters.
    pub metadata: Symbol,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CalloraCheckpoint;

#[contractimpl]
impl CalloraCheckpoint {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialize the checkpoint contract with an admin address.
    ///
    /// Can only be called once. Subsequent calls return
    /// [`CheckpointError::AlreadyInitialized`].
    ///
    /// # Parameters
    /// * `env` - Soroban environment.
    /// * `admin` - Address permitted to call admin-only entrypoints.
    ///
    /// # Errors
    /// * [`CheckpointError::AlreadyInitialized`] -- `init` was called more than once.
    ///
    /// # Events
    /// Emits `init` with `admin` as topic and `()` as data.
    pub fn init(env: Env, admin: Address) -> Result<(), CheckpointError> {
        admin.require_auth();
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(CheckpointError::AlreadyInitialized);
        }

        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &admin);
        inst.set(&StorageKey::NextCheckpointId, &0u64);
        inst.set(&StorageKey::CheckpointCount, &0u64);

        env.events().publish((events::event_init(&env), admin), ());

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin helpers (internal)
    // -----------------------------------------------------------------------

    /// Read the current admin address from instance storage.
    ///
    /// Returns [`CheckpointError::NotInitialized`] when no admin has been set
    /// (i.e., `init` was never called).
    fn admin(env: &Env) -> Result<Address, CheckpointError> {
        env.storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(CheckpointError::NotInitialized)
    }

    /// Verify that `caller` is the current admin.
    ///
    /// Consumes the caller's auth via `require_auth`, then checks instance
    /// storage for the admin address. Returns [`CheckpointError::Unauthorized`]
    /// when the caller does not match.
    fn require_admin(env: &Env, caller: &Address) -> Result<(), CheckpointError> {
        caller.require_auth();
        let admin = Self::admin(env)?;
        if caller != &admin {
            return Err(CheckpointError::Unauthorized);
        }
        Ok(())
    }

    /// Atomically increment the sequential checkpoint ID counter and return
    /// the value that should be assigned to the newly created checkpoint.
    ///
    /// Uses checked arithmetic. Always updates both `NextCheckpointId` and
    /// `CheckpointCount` to keep them in sync.
    fn next_checkpoint_id(env: &Env) -> Result<u64, CheckpointError> {
        let current: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::NextCheckpointId)
            .unwrap_or(0);

        let next = current.checked_add(1).ok_or(CheckpointError::Overflow)?;

        env.storage()
            .instance()
            .set(&StorageKey::NextCheckpointId, &next);
        env.storage()
            .instance()
            .set(&StorageKey::CheckpointCount, &next);

        Ok(next)
    }

    /// Append `checkpoint_id` to the persistent per-subject index for `subject`.
    ///
    /// Creates the index list on first use. After writing, the entry's TTL is
    /// extended to [`BUMP_AMOUNT`] ledgers so the index survives as long as
    /// individual checkpoint records.
    fn append_to_subject_index(env: &Env, subject: &Address, checkpoint_id: u64) {
        let key = StorageKey::SubjectIndex(subject.clone());
        let mut ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(env));
        ids.push_back(checkpoint_id);
        env.storage().persistent().set(&key, &ids);
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }

    // -----------------------------------------------------------------------
    // Admin views
    // -----------------------------------------------------------------------

    /// Return the current admin address.
    ///
    /// # Errors
    /// * [`CheckpointError::NotInitialized`] -- contract was never initialized.
    pub fn get_admin(env: Env) -> Result<Address, CheckpointError> {
        Self::admin(&env)
    }

    /// Return the pending admin address for a two-step admin rotation, or
    /// `None` if no transfer is in progress.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::PendingAdmin)
    }

    // -----------------------------------------------------------------------
    // Two-step admin rotation
    // -----------------------------------------------------------------------

    /// Nominate a new admin. Only the current admin may call.
    ///
    /// The nominee must call [`CalloraCheckpoint::accept_admin`] to complete
    /// the transfer. Until then the current admin retains full authority.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_admin` -- Address of the proposed new admin.
    ///
    /// # Errors
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    ///
    /// # Events
    /// Emits `admin_nominated` with `(current_admin, new_admin)`.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), CheckpointError> {
        Self::require_admin(&env, &caller)?;

        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);

        env.events().publish(
            (
                events::event_admin_nominated(&env),
                Self::admin(&env)?,
                new_admin.clone(),
            ),
            new_admin,
        );

        Ok(())
    }

    /// Complete a pending admin transfer. Must be called by the nominated admin.
    ///
    /// # Errors
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * Panics with "no admin transfer pending" -- no nomination is in progress.
    /// * Panics with "unauthorized: caller is not pending admin" -- wrong caller.
    ///
    /// # Events
    /// Emits `admin_accepted` with `(old_admin, new_admin)`.
    pub fn accept_admin(env: Env, caller: Address) -> Result<(), CheckpointError> {
        caller.require_auth();

        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(CheckpointError::NoAdminTransferPending));

        if caller != pending {
            env.panic_with_error(CheckpointError::Unauthorized);
        }

        let old_admin = Self::admin(&env)?;
        let inst = env.storage().instance();
        inst.set(&StorageKey::Admin, &pending);
        inst.remove(&StorageKey::PendingAdmin);

        env.events().publish(
            (
                events::event_admin_accepted(&env),
                old_admin,
                pending.clone(),
            ),
            pending,
        );

        Ok(())
    }

    /// Cancel a pending admin transfer. Only the current admin may call.
    ///
    /// # Errors
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * Panics with "no admin transfer pending" -- no nomination is in progress.
    ///
    /// # Events
    /// Emits `admin_cancelled` with the cancelled pending admin.
    pub fn cancel_admin_transfer(env: Env, caller: Address) -> Result<(), CheckpointError> {
        Self::require_admin(&env, &caller)?;

        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .unwrap_or_else(|| env.panic_with_error(CheckpointError::NoAdminTransferPending));

        env.storage().instance().remove(&StorageKey::PendingAdmin);

        env.events()
            .publish((events::event_admin_cancelled(&env), caller, pending), ());

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Checkpoint creation
    // -----------------------------------------------------------------------

    /// Create a single immutable audit checkpoint recording a balance snapshot.
    ///
    /// The checkpoint is assigned a unique sequential ID, written to persistent
    /// storage with a 6-month TTL, and emitted as an event. Once written,
    /// checkpoints are **never updated** -- the log is append-only.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `subject` -- The address whose balance is being snapshotted.
    /// * `token` -- The token contract address (e.g., USDC).
    /// * `balance` -- The snapshotted balance in token base units. Must be >= 0.
    /// * `metadata` -- A free-form tag (e.g., "monthly", "audit-q3").
    ///   Soroban [`Symbol`] values are protocol-limited to 32 characters.
    ///
    /// # Returns
    /// The `u64` ID assigned to the newly created checkpoint.
    ///
    /// # Errors
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * [`CheckpointError::AmountNegative`] -- balance is negative.
    /// * [`CheckpointError::Overflow`] -- ID counter overflowed `u64`.
    ///
    /// # Events
    /// Emits `checkpoint_created` with `subject` as topic and the full
    /// [`CheckpointRecord`] as data.
    pub fn create_checkpoint(
        env: Env,
        caller: Address,
        subject: Address,
        token: Address,
        balance: i128,
        metadata: Symbol,
    ) -> Result<u64, CheckpointError> {
        Self::require_admin(&env, &caller)?;

        if balance < 0 {
            return Err(CheckpointError::AmountNegative);
        }

        let id = Self::next_checkpoint_id(&env)?;
        let timestamp = env.ledger().timestamp();

        let record = CheckpointRecord {
            id,
            subject: subject.clone(),
            token: token.clone(),
            balance,
            timestamp,
            metadata: metadata.clone(),
        };

        let key = StorageKey::Checkpoint(id);
        env.storage().persistent().set(&key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);

        // Maintain the per-subject index so get_checkpoints_for_subject can
        // return this subject's history without a full-range scan.
        Self::append_to_subject_index(&env, &subject, id);

        env.events()
            .publish((events::event_checkpoint_created(&env), subject), record);

        Ok(id)
    }

    /// Create multiple immutable audit checkpoints in a single atomic transaction.
    ///
    /// All validation runs **before** any checkpoint is written -- a failure on any
    /// item leaves the contract state unchanged.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `items` -- Vector of `(subject, token, balance, metadata)` tuples.
    ///   1 to [`MAX_BATCH_SIZE`] entries.
    ///
    /// # Returns
    /// A vector of `u64` checkpoint IDs in the same order as `items`.
    ///
    /// # Errors
    /// * [`CheckpointError::BatchEmpty`] -- `items` is empty.
    /// * [`CheckpointError::BatchTooLarge`] -- `items` exceeds [`MAX_BATCH_SIZE`].
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * [`CheckpointError::AmountNegative`] -- any balance is negative.
    /// * [`CheckpointError::Overflow`] -- ID counter overflowed `u64`.
    ///
    /// # Events
    /// Emits one `checkpoint_created` event per item.
    pub fn batch_create_checkpoints(
        env: Env,
        caller: Address,
        items: Vec<(Address, Address, i128, Symbol)>,
    ) -> Result<Vec<u64>, CheckpointError> {
        Self::require_admin(&env, &caller)?;

        let n = items.len();
        if n == 0 {
            return Err(CheckpointError::BatchEmpty);
        }
        if n > MAX_BATCH_SIZE {
            return Err(CheckpointError::BatchTooLarge);
        }

        // Validate all items before touching state.
        for item in items.iter() {
            let (_, _, balance, _metadata) = item;
            if balance < 0 {
                return Err(CheckpointError::AmountNegative);
            }
        }

        let mut ids: Vec<u64> = Vec::new(&env);
        let timestamp = env.ledger().timestamp();

        for item in items.iter() {
            let (subject, token, balance, metadata) = item;
            let id = Self::next_checkpoint_id(&env)?;

            let record = CheckpointRecord {
                id,
                subject: subject.clone(),
                token: token.clone(),
                balance,
                timestamp,
                metadata: metadata.clone(),
            };

            let key = StorageKey::Checkpoint(id);
            env.storage().persistent().set(&key, &record);
            env.storage()
                .persistent()
                .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);

            // Maintain the per-subject index for each item.
            Self::append_to_subject_index(&env, &subject, id);

            env.events().publish(
                (events::event_checkpoint_created(&env), subject.clone()),
                record,
            );

            ids.push_back(id);
        }

        Ok(ids)
    }

    // -----------------------------------------------------------------------
    // Checkpoint queries
    // -----------------------------------------------------------------------

    /// Return a single checkpoint record by its unique ID.
    ///
    /// Reading a checkpoint bumps its persistent TTL (buffer #26): audit
    /// records that are queried often but rarely rewritten would otherwise
    /// keep counting down to archival on the strength of their original
    /// write-time bump alone. A read extends the same [`LIFETIME_THRESHOLD`]
    /// / [`BUMP_AMOUNT`] window used at write time.
    ///
    /// # Errors
    /// * [`CheckpointError::CheckpointNotFound`] -- no checkpoint exists with
    ///   the requested ID.
    pub fn get_checkpoint(env: Env, id: u64) -> Result<CheckpointRecord, CheckpointError> {
        let key = StorageKey::Checkpoint(id);
        let record = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(CheckpointError::CheckpointNotFound)?;

        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);

        Ok(record)
    }

    /// Return a paginated range of checkpoint records.
    ///
    /// Retrieves up to `limit` checkpoints starting from `start_id`
    /// (inclusive). The result never exceeds [`MAX_PAGE_SIZE`] entries
    /// regardless of the requested limit.
    ///
    /// Checkpoint IDs are sequential starting at 1, so a query with
    /// `start_id = 1, limit = 50` returns the first 50 checkpoints
    /// (or fewer if fewer exist).
    ///
    /// Every record returned has its persistent TTL bumped (buffer #26),
    /// same as [`CalloraCheckpoint::get_checkpoint`].
    ///
    /// # Parameters
    /// * `start_id` -- The first checkpoint ID to retrieve (inclusive, 1-based).
    /// * `limit` -- Maximum number of records to return. Capped at
    ///   [`MAX_PAGE_SIZE`].
    ///
    /// # Errors
    /// * [`CheckpointError::InvalidPageSize`] -- `limit` is zero.
    pub fn get_checkpoints_range(
        env: Env,
        start_id: u64,
        limit: u32,
    ) -> Result<Vec<CheckpointRecord>, CheckpointError> {
        if limit == 0 {
            return Err(CheckpointError::InvalidPageSize);
        }

        let effective_limit = limit.min(MAX_PAGE_SIZE);
        let count = Self::get_checkpoint_count(env.clone());

        // If start exceeds total count, return empty.
        if start_id > count {
            return Ok(Vec::new(&env));
        }

        let end_id = start_id
            .checked_add(effective_limit as u64)
            .ok_or(CheckpointError::Overflow)?
            .min(count.saturating_add(1));

        let mut result: Vec<CheckpointRecord> = Vec::new(&env);
        for id in start_id..end_id {
            let key = StorageKey::Checkpoint(id);
            if let Some(record) = env.storage().persistent().get(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
                result.push_back(record);
            }
        }

        Ok(result)
    }

    /// Return a paginated list of checkpoint records for a specific subject.
    ///
    /// The result is ordered by creation order (ascending checkpoint ID).
    /// Returns an empty [`Vec`] when the subject has no recorded checkpoints.
    ///
    /// Reading the subject index bumps its persistent TTL. Every checkpoint
    /// record returned also has its TTL bumped (buffer #26), consistent with
    /// [`CalloraCheckpoint::get_checkpoint`] and
    /// [`CalloraCheckpoint::get_checkpoints_range`].
    ///
    /// # Parameters
    /// * `subject` -- The address whose checkpoint history is requested.
    /// * `start` -- Zero-based offset into the subject's ID list. Pass `0`
    ///   to begin from the earliest checkpoint.
    /// * `limit` -- Maximum number of records to return. Capped at
    ///   [`MAX_PAGE_SIZE`].
    ///
    /// # Errors
    /// * [`CheckpointError::InvalidPageSize`] -- `limit` is zero.
    pub fn get_checkpoints_for_subject(
        env: Env,
        subject: Address,
        start: u32,
        limit: u32,
    ) -> Result<Vec<CheckpointRecord>, CheckpointError> {
        if limit == 0 {
            return Err(CheckpointError::InvalidPageSize);
        }

        let index_key = StorageKey::SubjectIndex(subject.clone());

        // No index entry means the subject has no checkpoints yet — return empty.
        let all_ids: Vec<u64> = match env.storage().persistent().get(&index_key) {
            Some(ids) => ids,
            None => return Ok(Vec::new(&env)),
        };

        // Bump the index entry's TTL on every read so it stays alive as long
        // as the checkpoints it references.
        env.storage()
            .persistent()
            .extend_ttl(&index_key, LIFETIME_THRESHOLD, BUMP_AMOUNT);

        let total = all_ids.len();
        if start >= total || total == 0 {
            return Ok(Vec::new(&env));
        }

        let effective_limit = limit.min(MAX_PAGE_SIZE);
        let end = (start.saturating_add(effective_limit)).min(total);

        let mut result: Vec<CheckpointRecord> = Vec::new(&env);
        for idx in start..end {
            let id = all_ids.get(idx).unwrap();
            let key = StorageKey::Checkpoint(id);
            if let Some(record) = env.storage().persistent().get(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
                result.push_back(record);
            }
        }

        Ok(result)
    }

    /// Return the total number of checkpoints that have been created.
    ///
    /// This is an O(1) instance-storage read. It tracks the highest ID
    /// assigned so far (which is equal to the count since IDs are sequential
    /// starting at 1).
    pub fn get_checkpoint_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&StorageKey::CheckpointCount)
            .unwrap_or(0)
    }

    /// Return the most recent checkpoint ID.
    ///
    /// Returns `0` when no checkpoints have been created yet.
    pub fn get_latest_checkpoint_id(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&StorageKey::NextCheckpointId)
            .unwrap_or(0)
    }

    /// Return the most recently created checkpoint record, or `None` when
    /// no checkpoints exist.
    ///
    /// This is a convenience wrapper around
    /// [`CalloraCheckpoint::get_latest_checkpoint_id`] and
    /// [`CalloraCheckpoint::get_checkpoint`], and so also bumps the
    /// returned record's persistent TTL (buffer #26).
    pub fn get_latest_checkpoint(env: Env) -> Option<CheckpointRecord> {
        let latest_id = Self::get_latest_checkpoint_id(env.clone());
        if latest_id == 0 {
            return None;
        }
        Self::get_checkpoint(env, latest_id).ok()
    }

    // -----------------------------------------------------------------------
    // TTL management (buffer top-up)
    // -----------------------------------------------------------------------

    /// Extend the TTL of a single checkpoint record.
    ///
    /// Allows the admin to "top up" the persistent storage lifetime of a
    /// checkpoint so that important audit records do not expire. After the
    /// call the checkpoint's TTL is reset to [`BUMP_AMOUNT`] ledgers
    /// (â‰ˆ6 months).
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `checkpoint_id` -- The ID of the checkpoint to bump.
    ///
    /// # Errors
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::CheckpointNotFound`] -- no checkpoint with the
    ///   given ID exists.
    pub fn bump_checkpoint_ttl(
        env: Env,
        caller: Address,
        checkpoint_id: u64,
    ) -> Result<(), CheckpointError> {
        Self::require_admin(&env, &caller)?;

        let key = StorageKey::Checkpoint(checkpoint_id);
        if !env.storage().persistent().has(&key) {
            return Err(CheckpointError::CheckpointNotFound);
        }

        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);

        Ok(())
    }

    /// Extend the TTL of a range of checkpoint records.
    ///
    /// Convenience wrapper around [`CalloraCheckpoint::bump_checkpoint_ttl`]
    /// for bulk operations. Iterates from `start_id` to `end_id` (inclusive)
    /// and bumps every checkpoint that exists. Missing IDs in the range are
    /// silently skipped so callers can use the current checkpoint count as
    /// `end_id` without worrying about gaps.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `start_id` -- First checkpoint ID to bump (inclusive, 1-based).
    /// * `end_id` -- Last checkpoint ID to bump (inclusive).
    ///
    /// # Errors
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::InvalidPageSize`] -- `start_id` is greater than `end_id`.
    pub fn bump_checkpoints_ttl_range(
        env: Env,
        caller: Address,
        start_id: u64,
        end_id: u64,
    ) -> Result<(), CheckpointError> {
        Self::require_admin(&env, &caller)?;

        if start_id > end_id {
            return Err(CheckpointError::InvalidPageSize);
        }

        for id in start_id..=end_id {
            let key = StorageKey::Checkpoint(id);
            if env.storage().persistent().has(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Upgrade
    // -----------------------------------------------------------------------

    /// Admin-gated contract upgrade. Replaces the WASM and persists the
    /// new hash in instance storage.
    ///
    /// # Parameters
    /// * `caller` -- Must be the current admin; must authorize.
    /// * `new_wasm_hash` -- The 32-byte hash of the replacement WASM.
    ///
    /// # Errors
    /// * [`CheckpointError::Unauthorized`] -- caller is not the current admin.
    /// * [`CheckpointError::NotInitialized`] -- contract not initialized.
    ///
    /// # Events
    /// Emits `upgraded` with the admin as topic and the new WASM hash as data.
    pub fn upgrade(
        env: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), CheckpointError> {
        Self::require_admin(&env, &caller)?;

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

        env.events().publish(
            (events::event_upgraded(&env), Self::admin(&env)?),
            new_wasm_hash,
        );

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
