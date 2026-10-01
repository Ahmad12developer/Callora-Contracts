//! Event topic Symbol constructors and structured emit helpers for the
//! Callora Settlement contract.
//!
//! # Design
//!
//! This module is the **single source of truth** for every event emitted by the
//! settlement contract. It exposes two layers:
//!
//! 1. **Topic constructors** – `event_*(&env) -> Symbol` functions that return
//!    the stable topic Symbol for a given event. These guarantee byte-level
//!    identity and prevent accidental topic-name drift across call sites.
//!
//! 2. **Emit helpers** – `emit_*(&env, …)` functions that bundle the topic
//!    construction, data struct construction, and `env.events().publish()` call
//!    into one place. Call sites in `lib.rs`, `admin.rs`, and `limits.rs` use
//!    these helpers exclusively; no inline `Symbol::new(…)` or
//!    `env.events().publish(…)` calls appear outside this module.
//!
//! # Adding a new event
//!
//! 1. Add `pub fn event_<name>(env: &Env) -> Symbol` with a doc-comment.
//! 2. Add a corresponding `pub fn emit_<name>(env: &Env, …)` function.
//! 3. Add a snapshot test in the `#[cfg(test)] mod tests` block asserting
//!    byte-identity of the topic string.
//! 4. Update `EVENT_SCHEMA.md` and `docs/EVENT_TOPICS.md`.

use soroban_sdk::{Address, BytesN, Env, Symbol};

use crate::limits::MinBalanceChanged;
use crate::types::{
    AdminBroadcast, AdminMigrationEvent, BalanceCreditedEvent, DailyWithdrawCapChanged,
    DeductionRecordedEvent, DepositEvent, DeveloperClaimWindowChanged, DeveloperForceCreditedEvent,
    DeveloperWithdrawEvent, GlobalPool, PaymentReceivedEvent, UpgradeCancelledEvent, UpgradeProposedEvent,
    VaultAcceptedEvent, VaultProposedEvent,
};

// ─── Topic constructors ──────────────────────────────────────────────────────

/// Returns the Symbol for the `"initialized"` event topic.
///
/// **What**: Returns the canonical symbol for contract initialization events.
///
/// **How**: Creates a `Symbol` from `"initialized"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_initialized(env: &Env) -> Symbol {
    Symbol::new(env, "initialized")
}

/// Returns the Symbol for the `"payment_received"` event topic.
///
/// **What**: Returns the canonical symbol for payment received events.
///
/// **How**: Creates a `Symbol` from `"payment_received"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_payment_received(env: &Env) -> Symbol {
    Symbol::new(env, "payment_received")
}
/// Returns the Symbol for the `supported_token_added` event topic.
pub fn event_supported_token_added(env: &Env) -> Symbol {
    Symbol::new(env, "supported_token_added")
}

/// Returns the Symbol for the `supported_token_removed` event topic.
pub fn event_supported_token_removed(env: &Env) -> Symbol {
    Symbol::new(env, "supported_token_removed")
}

/// Emit `supported_token_added` when an admin enables a payment token.
pub fn emit_supported_token_added(env: &Env, caller: &Address, token: &Address) {
    env.events().publish(
        (
            event_supported_token_added(env),
            caller.clone(),
            token.clone(),
        ),
        token.clone(),
    );
}

/// Emit `supported_token_removed` when an admin disables a payment token.
pub fn emit_supported_token_removed(env: &Env, caller: &Address, token: &Address) {
    env.events().publish(
        (
            event_supported_token_removed(env),
            caller.clone(),
            token.clone(),
        ),
        token.clone(),
    );
}

/// Returns the canonical Symbol for the `deduction_recorded` event topic.
pub fn event_deduction_recorded(env: &Env) -> Symbol {
    Symbol::new(env, "deduction_recorded")
}

/// Emit an accounting-only vault deduction after its replay marker is stored.
pub fn emit_deduction_recorded(env: &Env, data: DeductionRecordedEvent) {
    env.events().publish((event_deduction_recorded(env),), data);
}

/// Returns the Symbol for the `"balance_credited"` event topic.
///
/// **What**: Returns the canonical symbol for balance credited events.
///
/// **How**: Creates a `Symbol` from `"balance_credited"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_balance_credited(env: &Env) -> Symbol {
    Symbol::new(env, "balance_credited")
}

/// Returns the Symbol for the `"deposit"` event topic.
///
/// **What**: Returns the canonical symbol for developer deposit events.
///
/// **How**: Creates a `Symbol` from `"deposit"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_deposit(env: &Env) -> Symbol {
    Symbol::new(env, "deposit")
}

/// Returns the Symbol for the `"developer_withdraw"` event topic.
///
/// **What**: Returns the canonical symbol for developer withdrawal events.
///
/// **How**: Creates a `Symbol` from `"developer_withdraw"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_developer_withdraw(env: &Env) -> Symbol {
    Symbol::new(env, "developer_withdraw")
}

/// Returns the Symbol for the `"daily_withdraw_cap_changed"` event topic.
///
/// **What**: Returns the canonical symbol for daily withdrawal cap change events.
///
/// **How**: Creates a `Symbol` from `"daily_withdraw_cap_changed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_daily_withdraw_cap_changed(env: &Env) -> Symbol {
    Symbol::new(env, "daily_withdraw_cap_changed")
}

/// Returns the Symbol for the `"claim_window_changed"` event topic.
///
/// **What**: Returns the canonical symbol for claim window change events.
///
/// **How**: Creates a `Symbol` from `"claim_window_changed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_developer_claim_window_changed(env: &Env) -> Symbol {
    Symbol::new(env, "claim_window_changed")
}

/// Returns the Symbol for the `"admin_nominated"` event topic.
///
/// **What**: Returns the canonical symbol for admin nomination events.
///
/// **How**: Creates a `Symbol` from `"admin_nominated"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_nominated(env: &Env) -> Symbol {
    Symbol::new(env, "admin_nominated")
}

/// Returns the Symbol for the `"admin_accepted"` event topic.
///
/// **What**: Returns the canonical symbol for admin acceptance events.
///
/// **How**: Creates a `Symbol` from `"admin_accepted"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_accepted(env: &Env) -> Symbol {
    Symbol::new(env, "admin_accepted")
}

/// Returns the Symbol for the `"admin_cancelled"` event topic.
///
/// **What**: Returns the canonical symbol for admin transfer cancellation events.
///
/// **How**: Creates a `Symbol` from `"admin_cancelled"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "admin_cancelled")
}

/// Returns the Symbol for the `"vault_proposed"` event topic.
///
/// **What**: Returns the canonical symbol for vault proposal events.
///
/// **How**: Creates a `Symbol` from `"vault_proposed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_vault_proposed(env: &Env) -> Symbol {
    Symbol::new(env, "vault_proposed")
}

/// Returns the Symbol for the `"vault_accepted"` event topic.
///
/// **What**: Returns the canonical symbol for vault acceptance events.
///
/// **How**: Creates a `Symbol` from `"vault_accepted"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_vault_accepted(env: &Env) -> Symbol {
    Symbol::new(env, "vault_accepted")
}

/// Returns the Symbol for the `"upgraded"` event topic.
///
/// **What**: Returns the canonical symbol for contract upgrade events.
///
/// **How**: Creates a `Symbol` from `"upgraded"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_upgraded(env: &Env) -> Symbol {
    Symbol::new(env, "upgraded")
}

/// Returns the Symbol for the `"developer_force_credited"` event topic.
///
/// **What**: Returns the canonical symbol for developer force credit events.
///
/// **How**: Creates a `Symbol` from `"developer_force_credited"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_developer_force_credited(env: &Env) -> Symbol {
    Symbol::new(env, "developer_force_credited")
}

/// Returns the Symbol for the `"admin_broadcast"` event topic.
///
/// **What**: Returns the canonical symbol for admin emergency broadcast events.
///
/// **How**: Creates a `Symbol` from `"admin_broadcast"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_broadcast(env: &Env) -> Symbol {
    Symbol::new(env, "admin_broadcast")
}

/// Returns the Symbol for the `"admin_migration_proposed"` event topic.
///
/// **What**: Returns the canonical symbol for admin migration proposal events.
///
/// **How**: Creates a `Symbol` from `"admin_migration_proposed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_migration_proposed(env: &Env) -> Symbol {
    Symbol::new(env, "admin_migration_proposed")
}

/// Returns the Symbol for the `"admin_migration"` event topic.
///
/// **What**: Returns the canonical symbol for admin migration execution events.
///
/// **How**: Creates a `Symbol` from `"admin_migration"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_admin_migration(env: &Env) -> Symbol {
    Symbol::new(env, "admin_migration")
}

/// Returns the Symbol for the canonical event version marker used by Callora.
pub fn event_version_v1(env: &Env) -> Symbol {
    Symbol::new(env, "callora.v1")
}

/// Returns the Symbol for the `"developer_min_balance_changed"` event topic.
///
/// **What**: Returns the canonical symbol for developer minimum balance change events.
///
/// **How**: Creates a `Symbol` from `"developer_min_balance_changed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_developer_min_balance_changed(env: &Env) -> Symbol {
    Symbol::new(env, "developer_min_balance_changed")
}

/// Returns the Symbol for the `"metadata_removed"` event topic.
///
/// **What**: Returns the canonical symbol for metadata removal events.
///
/// **How**: Creates a `Symbol` from `"metadata_removed"`.
///
/// **Why**: Centralizes topic creation to guarantee byte-identity across call sites.
///
/// # Arguments
/// * `env` - Soroban environment handle.
pub fn event_metadata_removed(env: &Env) -> Symbol {
    Symbol::new(env, "metadata_removed")
}

/// Returns the Symbol for the `"price_set"` event topic.
pub fn event_price_set(env: &Env) -> Symbol {
    Symbol::new(env, "price_set")
}

/// Returns the Symbol for the `"price_removed"` event topic.
pub fn event_price_removed(env: &Env) -> Symbol {
    Symbol::new(env, "price_removed")
}

// ─── Emit helpers ────────────────────────────────────────────────────────────

/// Emit `"price_set"` when an offering price is created or changed.
pub fn emit_price_set(
    env: &Env,
    offering_id: &soroban_sdk::String,
    old: Option<soroban_sdk::String>,
    new: &soroban_sdk::String,
) {
    env.events().publish(
        (event_price_set(env), offering_id.clone()),
        (old, new.clone()),
    );
}

/// Emit `"price_removed"` when an existing offering price is removed.
pub fn emit_price_removed(env: &Env, offering_id: &soroban_sdk::String, old: &soroban_sdk::String) {
    env.events()
        .publish((event_price_removed(env), offering_id.clone()), old.clone());
}

/// Emit `"initialized"` once when the settlement contract is first set up.
///
/// **What**: Publishes the initialization event containing the global pool configuration snapshot.
///
/// **How**: Calls `env.events().publish()` with topic `(initialized, admin, vault)` and payload `GlobalPool`.
///
/// **Why**: Allows off-chain indexers to discover and record contract deployment parameters.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Primary contract administrator address.
/// * `vault` - Authorized vault contract address.
/// * `pool` - Initial global pool parameters snapshot.
pub fn emit_initialized(env: &Env, admin: &Address, vault: &Address, pool: &GlobalPool) {
    env.events().publish(
        (event_initialized(env), admin.clone(), vault.clone()),
        pool.clone(),
    );
}

/// Emit `"payment_received"` when an inbound payment is routed to the global
/// pool or a developer balance.
///
/// **What**: Publishes a payment received event recording caller and payment metadata.
///
/// **How**: Calls `env.events().publish()` with topic `(payment_received, caller)` and payload `PaymentReceivedEvent`.
///
/// **Why**: Indexers track incoming revenue allocations across developers and global pool.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Account or vault address making the payment deposit.
/// * `payload` - Structured payment received details.
pub fn emit_payment_received(env: &Env, caller: &Address, payload: PaymentReceivedEvent) {
    env.events()
        .publish((event_payment_received(env), caller.clone()), payload);
}

/// Emit `"balance_credited"` when a developer's balance is incremented.
///
/// **What**: Publishes a developer credit event recording developer and amount credited.
///
/// **How**: Calls `env.events().publish()` with topic `(balance_credited, developer)` and payload `BalanceCreditedEvent`.
///
/// **Why**: Provides accounting transparency for developer revenue accrual.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Target developer account address.
/// * `payload` - Structured credit event details.
pub fn emit_balance_credited(env: &Env, developer: &Address, payload: BalanceCreditedEvent) {
    env.events()
        .publish((event_balance_credited(env), developer.clone()), payload);
}

/// Emit `"deposit"` alongside each developer credit (both single and batch).
///
/// **What**: Publishes a deposit event paired with developer credit.
///
/// **How**: Calls `env.events().publish()` with topic `(deposit, developer)` and payload `DepositEvent`.
///
/// **Why**: Maintained for indexer compatibility tracking developer deposit entries.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Developer account address.
/// * `payload` - Structured deposit details.
pub fn emit_deposit(env: &Env, developer: &Address, payload: DepositEvent) {
    env.events()
        .publish((event_deposit(env), developer.clone()), payload);
}

/// Emit `"developer_withdraw"` when a developer withdraws accrued balance.
///
/// **What**: Publishes a developer withdrawal event recording payout amount.
///
/// **How**: Calls `env.events().publish()` with topic `(developer_withdraw, developer)` and payload `DeveloperWithdrawEvent`.
///
/// **Why**: Indexers monitor withdrawal frequency, daily cap usage, and payout amounts.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Developer account performing withdrawal.
/// * `payload` - Structured withdrawal event details.
pub fn emit_developer_withdraw(env: &Env, developer: &Address, payload: DeveloperWithdrawEvent) {
    env.events()
        .publish((event_developer_withdraw(env), developer.clone()), payload);
}

/// Emit `"daily_withdraw_cap_changed"` when the admin updates a developer's
/// daily cap.
///
/// **What**: Publishes an event when a developer's daily withdrawal cap is modified.
///
/// **How**: Calls `env.events().publish()` with topic `(daily_withdraw_cap_changed, caller)` and payload `DailyWithdrawCapChanged`.
///
/// **Why**: Audit log for administrative cap adjustments.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Admin address changing the cap.
/// * `payload` - Structured daily withdrawal cap change details.
pub fn emit_daily_withdraw_cap_changed(
    env: &Env,
    caller: &Address,
    payload: DailyWithdrawCapChanged,
) {
    env.events().publish(
        (event_daily_withdraw_cap_changed(env), caller.clone()),
        payload,
    );
}

/// Emit `"claim_window_changed"` when a developer claim window is set or cleared.
///
/// **What**: Publishes an event when a developer's claim window parameters are updated.
///
/// **How**: Calls `env.events().publish()` with topic `(claim_window_changed, developer)` and payload `DeveloperClaimWindowChanged`.
///
/// **Why**: Audit trail for claim window timeline adjustments.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Target developer address.
/// * `payload` - Structured claim window change details.
pub fn emit_developer_claim_window_changed(
    env: &Env,
    developer: &Address,
    payload: DeveloperClaimWindowChanged,
) {
    env.events().publish(
        (event_developer_claim_window_changed(env), developer.clone()),
        payload,
    );
}

/// Emit `"admin_nominated"` when the current admin nominates a successor.
///
/// **What**: Publishes an event when an admin succession process is initiated.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_nominated, current_admin, new_admin)` and payload `new_admin`.
///
/// **Why**: Indexers track upcoming admin transfers for governance operations.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `current_admin` - Address of current admin nominating a successor.
/// * `new_admin` - Address of nominated pending admin.
pub fn emit_admin_nominated(env: &Env, current_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (
            event_admin_nominated(env),
            current_admin.clone(),
            new_admin.clone(),
        ),
        new_admin.clone(),
    );
}

/// Emit `"admin_accepted"` when the pending admin finalizes the transfer.
///
/// **What**: Publishes an event when a pending admin accepts the administrator role.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_accepted, old_admin, new_admin)` and payload `new_admin`.
///
/// **Why**: Confirms completion of two-step admin role rotation.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `old_admin` - Address of previous admin.
/// * `new_admin` - Address of newly accepted admin.
pub fn emit_admin_accepted(env: &Env, old_admin: &Address, new_admin: &Address) {
    env.events().publish(
        (
            event_admin_accepted(env),
            old_admin.clone(),
            new_admin.clone(),
        ),
        new_admin.clone(),
    );
}

/// Emit `"admin_cancelled"` when the admin cancels a pending transfer.
///
/// **What**: Publishes an event when a pending admin nomination is revoked.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_cancelled, admin)` and payload `admin`.
///
/// **Why**: Notifies indexers that a proposed admin transfer has been voided.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address revoking nomination.
pub fn emit_admin_cancelled(env: &Env, admin: &Address) {
    env.events()
        .publish((event_admin_cancelled(env), admin.clone()), admin.clone());
}

/// Emit `"vault_proposed"` when the admin proposes a new vault.
///
/// **What**: Publishes an event when a vault rotation proposal is submitted.
///
/// **How**: Calls `env.events().publish()` with topic `(vault_proposed, admin)` and payload `VaultProposedEvent`.
///
/// **Why**: Tracks proposed vault contracts prior to activation.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `admin` - Admin address initiating proposal.
/// * `payload` - Structured vault proposal details.
pub fn emit_vault_proposed(env: &Env, admin: &Address, payload: VaultProposedEvent) {
    env.events()
        .publish((event_vault_proposed(env), admin.clone()), payload);
}

/// Emit `"vault_accepted"` when the proposed vault rotation is accepted.
///
/// **What**: Publishes an event when a proposed vault is accepted and activated.
///
/// **How**: Calls `env.events().publish()` with topic `(vault_accepted, new_vault)` and payload `VaultAcceptedEvent`.
///
/// **Why**: Indexers update active vault address references.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `new_vault` - Address of newly accepted vault.
/// * `payload` - Structured vault acceptance details.
pub fn emit_vault_accepted(env: &Env, new_vault: &Address, payload: VaultAcceptedEvent) {
    env.events()
        .publish((event_vault_accepted(env), new_vault.clone()), payload);
}

/// Emit `"upgraded"` when the contract WASM is replaced.
///
/// **What**: Publishes an event when contract executable code is upgraded.
///
/// **How**: Calls `env.events().publish()` with topic `(upgraded, caller)` and payload `new_wasm_hash`.
///
/// **Why**: Audit logging of on-chain contract code modifications.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Admin address executing the upgrade.
/// * `new_wasm_hash` - 32-byte hash of new WASM code.
pub fn emit_upgraded(env: &Env, caller: &Address, new_wasm_hash: &BytesN<32>) {
    env.events()
        .publish((event_upgraded(env), caller.clone()), new_wasm_hash.clone());
}

/// Emit `"developer_force_credited"` when the admin manually credits a
/// developer balance outside the normal payment flow.
///
/// **What**: Publishes an event when a developer balance is manually credited by admin.
///
/// **How**: Calls `env.events().publish()` with topic `(developer_force_credited, developer)` and payload `DeveloperForceCreditedEvent`.
///
/// **Why**: Audit trail for manual balance corrections and migrations.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Developer account address receiving credit.
/// * `payload` - Structured force credit details.
pub fn emit_developer_force_credited(
    env: &Env,
    developer: &Address,
    payload: DeveloperForceCreditedEvent,
) {
    env.events().publish(
        (event_developer_force_credited(env), developer.clone()),
        payload,
    );
}

/// Emit `"admin_broadcast"` when the admin sends an emergency message.
///
/// **What**: Publishes an admin broadcast message event.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_broadcast, caller)` and payload `AdminBroadcast`.
///
/// **Why**: Emits system status alerts and emergency messages to indexers.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Admin address issuing broadcast.
/// * `payload` - Structured broadcast message payload.
pub fn emit_admin_broadcast(env: &Env, caller: &Address, payload: AdminBroadcast) {
    env.events()
        .publish((event_admin_broadcast(env), caller.clone()), payload);
}

/// Emit `"admin_migration_proposed"` when a timelock'd developer balance
/// migration proposal is recorded.
///
/// **What**: Publishes an event when a balance migration is proposed under timelock.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_migration_proposed, from)` and payload `PendingDeveloperMigration`.
///
/// **Why**: Notifies indexers of pending timelocked balance migrations.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `from` - Source developer address.
/// * `payload` - Structured pending migration payload.
pub fn emit_admin_migration_proposed(
    env: &Env,
    from: &Address,
    payload: crate::timelock::PendingDeveloperMigration,
) {
    env.events()
        .publish((event_admin_migration_proposed(env), from.clone()), payload);
}

/// Emit `"admin_migration"` when a pending balance migration is executed.
///
/// **What**: Publishes an event when a timelocked balance migration is executed.
///
/// **How**: Calls `env.events().publish()` with topic `(admin_migration, from, to)` and payload `AdminMigrationEvent`.
///
/// **Why**: Traces executed balance transfers between developer accounts.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `from` - Source developer address.
/// * `to` - Destination developer address.
/// * `payload` - Structured migration event payload.
pub fn emit_admin_migration(env: &Env, from: &Address, to: &Address, payload: AdminMigrationEvent) {
    env.events().publish(
        (event_admin_migration(env), from.clone(), to.clone()),
        payload,
    );
}

/// Emit `"developer_min_balance_changed"` when the admin sets a developer's
/// minimum withdrawal threshold.
///
/// **What**: Publishes an event when a developer's minimum balance threshold is changed.
///
/// **How**: Calls `env.events().publish()` with topic `(developer_min_balance_changed, developer)` and payload `MinBalanceChanged`.
///
/// **Why**: Audit trail for minimum balance configuration changes.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `developer` - Target developer account address.
/// * `payload` - Structured minimum balance change details.
pub fn emit_developer_min_balance_changed(
    env: &Env,
    developer: &Address,
    payload: MinBalanceChanged,
) {
    env.events().publish(
        (event_developer_min_balance_changed(env), developer.clone()),
        payload,
    );
}

// ─── Upgrade timelock events ─────────────────────────────────────────────────

/// Returns the Symbol for the `"upgrade_proposed"` event topic.
pub fn event_upgrade_proposed(env: &Env) -> Symbol {
    Symbol::new(env, "upgrade_proposed")
}

/// Returns the Symbol for the `"upgrade_cancelled"` event topic.
pub fn event_upgrade_cancelled(env: &Env) -> Symbol {
    Symbol::new(env, "upgrade_cancelled")
}

/// Emit `"upgrade_proposed"` when the admin proposes a timelocked WASM upgrade.
///
/// **What**: Records a pending WASM hash together with the execution deadline.
///
/// **How**: `env.events().publish()` with topic `(upgrade_proposed, caller)` and payload `UpgradeProposedEvent`.
///
/// **Why**: Lets off-chain watchers detect and verify an upcoming upgrade before it executes.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Admin address proposing the upgrade.
/// * `payload` - Structured proposal details.
pub fn emit_upgrade_proposed(env: &Env, caller: &Address, payload: UpgradeProposedEvent) {
    env.events()
        .publish((event_upgrade_proposed(env), caller.clone()), payload);
}

/// Emit `"upgrade_cancelled"` when the admin cancels a pending upgrade proposal.
///
/// **What**: Records that a previously proposed WASM upgrade has been voided.
///
/// **How**: `env.events().publish()` with topic `(upgrade_cancelled, caller)` and payload `UpgradeCancelledEvent`.
///
/// **Why**: Audit trail confirming no upgrade took place for the cancelled hash.
///
/// # Arguments
/// * `env` - Soroban environment handle.
/// * `caller` - Admin address cancelling the proposal.
/// * `payload` - Structured cancellation details.
pub fn emit_upgrade_cancelled(env: &Env, caller: &Address, payload: UpgradeCancelledEvent) {
    env.events()
        .publish((event_upgrade_cancelled(env), caller.clone()), payload);
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    // ── Topic byte-identity snapshots ─────────────────────────────────────

    /// Every constructor must map to exactly the expected byte string.
    /// Changing any of these would be a breaking change to the on-chain
    /// interface; that is why they are explicitly snapshot-tested.

    #[test]
    fn test_event_initialized_bytes() {
        let env = Env::default();
        assert_eq!(event_initialized(&env), Symbol::new(&env, "initialized"));
    }

    #[test]
    fn test_event_deduction_recorded_bytes() {
        let env = Env::default();
        assert_eq!(
            event_deduction_recorded(&env),
            Symbol::new(&env, "deduction_recorded")
        );
    }

    #[test]
    fn test_event_payment_received_bytes() {
        let env = Env::default();
        assert_eq!(
            event_payment_received(&env),
            Symbol::new(&env, "payment_received")
        );
    }

    #[test]
    fn test_event_balance_credited_bytes() {
        let env = Env::default();
        assert_eq!(
            event_balance_credited(&env),
            Symbol::new(&env, "balance_credited")
        );
    }

    #[test]
    fn test_event_deposit_bytes() {
        let env = Env::default();
        assert_eq!(event_deposit(&env), Symbol::new(&env, "deposit"));
    }

    #[test]
    fn test_event_developer_withdraw_bytes() {
        let env = Env::default();
        assert_eq!(
            event_developer_withdraw(&env),
            Symbol::new(&env, "developer_withdraw")
        );
    }

    #[test]
    fn test_event_daily_withdraw_cap_changed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_daily_withdraw_cap_changed(&env),
            Symbol::new(&env, "daily_withdraw_cap_changed")
        );
    }

    #[test]
    fn test_event_developer_claim_window_changed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_developer_claim_window_changed(&env),
            Symbol::new(&env, "claim_window_changed")
        );
    }

    #[test]
    fn test_event_admin_nominated_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_nominated(&env),
            Symbol::new(&env, "admin_nominated")
        );
    }

    #[test]
    fn test_event_admin_accepted_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_accepted(&env),
            Symbol::new(&env, "admin_accepted")
        );
    }

    #[test]
    fn test_event_admin_cancelled_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_cancelled(&env),
            Symbol::new(&env, "admin_cancelled")
        );
    }

    #[test]
    fn test_event_vault_proposed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_vault_proposed(&env),
            Symbol::new(&env, "vault_proposed")
        );
    }

    #[test]
    fn test_event_vault_accepted_bytes() {
        let env = Env::default();
        assert_eq!(
            event_vault_accepted(&env),
            Symbol::new(&env, "vault_accepted")
        );
    }

    #[test]
    fn test_event_upgraded_bytes() {
        let env = Env::default();
        assert_eq!(event_upgraded(&env), Symbol::new(&env, "upgraded"));
    }

    #[test]
    fn test_event_developer_force_credited_bytes() {
        let env = Env::default();
        assert_eq!(
            event_developer_force_credited(&env),
            Symbol::new(&env, "developer_force_credited")
        );
    }

    #[test]
    fn test_event_admin_broadcast_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_broadcast(&env),
            Symbol::new(&env, "admin_broadcast")
        );
    }

    #[test]
    fn test_admin_migration_event_bytes() {
        let env = Env::default();
        assert_eq!(
            event_admin_migration_proposed(&env),
            Symbol::new(&env, "admin_migration_proposed")
        );
        assert_eq!(
            event_admin_migration(&env),
            Symbol::new(&env, "admin_migration")
        );
    }

    #[test]
    fn test_event_developer_min_balance_changed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_developer_min_balance_changed(&env),
            Symbol::new(&env, "developer_min_balance_changed")
        );
    }

    #[test]
    fn test_event_metadata_removed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_metadata_removed(&env),
            Symbol::new(&env, "metadata_removed")
        );
    }

    #[test]
    fn test_event_upgrade_proposed_bytes() {
        let env = Env::default();
        assert_eq!(
            event_upgrade_proposed(&env),
            Symbol::new(&env, "upgrade_proposed")
        );
    }

    #[test]
    fn test_event_upgrade_cancelled_bytes() {
        let env = Env::default();
        assert_eq!(
            event_upgrade_cancelled(&env),
            Symbol::new(&env, "upgrade_cancelled")
        );
    }
}
