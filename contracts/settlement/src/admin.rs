//! Admin-only developer balance recovery operations.

use soroban_sdk::{Address, Env};

use crate::types::AdminMigrationEvent;
use crate::{events, timelock, CalloraSettlement, SettlementError, StorageKey};

fn require_admin(env: &Env, caller: &Address) {
    caller.require_auth();
    let admin = CalloraSettlement::get_admin(env.clone()).unwrap();
    if caller != &admin {
        env.panic_with_error(SettlementError::Unauthorized);
    }
}

/// Retrieve the USDC token address from instance storage, panicking with
/// [`SettlementError::UsdcTokenNotConfigured`] if it has not been set.
fn get_usdc(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&StorageKey::Usdc)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::UsdcTokenNotConfigured))
}

pub(crate) fn propose_balance_migration(env: &Env, caller: &Address, from: &Address, to: &Address) {
    require_admin(env, caller);
    if from == to {
        env.panic_with_error(SettlementError::MigrationSameAddress);
    }
    if to == &env.current_contract_address() {
        env.panic_with_error(SettlementError::InvalidMigrationTarget);
    }

    let usdc_token = get_usdc(env);

    // Read the source developer's V2 per-token balance.
    let amount: i128 = env
        .storage()
        .persistent()
        .get(&StorageKey::DeveloperBalance(
            from.clone(),
            usdc_token.clone(),
        ))
        .unwrap_or(0);
    if amount <= 0 {
        env.panic_with_error(SettlementError::NoDeveloperBalance);
    }

    let proposed_at = env.ledger().timestamp();
    let execute_after = proposed_at
        .checked_add(timelock::DEVELOPER_MIGRATION_TIMELOCK_SECONDS)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::TimelockOverflow));
    let migration = timelock::PendingDeveloperMigration {
        from: from.clone(),
        to: to.clone(),
        amount,
        proposed_at,
        execute_after,
    };
    timelock::set_pending_migration(env, &migration);
    events::emit_admin_migration_proposed(env, from, migration);
}

pub(crate) fn execute_balance_migration(env: &Env, caller: &Address, from: &Address) {
    require_admin(env, caller);
    let migration = timelock::get_pending_migration(env, from)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::MigrationNotFound));
    let executed_at = env.ledger().timestamp();
    if executed_at < migration.execute_after {
        env.panic_with_error(SettlementError::TimelockNotExpired);
    }

    let usdc_token = get_usdc(env);
    let source_key = StorageKey::DeveloperBalance(from.clone(), usdc_token.clone());
    let destination_key = StorageKey::DeveloperBalance(migration.to.clone(), usdc_token.clone());

    let source_balance: i128 = env.storage().persistent().get(&source_key).unwrap_or(0);
    let new_source_balance = source_balance
        .checked_sub(migration.amount)
        .filter(|b| *b >= 0)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::MigrationBalanceChanged));
    let destination_balance: i128 = env
        .storage()
        .persistent()
        .get(&destination_key)
        .unwrap_or(0);
    let new_destination_balance = destination_balance
        .checked_add(migration.amount)
        .unwrap_or_else(|| env.panic_with_error(SettlementError::DeveloperOverflow));

    env.storage()
        .persistent()
        .set(&source_key, &new_source_balance);
    env.storage()
        .persistent()
        .set(&destination_key, &new_destination_balance);
    env.storage().persistent().extend_ttl(
        &source_key,
        crate::types::PERSISTENT_BUMP_THRESHOLD,
        crate::types::PERSISTENT_BUMP_AMOUNT,
    );
    env.storage().persistent().extend_ttl(
        &destination_key,
        crate::types::PERSISTENT_BUMP_THRESHOLD,
        crate::types::PERSISTENT_BUMP_AMOUNT,
    );

    // Register the destination developer in the paged index if not already present.
    CalloraSettlement::index_insert(env, migration.to.clone());
    timelock::remove_pending_migration(env, from);

    events::emit_admin_migration(
        env,
        from,
        &migration.to.clone(),
        AdminMigrationEvent {
            from: from.clone(),
            to: migration.to,
            amount: migration.amount,
            executed_at,
        },
    );
}
