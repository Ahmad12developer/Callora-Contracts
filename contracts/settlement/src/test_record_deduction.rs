extern crate std;

use crate::{
    CalloraSettlement, CalloraSettlementClient, DeductionRecordedEvent, SettlementError,
    StorageKey, INSTANCE_BUMP_AMOUNT, PERSISTENT_BUMP_AMOUNT,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, Env, FromVal, IntoVal, Symbol};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let contract_id = env.register(CalloraSettlement, ());
    CalloraSettlementClient::new(&env, &contract_id).init(&admin, &vault);
    (env, contract_id, vault)
}

fn has_marker(env: &Env, contract_id: &Address, request_id: u64) -> bool {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .has(&StorageKey::DeductionRequest(request_id))
    })
}

#[test]
fn record_deduction_rejects_non_positive_without_consuming_request_id() {
    let (env, contract_id, _) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.record_deduction(&50, &1);

    for amount in [0, -1, i128::MIN] {
        assert_eq!(
            client.try_record_deduction(&amount, &2),
            Err(Ok(SettlementError::AmountNotPositive.into()))
        );
        assert!(env.events().all().is_empty());
        assert_eq!(client.get_total_received(), 50);
        assert!(!has_marker(&env, &contract_id, 2));
    }
    client.record_deduction(&25, &2);
    assert_eq!(client.get_total_received(), 75);
}

#[test]
fn record_deduction_rejects_replay_without_changing_total_or_marker() {
    let (env, contract_id, _) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.record_deduction(&100, &42);
    // A changed amount must not bypass replay detection either.
    for amount in [100, 200] {
        assert_eq!(
            client.try_record_deduction(&amount, &42),
            Err(Ok(SettlementError::DuplicateRequestId.into()))
        );
        assert!(env.events().all().is_empty());
        assert_eq!(client.get_total_received(), 100);
        assert!(has_marker(&env, &contract_id, 42));
    }
    client.record_deduction(&200, &43);
    assert_eq!(client.get_total_received(), 300);
}

#[test]
fn record_deduction_emits_exact_payload_and_only_updates_accounting() {
    let (env, contract_id, vault) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    let pool_before = client.get_global_pool();

    // All u64 IDs are opaque and valid, including zero and the maximum.
    for request_id in [0, u64::MAX] {
        client.record_deduction(&75, &request_id);
        assert_eq!(env.events().all().len(), 1);
        let (emitter, topics, data) = env.events().all().last().unwrap();
        assert_eq!(emitter, contract_id);
        assert_eq!(
            topics,
            soroban_sdk::vec![&env, Symbol::new(&env, "deduction_recorded").into_val(&env)]
        );
        assert_eq!(
            DeductionRecordedEvent::from_val(&env, &data),
            DeductionRecordedEvent {
                amount: 75,
                request_id,
            }
        );
        assert_eq!(
            env.auths(),
            std::vec![(
                vault.clone(),
                soroban_sdk::testutils::AuthorizedInvocation {
                    function: soroban_sdk::testutils::AuthorizedFunction::Contract((
                        contract_id.clone(),
                        Symbol::new(&env, "record_deduction"),
                        (75i128, request_id).into_val(&env),
                    )),
                    sub_invocations: std::vec![],
                },
            ),]
        );
    }
    assert_eq!(client.get_total_received(), 150);
    assert_eq!(client.get_global_pool(), pool_before);
}

#[test]
fn record_deduction_bumps_persistent_marker_and_instance_ttl() {
    let (env, contract_id, _) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.record_deduction(&1, &7);
    env.as_contract(&contract_id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&StorageKey::DeductionRequest(7)),
            PERSISTENT_BUMP_AMOUNT
        );
        assert_eq!(env.storage().instance().get_ttl(), INSTANCE_BUMP_AMOUNT);
    });

    env.ledger()
        .set_sequence_number(env.ledger().sequence() + 100);
    assert_eq!(
        client.try_record_deduction(&1, &7),
        Err(Ok(SettlementError::DuplicateRequestId.into()))
    );
    env.as_contract(&contract_id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&StorageKey::DeductionRequest(7)),
            PERSISTENT_BUMP_AMOUNT - 100
        );
    });
    assert_eq!(client.get_total_received(), 1);
    client.record_deduction(&1, &8);
    env.as_contract(&contract_id, || {
        assert_eq!(
            env.storage()
                .persistent()
                .get_ttl(&StorageKey::DeductionRequest(8)),
            PERSISTENT_BUMP_AMOUNT
        );
    });
}

#[test]
fn record_deduction_overflow_does_not_consume_request_id() {
    let (env, contract_id, _) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&StorageKey::TotalReceived, &(i128::MAX - 1));
    });
    assert_eq!(
        client.try_record_deduction(&2, &9),
        Err(Ok(SettlementError::PoolOverflow.into()))
    );
    assert!(env.events().all().is_empty());
    assert_eq!(client.get_total_received(), i128::MAX - 1);
    assert!(!has_marker(&env, &contract_id, 9));

    client.record_deduction(&1, &9);
    assert_eq!(client.get_total_received(), i128::MAX);
    assert!(has_marker(&env, &contract_id, 9));
}

#[test]
fn record_deduction_requires_vault_authorization() {
    let (env, contract_id, _) = setup();
    let client = CalloraSettlementClient::new(&env, &contract_id);
    env.mock_auths(&[]);
    assert!(client.try_record_deduction(&100, &10).is_err());
    assert!(env.events().all().is_empty());
    assert_eq!(client.get_total_received(), 0);
    assert!(!has_marker(&env, &contract_id, 10));

    env.mock_all_auths();
    client.record_deduction(&100, &10);
    assert_eq!(client.get_total_received(), 100);
}
