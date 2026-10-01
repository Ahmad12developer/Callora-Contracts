#[cfg(test)]
extern crate std;

use crate::{
    AdminMigrationEvent, CalloraSettlement, CalloraSettlementClient, SettlementError, StorageKey,
    DEVELOPER_MIGRATION_TIMELOCK_SECONDS,
};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{token as token_mod, Address, Env, Error, IntoVal, InvokeError, Symbol};

fn setup() -> (Env, Address, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let token = Address::generate(&env);
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let contract = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract);
    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &token);
    client.receive_payment(&vault, &500i128, &false, &Some(from.clone()), &token, &100u32);
    (env, contract, admin, vault, from, to, token)
}

fn is_error<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
    expected: SettlementError,
) -> bool {
    match result {
        Err(Ok(error)) => error.into().get_code() == expected as u32,
        _ => false,
    }
}

#[test]
fn proposal_stores_balance_snapshot_and_deadline() {
    let (env, contract, admin, _vault, from, to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);

    client.propose_balance_migration(&admin, &from, &to);

    let pending = client.get_balance_migration(&from).unwrap();
    assert_eq!(pending.from, from);
    assert_eq!(pending.to, to);
    assert_eq!(pending.amount, 500i128);
    assert_eq!(pending.proposed_at, 1_700_000_000);
    assert_eq!(
        pending.execute_after,
        1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS
    );
}

#[test]
fn execution_requires_timelock_and_succeeds_at_boundary() {
    let (env, contract, admin, _vault, from, to, token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    client.propose_balance_migration(&admin, &from, &to);

    let early = client.try_execute_balance_migration(&admin, &from);
    assert!(is_error(early, SettlementError::TimelockNotExpired));
    assert_eq!(client.get_developer_balance(&from, &token), 500i128);
    assert_eq!(client.get_developer_balance(&to, &token), 0i128);

    env.ledger()
        .set_timestamp(1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS);
    client.execute_balance_migration(&admin, &from);

    assert_eq!(client.get_developer_balance(&from, &token), 0i128);
    assert_eq!(client.get_developer_balance(&to, &token), 500i128);
    assert_eq!(client.get_balance_migration(&from), None);
}

#[test]
fn execution_adds_to_destination_and_leaves_later_source_credits() {
    let (env, contract, admin, vault, from, to, token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    client.receive_payment(&vault, &40i128, &false, &Some(to.clone()), &token, &1u32);
    client.propose_balance_migration(&admin, &from, &to);
    client.receive_payment(&vault, &25i128, &false, &Some(from.clone()), &token, &2u32);
    env.ledger()
        .set_timestamp(1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS);

    client.execute_balance_migration(&admin, &from);

    assert_eq!(client.get_developer_balance(&from, &token), 25i128);
    assert_eq!(client.get_developer_balance(&to, &token), 540i128);
}

#[test]
fn execute_emits_admin_migration_event_and_cannot_replay() {
    let (env, contract, admin, _vault, from, to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    client.propose_balance_migration(&admin, &from, &to);
    let executed_at = 1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS + 1;
    env.ledger().set_timestamp(executed_at);

    client.execute_balance_migration(&admin, &from);

    let events = env.events().all();
    let event = events
        .iter()
        .find(|event| {
            let topic: Symbol = event.1.get(0).unwrap().into_val(&env);
            topic == Symbol::new(&env, "admin_migration")
        })
        .expect("admin_migration event");
    let data: AdminMigrationEvent = event.2.into_val(&env);
    assert_eq!(data.from, from);
    assert_eq!(data.to, to);
    assert_eq!(data.amount, 500i128);
    assert_eq!(data.executed_at, executed_at);

    let replay = client.try_execute_balance_migration(&admin, &from);
    assert!(is_error(replay, SettlementError::MigrationNotFound));
}

#[test]
fn both_state_changes_require_current_admin_auth() {
    let (env, contract, admin, _vault, from, to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    env.set_auths(&[]);
    assert!(client
        .try_propose_balance_migration(&admin, &from, &to)
        .is_err());

    env.mock_all_auths();
    client.propose_balance_migration(&admin, &from, &to);
    env.ledger()
        .set_timestamp(1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS);
    env.set_auths(&[]);
    assert!(client.try_execute_balance_migration(&admin, &from).is_err());
}

#[test]
fn unauthorized_address_is_rejected_even_when_it_authorizes() {
    let (env, contract, _admin, _vault, from, to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    let outsider = Address::generate(&env);

    let result = client.try_propose_balance_migration(&outsider, &from, &to);
    assert!(is_error(result, SettlementError::Unauthorized));
}

#[test]
fn invalid_proposals_are_rejected() {
    let (env, contract, admin, _vault, from, _to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    assert!(is_error(
        client.try_propose_balance_migration(&admin, &from, &from),
        SettlementError::MigrationSameAddress
    ));
    assert!(is_error(
        client.try_propose_balance_migration(&admin, &from, &contract),
        SettlementError::InvalidMigrationTarget
    ));
    let empty = Address::generate(&env);
    let target = Address::generate(&env);
    assert!(is_error(
        client.try_propose_balance_migration(&admin, &empty, &target),
        SettlementError::NoDeveloperBalance
    ));
}

#[test]
fn reproposal_replaces_target_and_restarts_timelock() {
    let (env, contract, admin, _vault, from, first_to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    let second_to = Address::generate(&env);
    client.propose_balance_migration(&admin, &from, &first_to);
    env.ledger().set_timestamp(1_700_000_100);

    client.propose_balance_migration(&admin, &from, &second_to);

    let pending = client.get_balance_migration(&from).unwrap();
    assert_eq!(pending.to, second_to);
    assert_eq!(
        pending.execute_after,
        1_700_000_100 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS
    );
}

#[test]
fn proposal_rejects_timestamp_overflow() {
    let (env, contract, admin, _vault, from, to, _token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    env.ledger().set_timestamp(u64::MAX);

    let result = client.try_propose_balance_migration(&admin, &from, &to);

    assert!(is_error(result, SettlementError::TimelockOverflow));
    assert_eq!(client.get_balance_migration(&from), None);
}

#[test]
fn execution_rejects_a_spent_snapshot_without_partial_writes() {
    let (env, contract, admin, _vault, from, to, token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    client.propose_balance_migration(&admin, &from, &to);
    env.as_contract(&contract, || {
        env.storage().persistent().set(
            &StorageKey::DeveloperBalance(from.clone(), token.clone()),
            &499_i128,
        );
    });
    env.ledger()
        .set_timestamp(1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS);

    let result = client.try_execute_balance_migration(&admin, &from);

    assert!(is_error(result, SettlementError::MigrationBalanceChanged));
    assert_eq!(client.get_developer_balance(&from, &token), 499i128);
    assert_eq!(client.get_developer_balance(&to, &token), 0i128);
    assert!(client.get_balance_migration(&from).is_some());
}

#[test]
fn destination_overflow_reverts_all_migration_state() {
    let (env, contract, admin, _vault, from, to, token) = setup();
    let client = CalloraSettlementClient::new(&env, &contract);
    client.propose_balance_migration(&admin, &from, &to);
    env.as_contract(&contract, || {
        env.storage().persistent().set(
            &StorageKey::DeveloperBalance(to.clone(), token.clone()),
            &i128::MAX,
        );
    });
    env.ledger()
        .set_timestamp(1_700_000_000 + DEVELOPER_MIGRATION_TIMELOCK_SECONDS);

    let result = client.try_execute_balance_migration(&admin, &from);

    assert!(is_error(result, SettlementError::DeveloperOverflow));
    assert_eq!(client.get_developer_balance(&from, &token), 500i128);
    assert_eq!(client.get_developer_balance(&to, &token), i128::MAX);
    assert!(client.get_balance_migration(&from).is_some());
}