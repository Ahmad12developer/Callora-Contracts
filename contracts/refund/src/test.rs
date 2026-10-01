#![cfg(test)]

use crate::{
    InitializedEvent, RefundConfigUpdatedEvent, RefundContract, RefundContractClient, RefundError,
    RefundProcessedEvent, RefundRequestedEvent, RefundStatus, MAX_REQUESTER_REFUNDS,
};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::{Address, Env, IntoVal, Symbol};

fn setup() -> (Env, Address, RefundContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);
    client.init(&admin, &250, &100);
    (env, admin, client)
}

#[test]
fn test_init() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);

    client.init(&admin, &250, &100);

    let config = client.get_config();
    assert_eq!(config.fee_bps, 250);
    assert_eq!(config.min_refund_amount, 100);

    let stored_admin = client.get_admin();
    assert_eq!(stored_admin, admin);
}

#[test]
fn test_init_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);

    client.init(&admin, &250, &100);
    let result = client.try_init(&admin, &250, &100);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::AlreadyInitialized);
}

#[test]
fn test_init_fee_too_high() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);

    let result = client.try_init(&admin, &10_001, &100);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::FeeTooHigh);
}

#[test]
fn test_init_invalid_min_amount() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);

    let result = client.try_init(&admin, &250, &-1);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidAmount);
}

#[test]
fn test_request_refund() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    assert_eq!(request_id, 1);

    let request = client.get_refund_request(&request_id);
    assert_eq!(request.id, 1);
    assert_eq!(request.requester, requester);
    assert_eq!(request.token, token);
    assert_eq!(request.amount, 500);
    assert_eq!(request.status, RefundStatus::Pending);
}

#[test]
fn test_requester_refund_index_is_paginated_in_creation_order() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let first = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "one"));
    let second = client.request_refund(&requester, &token, &600, &Symbol::new(&env, "two"));
    let third = client.request_refund(&requester, &token, &700, &Symbol::new(&env, "three"));

    let first_page = client.get_refunds_by_requester(&requester, &0, &2);
    assert_eq!(first_page.len(), 2);
    assert_eq!(first_page.get(0).unwrap(), first);
    assert_eq!(first_page.get(1).unwrap(), second);

    let second_page = client.get_refunds_by_requester(&requester, &2, &50);
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page.get(0).unwrap(), third);
}

#[test]
fn test_requester_refund_index_is_bounded() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    for _ in 0..(MAX_REQUESTER_REFUNDS + 1) {
        client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    }

    let oversized_limit = MAX_REQUESTER_REFUNDS + 1;
    let page = client.get_refunds_by_requester(&requester, &0, &oversized_limit);
    assert_eq!(page.len(), MAX_REQUESTER_REFUNDS.min(crate::MAX_REQUESTER_REFUNDS_PAGE_SIZE));
    assert_eq!(page.get(0).unwrap(), 2);
}

#[test]
fn test_request_refund_amount_too_low() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    // Min amount is 100, trying 50
    let result = client.try_request_refund(&requester, &token, &50, &Symbol::new(&env, "test"));
    assert_eq!(result.unwrap_err().unwrap(), RefundError::AmountTooLow);
}

#[test]
fn test_request_refund_invalid_amount() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let result = client.try_request_refund(&requester, &token, &0, &Symbol::new(&env, "test"));
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidAmount);

    let result = client.try_request_refund(&requester, &token, &-100, &Symbol::new(&env, "test"));
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidAmount);
}

#[test]
fn test_approve_refund() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    client.approve_refund(&admin, &request_id);

    let request = client.get_refund_request(&request_id);
    assert_eq!(request.status, RefundStatus::Approved);
}

#[test]
fn test_approve_refund_unauthorized() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);
    let fake_admin = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let result = client.try_approve_refund(&fake_admin, &request_id);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::Unauthorized);
}

#[test]
fn test_approve_refund_not_found() {
    let (env, admin, client) = setup();

    let result = client.try_approve_refund(&admin, &999);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::NotFound);
}

#[test]
fn test_approve_refund_invalid_status() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.approve_refund(&admin, &request_id);

    // Try to approve again
    let result = client.try_approve_refund(&admin, &request_id);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidStatus);
}

#[test]
fn test_reject_refund() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    client.reject_refund(&admin, &request_id);

    let request = client.get_refund_request(&request_id);
    assert_eq!(request.status, RefundStatus::Rejected);
    assert!(request.processed_at.is_some());
}

#[test]
fn test_reject_refund_unauthorized() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);
    let fake_admin = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let result = client.try_reject_refund(&fake_admin, &request_id);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::Unauthorized);
}

#[test]
fn test_process_refund() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.approve_refund(&admin, &request_id);

    client.process_refund(&admin, &request_id);

    let request = client.get_refund_request(&request_id);
    assert_eq!(request.status, RefundStatus::Processed);
    assert!(request.processed_at.is_some());

    let total = client.get_total_refunds();
    assert_eq!(total, 500);
}

#[test]
fn test_process_refund_invalid_status() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    // Not approved yet - should fail

    let result = client.try_process_refund(&admin, &request_id);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidStatus);
}

#[test]
fn test_update_config() {
    let (env, admin, client) = setup();

    client.update_config(&admin, &500, &200);

    let config = client.get_config();
    assert_eq!(config.fee_bps, 500);
    assert_eq!(config.min_refund_amount, 200);
}

#[test]
fn test_update_config_unauthorized() {
    let (env, admin, client) = setup();
    let fake_admin = Address::generate(&env);

    let result = client.try_update_config(&fake_admin, &500, &200);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::Unauthorized);
}

#[test]
fn test_update_config_fee_too_high() {
    let (env, admin, client) = setup();

    let result = client.try_update_config(&admin, &10_001, &200);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::FeeTooHigh);
}

#[test]
fn test_update_config_invalid_amount() {
    let (env, admin, client) = setup();

    let result = client.try_update_config(&admin, &500, &-1);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::InvalidAmount);
}

#[test]
fn test_get_total_refunds() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.approve_refund(&admin, &request_id);
    client.process_refund(&admin, &request_id);

    let total = client.get_total_refunds();
    assert_eq!(total, 500);
}

#[test]
fn test_get_refund_counter() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.request_refund(&requester, &token, &300, &Symbol::new(&env, "test2"));

    let counter = client.get_refund_counter();
    assert_eq!(counter, 2);
}

#[test]
fn test_initialized_event_shape() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);

    client.init(&admin, &250, &100);

    let events = env.events().all();
    let event = events.last().unwrap();

    let topics = &event.1;
    assert_eq!(topics.len(), 1);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "initialized"));

    let data: InitializedEvent = event.2.into_val(&env);
    assert_eq!(data.admin, admin);
    assert_eq!(data.fee_bps, 250);
    assert_eq!(data.min_refund_amount, 100);
}

#[test]
fn test_refund_requested_event_shape() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);
    let reason = Symbol::new(&env, "test");

    let request_id = client.request_refund(&requester, &token, &500, &reason);

    let events = env.events().all();
    let event = events.last().unwrap();

    let topics = &event.1;
    assert_eq!(topics.len(), 1);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "refund_requested"));

    let data: RefundRequestedEvent = event.2.into_val(&env);
    assert_eq!(data.request_id, request_id);
    assert_eq!(data.requester, requester);
    assert_eq!(data.token, token);
    assert_eq!(data.amount, 500);
    assert_eq!(data.reason, reason);
}

#[test]
fn test_refund_processed_event_shape_for_each_transition() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    client.approve_refund(&admin, &request_id);
    let approved: RefundProcessedEvent = env.events().all().last().unwrap().2.into_val(&env);
    assert_eq!(approved.request_id, request_id);
    assert_eq!(approved.processor, admin);
    assert_eq!(approved.amount, 500);
    assert_eq!(approved.status, RefundStatus::Approved);

    client.process_refund(&admin, &request_id);
    let processed: RefundProcessedEvent = env.events().all().last().unwrap().2.into_val(&env);
    assert_eq!(processed.status, RefundStatus::Processed);

    // Reject only applies to a still-Pending request, so exercise it on a
    // second, independent request rather than the one already approved.
    let other_id = client.request_refund(&requester, &token, &200, &Symbol::new(&env, "test2"));
    client.reject_refund(&admin, &other_id);
    let rejected: RefundProcessedEvent = env.events().all().last().unwrap().2.into_val(&env);
    assert_eq!(rejected.request_id, other_id);
    assert_eq!(rejected.status, RefundStatus::Rejected);

    let topics = &env.events().all().last().unwrap().1;
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "refund_processed"));
}

#[test]
fn test_config_updated_event_shape() {
    let (env, admin, client) = setup();

    client.update_config(&admin, &500, &200);

    let events = env.events().all();
    let event = events.last().unwrap();

    let topics = &event.1;
    assert_eq!(topics.len(), 1);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "config_updated"));

    let data: RefundConfigUpdatedEvent = event.2.into_val(&env);
    assert_eq!(data.admin, admin);
    assert_eq!(data.fee_bps, 500);
    assert_eq!(data.min_refund_amount, 200);
}
