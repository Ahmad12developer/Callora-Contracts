use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Error, InvokeError};

fn is_not_initialized<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
) -> bool {
    let expected = SettlementError::NotInitialized as u32;
    match result {
        Err(Ok(e)) => e.into().get_code() == expected,
        _ => false,
    }
}

#[test]
fn test_get_admin_uninitialized() {
    let env = Env::default();
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    assert!(is_not_initialized(client.try_get_admin()));
}

#[test]
fn test_get_vault_uninitialized() {
    let env = Env::default();
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    assert!(is_not_initialized(client.try_get_vault()));
}

#[test]
fn test_get_global_pool_uninitialized() {
    let env = Env::default();
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    assert!(is_not_initialized(client.try_get_global_pool()));
}

#[test]
fn test_get_developer_balance_uninitialized() {
    let env = Env::default();
    let dev = Address::generate(&env);
    let token = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    assert!(is_not_initialized(
        client.try_get_developer_balance(&dev, &token)
    ));
}

#[test]
fn test_get_all_developer_balances_uninitialized() {
    let env = Env::default();
    env.mock_all_auths();
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let dummy = Address::generate(&env);
    let token = Address::generate(&env);

    assert!(is_not_initialized(
        client.try_get_all_developer_balances(&dummy, &token)
    ));
}

#[test]
fn test_get_developer_balance_returns_zero_when_not_stored() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let dev = Address::generate(&env);

    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    client.init(&admin, &vault);
    let token = Address::generate(&env);

    let balance = client.get_developer_balance(&dev, &token);
    assert_eq!(balance, 0);
}

/// `get_developer_balances_cursor` called before `init` must return
/// `NotInitialized` (it calls `get_admin` internally).
#[test]
fn test_get_developer_balances_cursor_uninitialized() {
    let env = Env::default();
    env.mock_all_auths();
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let dummy = Address::generate(&env);
    let token = Address::generate(&env);

    let result = client.try_get_developer_balances_cursor(&dummy, &None, &10u32, &token);
    assert!(
        is_not_initialized(result),
        "expected NotInitialized before init"
    );
}

#[test]
fn test_pagination_fewer_than_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);
    client.add_supported_token(&admin, &token);

    // 5 developers
    for _ in 0..5 {
        let dev = Address::generate(&env);
        client.receive_payment(&admin, &1000i128, &false, &Some(dev), &token, &1u32);
    }

    // limit 10
    let (page, next_cursor) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
    assert_eq!(page.len(), 5);
    assert!(next_cursor.is_none());
}

#[test]
fn test_pagination_exactly_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);
    client.add_supported_token(&admin, &token);

    // 10 developers
    let mut devs = soroban_sdk::Vec::new(&env);
    for _ in 0..10 {
        let dev = Address::generate(&env);
        client.receive_payment(&admin, &1000i128, &false, &Some(dev.clone()), &token, &1u32);
        devs.push_back(dev);
    }

    // limit 10
    let (page, next_cursor) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
    assert_eq!(page.len(), 10);
    assert!(next_cursor.is_some());

    // Page 2 using next_cursor
    let (page2, next_cursor2) =
        client.get_developer_balances_cursor(&admin, &next_cursor, &10u32, &token);
    assert_eq!(page2.len(), 0);
    assert!(next_cursor2.is_none());
}

#[test]
fn test_pagination_more_than_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);
    client.add_supported_token(&admin, &token);

    // 15 developers
    for _ in 0..15 {
        let dev = Address::generate(&env);
        client.receive_payment(&admin, &1000i128, &false, &Some(dev), &token, &1u32);
    }

    // Page 1: limit 10
    let (page1, cursor1) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
    assert_eq!(page1.len(), 10);
    assert!(cursor1.is_some());

    // Page 2: limit 10
    let (page2, cursor2) = client.get_developer_balances_cursor(&admin, &cursor1, &10u32, &token);
    assert_eq!(page2.len(), 5);
    assert!(cursor2.is_none());
}

#[test]
fn test_pagination_stable_ordering() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);
    client.add_supported_token(&admin, &token);

    for _ in 0..8 {
        let dev = Address::generate(&env);
        client.receive_payment(&admin, &1000i128, &false, &Some(dev), &token, &1u32);
    }

    let (p1_run1, cursor1_run1) =
        client.get_developer_balances_cursor(&admin, &None, &5u32, &token);
    let (p1_run2, cursor1_run2) =
        client.get_developer_balances_cursor(&admin, &None, &5u32, &token);

    assert_eq!(p1_run1.len(), 5);
    assert_eq!(p1_run1, p1_run2);
    assert_eq!(cursor1_run1, cursor1_run2);

    let (p2_run1, cursor2_run1) =
        client.get_developer_balances_cursor(&admin, &cursor1_run1, &5u32, &token);
    let (p2_run2, cursor2_run2) =
        client.get_developer_balances_cursor(&admin, &cursor1_run2, &5u32, &token);

    assert_eq!(p2_run1.len(), 3);
    assert_eq!(p2_run1, p2_run2);
    assert_eq!(cursor2_run1, cursor2_run2);
}

#[test]
fn test_pagination_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);

    let (page, next_cursor) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
    assert_eq!(page.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_pagination_invalid_cursor() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);
    let token = Address::generate(&env);
    client.add_supported_token(&admin, &token);

    for _ in 0..5 {
        let dev = Address::generate(&env);
        client.receive_payment(&admin, &1000i128, &false, &Some(dev), &token, &1u32);
    }

    let invalid_cursor = Some(Address::generate(&env));
    let (page, next_cursor) =
        client.get_developer_balances_cursor(&admin, &invalid_cursor, &10u32, &token);
    assert_eq!(page.len(), 0);
    assert!(next_cursor.is_none());
}

#[test]
fn test_minimum_balance_aliases_are_exposed_and_persisted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let dev = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    client.init(&admin, &vault);

    client.set_minimum_balance(&admin, &dev, &150i128);
    assert_eq!(client.get_minimum_balance(&dev), 150i128);

    client.set_developer_min_balance(&admin, &dev, &250i128);
    assert_eq!(client.get_minimum_balance(&dev), 250i128);
}

#[test]
fn test_get_withdrawal_today_resets_at_rollover_without_write() {
    use crate::{DailyWithdrawState, StorageKey};
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(86_399);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let dev = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    client.init(&admin, &vault);

    // Manually inject state for day 0
    let state = DailyWithdrawState {
        day: 0,
        amount: 500,
    };
    env.as_contract(&addr, || {
        env.storage()
            .persistent()
            .set(&StorageKey::WithdrawalToday(dev.clone()), &state);
    });

    // Check value at 86_399
    assert_eq!(client.get_withdrawal_today(&dev), 500i128);

    // Advance to day 1
    env.ledger().set_timestamp(86_400);

    // View reports 0 seamlessly without write
    assert_eq!(client.get_withdrawal_today(&dev), 0i128);
}
