#[cfg(test)]
mod settlement_tests {
    extern crate std;

    use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError, StorageKey};
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::{token, Address, BytesN, Env, Error, InvokeError, Symbol, TryFromVal};

    fn setup_contract() -> (Env, Address, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let third_party = Address::generate(&env);
        let token = Address::generate(&env);
        (env, addr, admin, vault, third_party, token)
    }

    fn is_error<V, CE: Into<Error>, E: Into<Error>>(
        result: Result<Result<V, CE>, Result<E, InvokeError>>,
        expected: SettlementError,
    ) -> bool {
        let expected_code = expected as u32;
        match result {
            Err(Ok(e)) => e.into().get_code() == expected_code,
            _ => false,
        }
    }

    fn create_usdc<'a>(
        env: &'a Env,
        admin: &Address,
    ) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
        let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
        let address = contract_address.address();
        let client = token::Client::new(env, &address);
        let admin_client = token::StellarAssetClient::new(env, &address);
        (address, client, admin_client)
    }

    #[test]
    fn test_settlement_initialization() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let token = Address::generate(&env);

        client.init(&admin, &vault);

        env.as_contract(&addr, || {
            let inst = env.storage().instance();
            assert!(inst.has(&StorageKey::Admin));
            assert!(inst.has(&StorageKey::Vault));
            assert!(inst.has(&StorageKey::GlobalPool));
            // DeveloperIndex is written lazily on first payment, not at init
        });

        assert_eq!(client.get_admin(), admin);
        assert_eq!(client.get_vault(), vault);

        let global_pool = client.get_global_pool();
        assert_eq!(global_pool.total_balance, 0);
        assert_eq!(global_pool.last_updated, 1_700_000_000);

        let all_balances = client.get_all_developer_balances(&admin, &token);
        assert_eq!(all_balances.len(), 0);
        assert_eq!(client.get_developer_balance(&developer, &token), 0);
    }
    #[test]
    #[should_panic(expected = "invalid config: admin and vault_address must be distinct")]
    fn test_init_admin_equals_vault_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);

        // Passing the same address for admin and vault should be rejected.
        client.init(&admin, &admin);
    }

    #[test]
    #[should_panic(expected = "invalid config: admin cannot be the contract itself")]
    fn test_init_admin_is_contract_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);

        // Passing the contract's own address as admin should be rejected.
        client.init(&addr, &vault);
    }

    #[test]
    #[should_panic(expected = "invalid config: vault_address cannot be the contract itself")]
    fn test_init_vault_is_contract_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);

        // Passing the contract's own address as vault_address should be rejected.
        client.init(&admin, &addr);
    }

    #[test]
    fn test_init_requires_admin_signature() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);

        env.set_auths(&[]);
        let result = client.try_init(&admin, &vault);
        assert!(
            result.is_err(),
            "expected init to require the admin signature"
        );
    }

    #[test]
    fn test_receive_payment_to_pool() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &1000i128, &true, &None, &token);

        let global_pool = client.get_global_pool();
        assert_eq!(global_pool.total_balance, 1000i128);
    }

    #[test]
    fn test_receive_payment_to_developer() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &500i128, &false, &Some(developer.clone()), &token);

        assert_eq!(client.get_developer_balance(&developer, &token), 500i128);
        assert_eq!(client.get_global_pool().total_balance, 0);
    }

    #[test]
    fn test_receive_multiple_payments_accumulate() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &token);
        client.receive_payment(&vault, &150i128, &false, &Some(developer.clone()), &token);

        assert_eq!(client.get_developer_balance(&developer, &token), 250i128);
    }

    #[test]
    fn test_get_developer_balance_when_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let balance = client.get_developer_balance(&developer, &token);
        assert_eq!(balance, 0);
    }

    #[test]
    fn test_get_all_developer_balances_when_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let all = client.get_all_developer_balances(&admin, &token);
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_admin_can_receive_payment_to_pool() {
        // Admin can route payments directly to global pool (not just via vault)
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);
        client.receive_payment(&admin, &100i128, &true, &None, &token);
    }

    #[test]
    fn test_admin_can_receive_payment_to_developer() {
        // Admin routing a payment directly to a developer (not via vault)
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&admin, &200i128, &false, &Some(developer.clone()), &token);

        assert_eq!(client.get_developer_balance(&developer, &token), 200i128);
    }

    #[test]
    fn test_pool_accumulates_across_multiple_payments() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &400i128, &true, &None, &token);
        client.receive_payment(&vault, &600i128, &true, &None, &token);

        assert_eq!(client.get_global_pool().total_balance, 1000i128);
    }

    #[test]
    fn test_get_developer_balance_returns_zero_for_unknown() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let stranger = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        assert_eq!(client.get_developer_balance(&stranger, &token), 0i128);
    }

    #[test]
    fn test_simulate_claim_returns_preview_without_side_effects() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(86_400);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let recipient = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, usdc, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &500i128, &false, &Some(developer.clone()), &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &400i128);
        usdc_admin_client.mint(&addr, &500i128);

        let preview = client
            .try_simulate_claim(&developer, &250i128, &Some(recipient.clone()))
            .unwrap()
            .unwrap();

        assert_eq!(preview.developer, developer);
        assert_eq!(preview.amount, 250i128);
        assert_eq!(preview.recipient, recipient);
        assert_eq!(preview.token, usdc_address);
        assert_eq!(preview.current_balance, 500i128);
        assert_eq!(preview.remaining_balance, 250i128);
        assert_eq!(preview.contract_balance, 500i128);
        assert_eq!(preview.daily_withdraw_cap, 400i128);
        assert_eq!(preview.withdrawn_today, 0i128);
        assert_eq!(preview.withdrawn_today_after, 250i128);

        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 500i128);
        assert_eq!(client.get_withdrawal_today(&developer), 0i128);
        assert_eq!(usdc.balance(&addr), 500i128);
        assert_eq!(usdc.balance(&recipient), 0i128);
    }

    #[test]
    fn test_simulate_claim_reuses_claim_validation_errors() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(50);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &100i128);

        let zero = client.try_simulate_claim(&developer, &0i128, &None);
        assert!(is_error(zero, SettlementError::AmountNotPositive));

        let overdraw = client.try_simulate_claim(&developer, &101i128, &None);
        assert!(is_error(overdraw, SettlementError::InsufficientDeveloperBalance));

        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();
        let closed = client.try_simulate_claim(&developer, &100i128, &None);
        assert!(is_error(closed, SettlementError::ClaimWindowClosed));
    }

    #[test]
    fn test_withdraw_developer_balance_succeeds_exact_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &100i128);

        let result = client.try_withdraw_developer_balance(&developer, &100i128, &None, &usdc_address);
        assert!(result.is_ok());
        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 0i128);
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&addr),
            0i128
        );
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&developer),
            100i128
        );
    }

    #[test]
    fn test_withdraw_developer_balance_rejects_overdraw() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &100i128);

        let result = client.try_withdraw_developer_balance(&developer, &101i128, &None, &usdc_address);
        assert!(result.is_err());
        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 100i128);
    }

    #[test]
    fn test_withdraw_developer_balance_rejects_non_positive_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let token = Address::generate(&env);

        client.init(&admin, &vault);

        let zero_result = client.try_withdraw_developer_balance(&developer, &0i128, &None, &token);
        let negative_result = client.try_withdraw_developer_balance(&developer, &-1i128, &None, &token);

        assert!(zero_result.is_err());
        assert!(negative_result.is_err());
    }

    #[test]
    fn test_withdraw_developer_balance_emits_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &200i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &200i128);

        let result = client.try_withdraw_developer_balance(&developer, &200i128, &None, &usdc_address);
        assert!(result.is_ok());

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "developer_withdraw")
                }
            })
            .expect("expected developer_withdraw event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, developer);

        let data: crate::DeveloperWithdrawEvent = ev.2.into_val(&env);
        assert_eq!(data.developer, developer);
        assert_eq!(data.amount, 200i128);
        assert_eq!(data.remaining_balance, 0i128);
        assert_eq!(data.to, developer);
    }

    #[test]
    fn test_withdraw_developer_balance_to_custodial_address() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let custodial = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &150i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &150i128);

        let result =
            client.try_withdraw_developer_balance(&developer, &150i128, &Some(custodial.clone()));
        assert!(result.is_ok());
        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 0i128);
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&addr),
            0i128
        );
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&custodial),
            150i128
        );
    }

    #[test]
    fn test_withdraw_developer_balance_emits_event_with_custodial_to() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let custodial = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &200i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &200i128);

        let result =
            client.try_withdraw_developer_balance(&developer, &200i128, &Some(custodial.clone()));
        assert!(result.is_ok());

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "developer_withdraw")
                }
            })
            .expect("expected developer_withdraw event");

        let data: crate::DeveloperWithdrawEvent = ev.2.into_val(&env);
        assert_eq!(data.developer, developer);
        assert_eq!(data.amount, 200i128);
        assert_eq!(data.remaining_balance, 0i128);
        assert_eq!(data.to, custodial);
    }

    #[test]
    #[should_panic(expected = "invalid recipient: cannot withdraw to contract itself")]
    fn test_withdraw_developer_balance_rejects_self_address() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &100i128);

        client.withdraw_developer_balance(&developer, &100i128, &Some(addr.clone()), &usdc_address);
    }

    #[test]
    fn test_get_all_developer_balances() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &300i128, &false, &Some(dev1.clone()), &token);
        client.receive_payment(&vault, &200i128, &false, &Some(dev2.clone()), &token);
        client.receive_payment(&vault, &150i128, &false, &Some(dev1.clone()), &token);

        let all = client.get_all_developer_balances(&admin, &token);
        assert_eq!(all.len(), 2);
        let mut dev1_seen = false;
        let mut dev2_seen = false;
        for balance in all.iter() {
            if balance.address == dev1 {
                assert_eq!(balance.balance, 450i128);
                dev1_seen = true;
            } else if balance.address == dev2 {
                assert_eq!(balance.balance, 200i128);
                dev2_seen = true;
            } else {
                panic!("unexpected developer in get_all_developer_balances");
            }
        }
        assert!(dev1_seen);
        assert!(dev2_seen);
    }

    #[test]
    fn test_get_all_developer_balances_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let all = client.get_all_developer_balances(&admin, &token);
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let dev3 = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &100i128, &false, &Some(dev1.clone()), &token);
        client.receive_payment(&vault, &200i128, &false, &Some(dev2.clone()), &token);
        client.receive_payment(&vault, &300i128, &false, &Some(dev3.clone()), &token);

        let page = client.get_developer_balances_page(&admin, &1u32, &2u32, &token);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().address, dev2);
        assert_eq!(page.get(1).unwrap().address, dev3);
    }

    #[test]
    fn test_get_developer_balances_page_respects_limit_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        for _ in 0..105 {
            let developer = Address::generate(&env);
            client.receive_payment(&vault, &1i128, &false, &Some(developer), &token);
        }

        // limit higher than MAX should be capped at MAX_DEVELOPER_BALANCES_PAGE_SIZE (100)
        let page = client.get_developer_balances_page(&admin, &0u32, &200u32, &token);
        assert_eq!(page.len(), 100);
    }

    #[test]
    fn test_get_all_developer_balances_large_index() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        for _ in 0..101 {
            let developer = Address::generate(&env);
            client.receive_payment(&vault, &1i128, &false, &Some(developer), &token);
        }

        let all = client.get_all_developer_balances(&admin, &token);
        assert_eq!(all.len(), 101);
    }

    #[test]
    fn test_set_admin_two_step() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);
        assert_eq!(client.get_admin(), admin); // Still old admin

        client.accept_admin();
        assert_eq!(client.get_admin(), new_admin);
    }

    #[test]
    fn test_get_pending_admin_none_before_nomination() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        assert_eq!(client.get_pending_admin(), None);
    }

    #[test]
    fn test_get_pending_admin_some_after_nomination() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);
        assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));

        // clears after acceptance
        client.accept_admin();
        assert_eq!(client.get_pending_admin(), None);
    }

    #[test]
    #[should_panic(expected = "no admin transfer pending")]
    fn test_accept_admin_fails_if_not_nominated() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.accept_admin();
    }

    #[test]
    fn test_set_admin_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        let result = client.try_set_admin(&vault, &new_admin);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_cancel_admin_transfer_success() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);
        assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));

        client.cancel_admin_transfer(&admin);
        assert_eq!(client.get_pending_admin(), None);

        let events = env.events().all();
        let last_event = events.last().unwrap();
        let event_name = Symbol::try_from_val(&env, &last_event.1.get(0).unwrap()).unwrap();
        assert_eq!(event_name, Symbol::new(&env, "admin_cancelled"));
    }

    #[test]
    fn test_cancel_admin_transfer_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);

        let result = client.try_cancel_admin_transfer(&vault);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    #[should_panic(expected = "no admin transfer pending")]
    fn test_cancel_admin_transfer_no_pending_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.cancel_admin_transfer(&admin);
    }

    #[test]
    fn test_set_vault_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        let attacker = Address::generate(&env);
        let result = client.try_set_vault(&attacker, &new_vault);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_propose_and_accept_vault_happy_path() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        // Step 1: propose by admin
        client.propose_vault(&admin, &new_vault);
        assert_eq!(client.get_vault(), vault); // still old until accepted

        // Step 2: accept by pending vault
        client.accept_vault(&new_vault);
        assert_eq!(client.get_vault(), new_vault);
    }

    #[test]
    fn test_propose_vault_emits_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.propose_vault(&admin, &new_vault);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "vault_proposed")
                }
            })
            .expect("expected vault_proposed event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, admin);

        let data: crate::VaultProposedEvent = ev.2.into_val(&env);
        assert_eq!(data.current_vault, vault);
        assert_eq!(data.proposed_vault, new_vault);
    }

    #[test]
    fn test_accept_vault_emits_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.propose_vault(&admin, &new_vault);
        client.accept_vault(&new_vault);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "vault_accepted")
                }
            })
            .expect("expected vault_accepted event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, new_vault);

        let data: crate::VaultAcceptedEvent = ev.2.into_val(&env);
        assert_eq!(data.old_vault, vault);
        assert_eq!(data.new_vault, new_vault);
        assert_eq!(data.accepted_by, new_vault);
    }

    // â”€â”€ admin rotation edge cases â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_set_admin_to_same_address_succeeds() {
        // Admin can nominate themselves again (useful for re-confirming control)
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &admin);
        // Still current admin until accept
        assert_eq!(client.get_admin(), admin);

        client.accept_admin();
        assert_eq!(client.get_admin(), admin);
    }

    #[test]
    fn test_set_vault_to_same_address_succeeds() {
        // Admin can propose + accept the same vault (no-op but valid)
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.propose_vault(&admin, &vault);
        client.accept_vault(&vault);
        assert_eq!(client.get_vault(), vault);
    }

    #[test]
    fn test_rapid_consecutive_admin_updates() {
        // Admin can change nomination before acceptance
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin1 = Address::generate(&env);
        let new_admin2 = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        // First nomination
        client.set_admin(&admin, &new_admin1);
        // Change nomination before acceptance
        client.set_admin(&admin, &new_admin2);
        // Only second nominee can accept
        client.accept_admin();
        assert_eq!(client.get_admin(), new_admin2);
    }

    #[test]
    fn test_admin_cannot_accept_own_nomination() {
        // Current admin cannot bypass two-step process
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &admin);
        // Admin must still accept to complete transfer
        client.accept_admin();
        assert_eq!(client.get_admin(), admin);
    }

    #[test]
    fn test_pending_admin_cannot_set_admin() {
        // Pending admin has no privileges until accepted
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);
        // New admin tries to set another admin before accepting
        let result = client.try_set_admin(&new_admin, &vault);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_vault_update_after_admin_rotation() {
        // Ensure vault updates work correctly after admin change
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        // Rotate admin
        client.set_admin(&admin, &new_admin);
        client.accept_admin();

        // New admin updates vault
        client.propose_vault(&new_admin, &new_vault);
        client.accept_vault(&new_vault);
        assert_eq!(client.get_vault(), new_vault);
        assert_eq!(client.get_admin(), new_admin);
    }

    #[test]
    fn test_admin_rotation_preserves_state() {
        // Admin rotation doesn't affect pool or balances
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Add some balance
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &token);
        let dev_balance_before = client.get_developer_balance(&developer, &token);
        let pool_before = client.get_global_pool();

        // Rotate admin
        client.set_admin(&admin, &new_admin);
        client.accept_admin();

        // State preserved
        assert_eq!(client.get_developer_balance(&developer, &token), dev_balance_before);
        assert_eq!(
            client.get_global_pool().total_balance,
            pool_before.total_balance
        );
    }

    // â”€â”€ event emission tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_set_admin_emits_nomination_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);

        let events = env.events().all();
        let nom_ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "admin_nominated")
                }
            })
            .expect("expected admin_nominated event");

        let topic_current: Address = nom_ev.1.get(1).unwrap().into_val(&env);
        let topic_new: Address = nom_ev.1.get(2).unwrap().into_val(&env);
        assert_eq!(topic_current, admin);
        assert_eq!(topic_new, new_admin);
    }

    #[test]
    fn test_accept_admin_emits_accepted_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_admin(&admin, &new_admin);
        client.accept_admin();

        let events = env.events().all();
        let acc_ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "admin_accepted")
                }
            })
            .expect("expected admin_accepted event");

        let topic_old: Address = acc_ev.1.get(1).unwrap().into_val(&env);
        let topic_new: Address = acc_ev.1.get(2).unwrap().into_val(&env);
        assert_eq!(topic_old, admin);
        assert_eq!(topic_new, new_admin);
    }

    // â”€â”€ panic / error paths â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_double_init_returns_already_initialized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let result = client.try_init(&admin, &vault);
        assert!(
            is_error(result, SettlementError::AlreadyInitialized),
            "expected AlreadyInitialized"
        );
    }

    #[test]
    fn test_receive_payment_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let result = client.try_receive_payment(&vault, &0i128, &true, &None, &token);
        assert!(is_error(result, SettlementError::AmountNotPositive));
    }

    #[test]
    fn test_receive_payment_negative_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let result = client.try_receive_payment(&vault, &-1i128, &true, &None, &token);
        assert!(is_error(result, SettlementError::AmountNotPositive));
    }

    #[test]
    fn test_receive_payment_to_pool_overflow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        env.as_contract(&addr, || {
            let inst = env.storage().instance();
            let pool = crate::GlobalPool {
                total_balance: i128::MAX,
                last_updated: env.ledger().timestamp(),
            };
            inst.set(&crate::StorageKey::GlobalPool, &pool);
        });

        let result = client.try_receive_payment(&vault, &1i128, &true, &None, &token);
        assert!(is_error(result, SettlementError::PoolOverflow));
    }

    #[test]
    fn test_receive_payment_to_developer_overflow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        env.as_contract(&addr, || {
            env.storage().persistent().set(
                &crate::StorageKey::DeveloperBalance(developer.clone(), token.clone()),
                &i128::MAX,
            );
        });

        let result = client.try_receive_payment(&vault, &1i128, &false, &Some(developer), &token);
        assert!(is_error(result, SettlementError::DeveloperOverflow));
    }

    #[test]
    fn test_receive_payment_pool_false_no_developer() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let result = client.try_receive_payment(&vault, &100i128, &false, &None, &token);
        assert!(is_error(result, SettlementError::DeveloperRequired));
    }

    #[test]
    fn test_receive_payment_pool_true_with_developer() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let result = client.try_receive_payment(&vault, &100i128, &true, &Some(developer), &token);
        assert!(is_error(result, SettlementError::DeveloperMustBeNone));
    }

    #[test]
    fn test_receive_payment_authorization_matrix() {
        enum CallerRole {
            Vault,
            Admin,
            ThirdParty,
        }

        struct Case {
            name: &'static str,
            role: CallerRole,
            should_succeed: bool,
        }

        let cases = [
            Case {
                name: "vault address succeeds",
                role: CallerRole::Vault,
                should_succeed: true,
            },
            Case {
                name: "admin address succeeds",
                role: CallerRole::Admin,
                should_succeed: true,
            },
            Case {
                name: "third party fails",
                role: CallerRole::ThirdParty,
                should_succeed: false,
            },
        ];

        for case in cases {
            let (env, addr, admin, vault, third_party, token) = setup_contract();
            let client = CalloraSettlementClient::new(&env, &addr);
            let caller = match case.role {
                CallerRole::Vault => vault,
                CallerRole::Admin => admin,
                CallerRole::ThirdParty => third_party,
            };

            let result = client.try_receive_payment(&caller, &100i128, &true, &None, &token);

            if case.should_succeed {
                assert!(result.is_ok(), "expected success for case: {}", case.name);
                assert_eq!(client.get_global_pool().total_balance, 100i128);
            } else {
                assert!(
                    is_error(result, SettlementError::Unauthorized),
                    "expected Unauthorized for case: {}",
                    case.name
                );
            }
        }
    }

    // â”€â”€ event shape tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_payment_received_event_to_pool() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &1000i128, &true, &None, &token);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "payment_received")
                }
            })
            .expect("expected payment_received event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, vault);

        let data: crate::PaymentReceivedEvent = ev.2.into_val(&env);
        assert_eq!(data.from_vault, vault);
        assert_eq!(data.amount, 1000i128);
        assert!(data.to_pool);
        assert!(data.developer.is_none());
    }

    /// Snapshot: asserts the full `PaymentReceivedEvent` struct shape and values
    /// in a single comparison, for the to_pool=true branch. Any future change to
    /// the struct's fields or this call's emitted values will fail this test,
    /// making accidental output-structure drift impossible to miss.
    #[test]
    fn test_payment_received_event_snapshot_to_pool() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.receive_payment(&vault, &750i128, &true, &None);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "payment_received")
                }
            })
            .expect("expected payment_received event");

        let data: crate::PaymentReceivedEvent = ev.2.into_val(&env);

        let expected = crate::PaymentReceivedEvent {
            from_vault: vault.clone(),
            amount: 750i128,
            to_pool: true,
            developer: None,
        };

        assert_eq!(data, expected);
    }

    /// Snapshot: asserts the full `PaymentReceivedEvent` struct shape and values
    /// in a single comparison, for the to_pool=false (developer) branch.
    #[test]
    fn test_payment_received_event_snapshot_to_developer() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.receive_payment(&vault, &321i128, &false, &Some(developer.clone()));

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "payment_received")
                }
            })
            .expect("expected payment_received event");

        let data: crate::PaymentReceivedEvent = ev.2.into_val(&env);

        let expected = crate::PaymentReceivedEvent {
            from_vault: vault.clone(),
            amount: 321i128,
            to_pool: false,
            developer: Some(developer.clone()),
        };

        assert_eq!(data, expected);
    }

    #[test]
    fn test_payment_received_and_balance_credited_events_to_developer() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &500i128, &false, &Some(developer.clone()), &token);

        let events = env.events().all();
        let pr_ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "payment_received")
                }
            })
            .expect("expected payment_received event");

        let pr_data: crate::PaymentReceivedEvent = pr_ev.2.into_val(&env);
        assert!(!pr_data.to_pool);
        assert_eq!(pr_data.developer, Some(developer.clone()));

        let bc_ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "balance_credited")
                }
            })
            .expect("expected balance_credited event");

        let topic1: Address = bc_ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, developer);

        let bc_data: crate::BalanceCreditedEvent = bc_ev.2.into_val(&env);
        assert_eq!(bc_data.developer, developer);
        assert_eq!(bc_data.amount, 500i128);
        assert_eq!(bc_data.new_balance, 500i128);
    }

    #[test]
    fn test_balance_credited_new_balance_is_cumulative() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        client.receive_payment(&vault, &300i128, &false, &Some(developer.clone()), &token);
        client.receive_payment(&vault, &200i128, &false, &Some(developer.clone()), &token);

        // grab the last balance_credited event
        let events = env.events().all();
        let bc_ev = events
            .iter()
            .rev()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "balance_credited")
                }
            })
            .expect("expected balance_credited event");

        let bc_data: crate::BalanceCreditedEvent = bc_ev.2.into_val(&env);
        assert_eq!(bc_data.new_balance, 500i128);
    }

    // â”€â”€ regression tests: ensure settlement logic intact after rotation â”€â”€â”€â”€â”€

    #[test]
    fn test_receive_payment_works_after_admin_rotation() {
        // Ensure payment processing still works after admin change
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Rotate admin
        client.set_admin(&admin, &new_admin);
        client.accept_admin();

        // Vault can still send payments
        client.receive_payment(&vault, &1000i128, &true, &None, &token);
        assert_eq!(client.get_global_pool().total_balance, 1000i128);
    }

    #[test]
    fn test_receive_payment_works_after_vault_update() {
        // Ensure payment processing works with new vault address
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Update vault
        client.propose_vault(&admin, &new_vault);
        client.accept_vault(&new_vault);

        // Old vault cannot send payments
        let result = client.try_receive_payment(&vault, &1000i128, &true, &None, &token);
        assert!(is_error(result, SettlementError::Unauthorized));

        // New vault can send payments
        client.receive_payment(&new_vault, &1000i128, &true, &None, &token);
        assert_eq!(client.get_global_pool().total_balance, 1000i128);
    }

    #[test]
    fn test_developer_withdrawal_after_admin_rotation() {
        // Ensure developer balances accessible after admin change
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Credit developer
        client.receive_payment(&vault, &500i128, &false, &Some(developer.clone()), &token);

        // Rotate admin
        client.set_admin(&admin, &new_admin);
        client.accept_admin();

        // Balance still accessible
        assert_eq!(client.get_developer_balance(&developer, &token), 500i128);

        // Admin can still view all balances
        let all_balances = client.get_all_developer_balances(&new_admin, &token);
        assert_eq!(all_balances.len(), 1);
        assert_eq!(all_balances.get(0).unwrap().balance, 500i128);
    }

    #[test]
    fn test_multiple_payments_accumulate_after_vault_update() {
        // Ensure accumulation logic works correctly after vault changes
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Some payments from old vault
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()), &token);

        // Update vault
        client.propose_vault(&admin, &new_vault);
        client.accept_vault(&new_vault);

        // More payments from new vault
        client.receive_payment(&new_vault, &150i128, &false, &Some(developer.clone()), &token);
        client.receive_payment(&new_vault, &200i128, &false, &Some(developer.clone()), &token);

        // Total should accumulate correctly
        assert_eq!(client.get_developer_balance(&developer, &token), 450i128);
    }

    #[test]
    fn test_global_pool_timestamp_updates_after_admin_change() {
        // Ensure pool timestamp updates correctly regardless of admin
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);

        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Initial payment
        client.receive_payment(&vault, &1000i128, &true, &None, &token);
        let pool_before = client.get_global_pool();
        assert_eq!(pool_before.last_updated, 1_700_000_000);

        // Rotate admin and advance time
        client.set_admin(&admin, &new_admin);
        client.accept_admin();
        env.ledger().set_timestamp(1_700_000_100);

        // New payment updates timestamp
        client.receive_payment(&vault, &500i128, &true, &None, &token);
        let pool_after = client.get_global_pool();
        assert_eq!(pool_after.last_updated, 1_700_000_100);
        assert_eq!(pool_after.total_balance, 1500i128);
    }

    /// `last_updated` reflects the ledger timestamp at the moment of each pool credit.
    #[test]
    fn test_global_pool_last_updated_on_receive_payment() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let token = Address::generate(&env);

        env.ledger().set_timestamp(1_000);
        client.init(&admin, &vault);
        assert_eq!(client.get_global_pool().last_updated, 1_000);

        // Advance time and credit pool ï¿½ last_updated must change
        env.ledger().set_timestamp(2_000);
        client.receive_payment(&vault, &100i128, &true, &None, &token);
        let pool = client.get_global_pool();
        assert_eq!(pool.last_updated, 2_000);
        assert_eq!(pool.total_balance, 100i128);

        // Advance again ï¿½ each credit stamps the new time
        env.ledger().set_timestamp(3_000);
        client.receive_payment(&vault, &50i128, &true, &None, &token);
        let pool2 = client.get_global_pool();
        assert_eq!(pool2.last_updated, 3_000);
        assert_eq!(pool2.total_balance, 150i128);
    }

    /// Routing to a developer does NOT update `last_updated` on the global pool.
    #[test]
    fn test_global_pool_last_updated_unchanged_for_developer_payment() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let token = Address::generate(&env);

        env.ledger().set_timestamp(1_000);
        client.init(&admin, &vault);

        env.ledger().set_timestamp(5_000);
        client.receive_payment(&vault, &200i128, &false, &Some(developer.clone()), &token);

        // Pool timestamp must still be the init timestamp
        assert_eq!(client.get_global_pool().last_updated, 1_000);
        assert_eq!(client.get_global_pool().total_balance, 0);
        assert_eq!(client.get_developer_balance(&developer, &token), 200i128);
    }

    // --- Authorization Matrix Tests ---

    #[test]
    fn test_set_admin_authorization_matrix() {
        let (env, addr, admin, vault, third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let new_admin = Address::generate(&env);

        // Admin can set admin
        client.set_admin(&admin, &new_admin);

        // Vault cannot set admin
        let result = client.try_set_admin(&vault, &new_admin);
        assert!(is_error(result, SettlementError::Unauthorized));

        // Third party cannot set admin
        let result = client.try_set_admin(&third_party, &new_admin);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_set_vault_authorization_matrix() {
        let (env, addr, admin, vault, third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let new_vault = Address::generate(&env);

        // Admin can propose vault (set_vault is an alias)
        client.propose_vault(&admin, &new_vault);

        // Vault cannot set vault
        let result = client.try_set_vault(&vault, &new_vault);
        assert!(is_error(result, SettlementError::Unauthorized));

        // Third party cannot set vault
        let result = client.try_set_vault(&third_party, &new_vault);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_accept_vault_rejects_unauthorized_caller() {
        let (env, addr, admin, vault, third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let new_vault = Address::generate(&env);

        client.propose_vault(&admin, &new_vault);
        assert_eq!(client.get_vault(), vault);

        let result = client.try_accept_vault(&third_party);
        assert!(result.is_err());
    }

    #[test]
    fn test_accept_vault_allows_admin_to_finalize() {
        let (env, addr, admin, vault, _third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let new_vault = Address::generate(&env);

        client.propose_vault(&admin, &new_vault);
        assert_eq!(client.get_vault(), vault);

        client.accept_vault(&admin);
        assert_eq!(client.get_vault(), new_vault);
    }

    #[test]
    fn test_propose_vault_rejects_self_address() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        let result = client.try_propose_vault(&admin, &addr);
        assert!(result.is_err());
    }

    #[test]
    fn test_accept_admin_authorization_matrix() {
        let (env, addr, admin, _vault, _third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let new_admin = Address::generate(&env);

        client.set_admin(&admin, &new_admin);

        // Accept for new_admin (using mock_all_auths which is ON from setup_contract)
        client.accept_admin();
        assert_eq!(client.get_admin(), new_admin);
    }

    #[test]
    fn test_get_all_developer_balances_authorization_matrix() {
        let (env, addr, admin, vault, third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);

        // Admin can call
        let _ = client.get_all_developer_balances(&admin, &token);

        // Vault cannot call
        let result = client.try_get_all_developer_balances(&vault, &token);
        assert!(is_error(result, SettlementError::Unauthorized));

        // Third party cannot call
        let result = client.try_get_all_developer_balances(&third_party, &token);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    // ── batch_receive_payment tests ──────────────────────────────────────────

    #[test]
    fn test_batch_receive_payment_credits_multiple_developers() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev1.clone(), 100i128));
        items.push_back((dev2.clone(), 200i128));

        client.batch_receive_payment(&vault, &items, &token);

        assert_eq!(client.get_developer_balance(&dev1, &token), 100i128);
        assert_eq!(client.get_developer_balance(&dev2, &token), 200i128);
    }

    #[test]
    fn test_batch_receive_payment_accumulates_existing_balance() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        client.receive_payment(&vault, &50i128, &false, &Some(dev.clone()), &token);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), 75i128));
        client.batch_receive_payment(&vault, &items, &token);

        assert_eq!(client.get_developer_balance(&dev, &token), 125i128);
    }

    #[test]
    fn test_batch_receive_payment_admin_caller_allowed() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), 300i128));
        client.batch_receive_payment(&admin, &items, &token);

        assert_eq!(client.get_developer_balance(&dev, &token), 300i128);
    }

    #[test]
    fn test_batch_receive_payment_rejects_empty_batch() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);

        let items: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&env);
        let result = client.try_batch_receive_payment(&vault, &items, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_batch_receive_payment_rejects_oversized_batch() {
        use crate::MAX_BATCH_SIZE;
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        for _ in 0..=MAX_BATCH_SIZE {
            items.push_back((dev.clone(), 1i128));
        }
        let result = client.try_batch_receive_payment(&vault, &items, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_batch_receive_payment_rejects_zero_amount() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), 0i128));
        let result = client.try_batch_receive_payment(&vault, &items, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_batch_receive_payment_rejects_negative_amount() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), -1i128));
        let result = client.try_batch_receive_payment(&vault, &items, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_batch_receive_payment_unauthorized_caller_rejected() {
        let (env, addr, _admin, _vault, third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), 100i128));
        let result = client.try_batch_receive_payment(&third_party, &items, &token);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_batch_receive_payment_single_item() {
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let dev = Address::generate(&env);

        let mut items = soroban_sdk::Vec::new(&env);
        items.push_back((dev.clone(), 999i128));
        client.batch_receive_payment(&vault, &items, &token);

        assert_eq!(client.get_developer_balance(&dev, &token), 999i128);
    }

    #[test]
    fn test_batch_receive_payment_max_batch_size_accepted() {
        use crate::MAX_BATCH_SIZE;
        let (env, addr, _admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);

        let mut items = soroban_sdk::Vec::new(&env);
        let mut devs = std::vec::Vec::new();
        for _ in 0..MAX_BATCH_SIZE {
            let dev = Address::generate(&env);
            devs.push(dev.clone());
            items.push_back((dev, 1i128));
        }
        client.batch_receive_payment(&vault, &items, &token);

        for dev in &devs {
            assert_eq!(client.get_developer_balance(dev, &token), 1i128);
        }
    }

    // ── force_credit_developer tests ─────────────────────────────────────────

    #[test]
    fn test_force_credit_developer_happy_path() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);
        let reason = Symbol::new(&env, "offline_settlement");

        client.force_credit_developer(&admin, &developer, &1000i128, &token, &reason);

        assert_eq!(client.get_developer_balance(&developer, &token), 1000i128);
    }

    #[test]
    fn test_force_credit_developer_accumulates() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        client.force_credit_developer(&admin, &developer, &500i128, &Symbol::new(&env, "first"));
        client.force_credit_developer(&admin, &developer, &300i128, &Symbol::new(&env, "second"));

        assert_eq!(client.get_developer_balance(&developer, &token), 800i128);
    }

    #[test]
    fn test_force_credit_developer_unauthorized() {
        let (env, addr, _admin, vault, third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);
        let reason = Symbol::new(&env, "unauthorized_test");

        let vault_result = client.try_force_credit_developer(&vault, &developer, &100i128, &reason);
        assert!(is_error(vault_result, SettlementError::Unauthorized));

        let third_party_result =
            client.try_force_credit_developer(&third_party, &developer, &100i128, &token, &reason);
        assert!(is_error(third_party_result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_force_credit_developer_zero_amount() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let result = client.try_force_credit_developer(
            &admin,
            &developer,
            &0i128,
            &token,
            &Symbol::new(&env, "zero"),
        );
        assert!(is_error(result, SettlementError::AmountNotPositive));
    }

    #[test]
    fn test_force_credit_developer_negative_amount() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let result = client.try_force_credit_developer(
            &admin,
            &developer,
            &-1i128,
            &token,
            &Symbol::new(&env, "negative"),
        );
        assert!(is_error(result, SettlementError::AmountNotPositive));
    }

    #[test]
    fn test_force_credit_developer_emits_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::IntoVal;

        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);
        let reason = Symbol::new(&env, "dispute_resolution");

        client.force_credit_developer(&admin, &developer, &2500i128, &token, &reason);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "developer_force_credited")
                }
            })
            .expect("expected developer_force_credited event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, developer);

        let data: crate::DeveloperForceCreditedEvent = ev.2.into_val(&env);
        assert_eq!(data.developer, developer);
        assert_eq!(data.amount, 2500i128);
        assert_eq!(data.reason, reason);
        assert_eq!(data.new_balance, 2500i128);
    }

    #[test]
    fn test_force_credit_developer_repeated_reason() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer1 = Address::generate(&env);
        let developer2 = Address::generate(&env);
        let reason = Symbol::new(&env, "bulk_reconciliation");

        client.force_credit_developer(&admin, &developer1, &100i128, &token, &reason);
        client.force_credit_developer(&admin, &developer2, &200i128, &token, &reason);

        assert_eq!(client.get_developer_balance(&developer1, &token), 100i128);
        assert_eq!(client.get_developer_balance(&developer2, &token), 200i128);
    }

    #[test]
    fn test_force_credit_developer_overflow() {
        let (env, addr, admin, _vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        env.as_contract(&addr, || {
            env.storage().persistent().set(
                &crate::StorageKey::DeveloperBalance(developer.clone(), token.clone()),
                &i128::MAX,
            );
        });

        let result = client.try_force_credit_developer(
            &admin,
            &developer,
            &1i128,
            &token,
            &Symbol::new(&env, "overflow"),
        );
        assert!(is_error(result, SettlementError::DeveloperOverflow));
    }

    /// Property-based test that drives many randomized receive_payment calls
    /// (mix of to_pool=true / false) and asserts the conservation invariant:
    /// sum of all credits == pool total + sum of all developer balances.
    /// Includes overflow-boundary cases near i128::MAX.
    #[test]
    fn test_conservation_invariant_randomized() {
        let (env, addr, admin, vault, _third_party, token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);

        let mut developers = std::vec::Vec::new();
        for _ in 0..10 {
            developers.push(Address::generate(&env));
        }

        let mut total_credited: i128 = 0;

        // Simple deterministic pseudo-random generator
        let mut seed: u128 = 42;
        let mut next_rand = || {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            seed
        };

        // 1. Run 100 randomized payments with small-to-medium amounts
        for _ in 0..100 {
            let to_pool = (next_rand() % 2) == 0;
            let amount = (next_rand() % 1_000_000) as i128 + 1;

            if to_pool {
                client.receive_payment(&vault, &amount, &true, &None, &token);
            } else {
                let dev_idx = (next_rand() % 10) as usize;
                if let Some(developer) = developers.get(dev_idx) {
                    client.receive_payment(&vault, &amount, &false, &Some(developer.clone()), &token);
                }
            }
            total_credited += amount;
        }

        // 2. Drive towards i128::MAX boundary
        // Calculate remaining room to reach very close to i128::MAX
        let buffer = 1_000_000_000_i128;
        let remaining = i128::MAX - total_credited - buffer;

        if remaining > 0 {
            let half_remaining = remaining / 2;

            // Large credit to pool
            client.receive_payment(&vault, &half_remaining, &true, &None, &token);
            total_credited += half_remaining;

            // Large credit to a developer
            if let Some(developer) = developers.get(0) {
                client.receive_payment(&vault, &half_remaining, &false, &Some(developer.clone()), &token);
                total_credited += half_remaining;
            }
        }

        // Final Invariant Check
        let pool = client.get_global_pool();
        let mut sum_dev_balances: i128 = 0;

        let all_balances = client.get_all_developer_balances(&admin, &token);
        for record in all_balances.iter() {
            sum_dev_balances += record.balance;
        }

        assert_eq!(
            total_credited,
            pool.total_balance + sum_dev_balances,
            "Conservation invariant violated: total credits must equal pool + developer balances"
        );
    }
    #[test]
    fn test_upgrade_and_get_version() {
        let (env, addr, admin, _vault, _third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);

        assert_eq!(client.get_version(), None);

        let new_hash = BytesN::from_array(&env, &[1u8; 32]);
        client.upgrade(&admin, &new_hash);

        assert_eq!(client.get_version(), Some(new_hash.clone()));

        // An `upgraded` event should have been emitted
        let events = env.events().all();
        let ev = events.last().unwrap();
        let name = soroban_sdk::Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap();
        assert_eq!(name, soroban_sdk::Symbol::new(&env, "upgraded"));
    }

    // ── daily withdrawal cap tests ──────────────────────────────────────────

    #[test]
    fn test_withdraw_respects_daily_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &500i128);
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        // First withdrawal of 300 should succeed (under 500 cap)
        let result = client.try_withdraw_developer_balance(&developer, &300i128, &None);
        assert!(result.is_ok());
        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 700i128);

        // Second withdrawal of 300 would push total to 600 (over 500 cap)
        let result = client.try_withdraw_developer_balance(&developer, &300i128, &None);
        assert!(is_error(result, SettlementError::DailyWithdrawCapExceeded));
        assert_eq!(client.get_developer_balance(&developer, &usdc_address), 700i128);
    }

    #[test]
    fn test_daily_cap_accumulates_across_multiple_withdrawals() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &500i128);
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        // Withdraw 200 + 200 = 400, still under 500
        assert!(client
            .try_withdraw_developer_balance(&developer, &200i128, &None)
            .is_ok());
        assert!(client
            .try_withdraw_developer_balance(&developer, &200i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 600i128);

        // Third withdrawal of 100 would push to 500 (exact cap — allowed)
        assert!(client
            .try_withdraw_developer_balance(&developer, &100i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 500i128);

        // Fourth withdrawal of 1 would exceed cap
        let result = client.try_withdraw_developer_balance(&developer, &1i128, &None);
        assert!(is_error(result, SettlementError::DailyWithdrawCapExceeded));
    }

    #[test]
    fn test_daily_cap_zero_means_unlimited() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        // Cap = 0 explicitly means unlimited
        client.set_daily_withdraw_cap(&admin, &developer, &0i128);
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        assert!(client
            .try_withdraw_developer_balance(&developer, &1000i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 0i128);
    }

    #[test]
    fn test_no_cap_set_is_unlimited() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        // No cap set at all — should be unlimited
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        assert!(client
            .try_withdraw_developer_balance(&developer, &1000i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 0i128);
    }

    #[test]
    fn test_daily_cap_resets_on_new_day() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        // Day 0: timestamp = 0
        env.ledger().set_timestamp(0);
        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &500i128);
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        // Withdraw 400 on day 0
        assert!(client
            .try_withdraw_developer_balance(&developer, &400i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 600i128);

        // Another 200 would exceed the 500 cap
        let result = client.try_withdraw_developer_balance(&developer, &200i128, &None);
        assert!(is_error(result, SettlementError::DailyWithdrawCapExceeded));

        // Advance to day 1
        env.ledger().set_timestamp(86400);
        // Mint more USDC for the new day
        usdc_admin_client.mint(&addr, &500i128);

        // Withdrawal should succeed now (cap resets)
        assert!(client
            .try_withdraw_developer_balance(&developer, &500i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 100i128);
    }

    #[test]
    fn test_set_daily_withdraw_cap_unauthorized() {
        let (env, addr, _admin, vault, third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        // Vault cannot set cap
        let result = client.try_set_daily_withdraw_cap(&vault, &developer, &1000i128);
        assert!(is_error(result, SettlementError::Unauthorized));

        // Third party cannot set cap
        let result = client.try_set_daily_withdraw_cap(&third_party, &developer, &1000i128);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_set_daily_withdraw_cap_emits_event() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        client.set_daily_withdraw_cap(&admin, &developer, &1000i128);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "daily_withdraw_cap_changed")
                }
            })
            .expect("expected daily_withdraw_cap_changed event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, admin);

        let data: crate::DailyWithdrawCapChanged = ev.2.into_val(&env);
        assert_eq!(data.developer, developer);
        assert_eq!(data.new_cap, 1000i128);
    }

    #[test]
    fn test_get_daily_withdraw_cap_returns_zero_when_unset() {
        let (env, addr, _admin, _vault, _third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let cap = client.get_daily_withdraw_cap(&developer);
        assert_eq!(cap, 0);
    }

    #[test]
    fn test_get_withdrawal_today_returns_zero_after_no_withdrawals() {
        let (env, addr, _admin, _vault, _third_party, _token) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let today = client.get_withdrawal_today(&developer);
        assert_eq!(today, 0);
    }

    #[test]
    fn test_get_withdrawal_today_tracks_cumulative_withdrawals() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &1000i128);
        client.receive_payment(&vault, &1000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1000i128);

        assert_eq!(client.get_withdrawal_today(&developer), 0i128);

        client.withdraw_developer_balance(&developer, &300i128, &None);
        assert_eq!(client.get_withdrawal_today(&developer), 300i128);

        client.withdraw_developer_balance(&developer, &200i128, &None);
        assert_eq!(client.get_withdrawal_today(&developer), 500i128);
    }

    #[test]
    fn test_daily_cap_does_not_affect_other_developers() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &dev1, &500i128);
        // dev2 has no cap (unlimited)
        client.receive_payment(&vault, &1000i128, &false, &Some(dev1.clone()), &usdc_address);
        client.receive_payment(&vault, &500i128, &false, &Some(dev2.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &1500i128);

        // dev1 hits cap at 500
        assert!(client
            .try_withdraw_developer_balance(&dev1, &300i128, &None)
            .is_ok());
        // Still within cap (300 < 500)
        assert!(client
            .try_withdraw_developer_balance(&dev1, &200i128, &None)
            .is_ok());
        // Exceeds cap (300 + 200 + 1 > 500)
        let result = client.try_withdraw_developer_balance(&dev1, &1i128, &None);
        assert!(is_error(result, SettlementError::DailyWithdrawCapExceeded));

        // dev2 can still withdraw (no cap)
        assert!(client
            .try_withdraw_developer_balance(&dev2, &500i128, &None)
            .is_ok());
    }

    #[test]
    fn test_daily_cap_midnight_rollover() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(86_399);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client.set_daily_withdraw_cap(&admin, &developer, &1000i128);

        client.receive_payment(&vault, &2000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &2000i128);

        // Withdraw up to cap at t = 86_399
        assert!(client
            .try_withdraw_developer_balance(&developer, &1000i128, &None)
            .is_ok());

        // Attempt another withdrawal within the same day, should fail
        let result = client.try_withdraw_developer_balance(&developer, &1i128, &None);
        assert!(is_error(result, SettlementError::DailyWithdrawCapExceeded));

        // Advance to exactly t = 86_400 (midnight)
        env.ledger().set_timestamp(86_400);

        // Withdraw on new day, should succeed since counter is reset
        assert!(client
            .try_withdraw_developer_balance(&developer, &1000i128, &None)
            .is_ok());
    }

    #[test]
    fn test_daily_cap_zero_unlimited_across_rollover() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(86_399);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        // Cap of 0 = unlimited
        client.set_daily_withdraw_cap(&admin, &developer, &0i128);

        client.receive_payment(&vault, &5000i128, &false, &Some(developer.clone()), &usdc_address);
        usdc_admin_client.mint(&addr, &5000i128);

        // Unlimited huge withdraw at t = 86_399
        assert!(client
            .try_withdraw_developer_balance(&developer, &2500i128, &None)
            .is_ok());

        // Advance to exactly t = 86_400
        env.ledger().set_timestamp(86_400);

        // Unlimited huge withdraw at t = 86_400
        assert!(client
            .try_withdraw_developer_balance(&developer, &2500i128, &None)
            .is_ok());
    }

    // ── developer claim window tests ────────────────────────────────────────

    #[test]
    fn test_claim_window_blocks_withdraw_before_start() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(99);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()));
        usdc_admin_client.mint(&addr, &100i128);

        let result = client.try_withdraw_developer_balance(&developer, &100i128, &None);
        assert!(is_error(result, SettlementError::ClaimWindowClosed));
        assert_eq!(client.get_developer_balance(&developer), 100i128);
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&developer),
            0i128
        );
    }

    #[test]
    fn test_claim_window_allows_withdraw_at_start_and_end() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(100);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();
        client.receive_payment(&vault, &200i128, &false, &Some(developer.clone()));
        usdc_admin_client.mint(&addr, &200i128);

        assert!(client
            .try_withdraw_developer_balance(&developer, &50i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 150i128);

        env.ledger().set_timestamp(200);
        assert!(client
            .try_withdraw_developer_balance(&developer, &150i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 0i128);
        assert_eq!(
            token::Client::new(&env, &usdc_address).balance(&developer),
            200i128
        );
    }

    #[test]
    fn test_claim_window_blocks_withdraw_after_end() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(201);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();
        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()));
        usdc_admin_client.mint(&addr, &100i128);

        let result = client.try_withdraw_developer_balance(&developer, &100i128, &None);
        assert!(is_error(result, SettlementError::ClaimWindowClosed));
        assert_eq!(client.get_developer_balance(&developer), 100i128);
    }

    #[test]
    fn test_claim_window_clear_restores_unrestricted_withdraw() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(201);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();
        assert!(client.get_developer_claim_window(&developer).is_some());

        client
            .try_clear_developer_claim_window(&admin, &developer)
            .unwrap()
            .unwrap();
        assert!(client.get_developer_claim_window(&developer).is_none());

        client.receive_payment(&vault, &100i128, &false, &Some(developer.clone()));
        usdc_admin_client.mint(&addr, &100i128);

        assert!(client
            .try_withdraw_developer_balance(&developer, &100i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&developer), 0i128);
    }

    #[test]
    fn test_set_claim_window_rejects_invalid_range() {
        let (env, addr, admin, _vault, _third_party) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let result = client.try_set_developer_claim_window(&admin, &developer, &200u64, &100u64);

        assert!(is_error(result, SettlementError::InvalidClaimWindow));
        assert!(client.get_developer_claim_window(&developer).is_none());
    }

    #[test]
    fn test_set_and_clear_claim_window_unauthorized() {
        let (env, addr, _admin, vault, third_party) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        let result = client.try_set_developer_claim_window(&vault, &developer, &100u64, &200u64);
        assert!(is_error(result, SettlementError::Unauthorized));

        let result =
            client.try_set_developer_claim_window(&third_party, &developer, &100u64, &200u64);
        assert!(is_error(result, SettlementError::Unauthorized));

        let result = client.try_clear_developer_claim_window(&third_party, &developer);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_claim_window_is_per_developer() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(201);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let restricted = Address::generate(&env);
        let unrestricted = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);

        client.init(&admin, &vault);
        client.set_usdc_token(&admin, &usdc_address);
        client
            .try_set_developer_claim_window(&admin, &restricted, &100u64, &200u64)
            .unwrap()
            .unwrap();
        client.receive_payment(&vault, &100i128, &false, &Some(restricted.clone()));
        client.receive_payment(&vault, &100i128, &false, &Some(unrestricted.clone()));
        usdc_admin_client.mint(&addr, &200i128);

        let result = client.try_withdraw_developer_balance(&restricted, &100i128, &None);
        assert!(is_error(result, SettlementError::ClaimWindowClosed));
        assert!(client
            .try_withdraw_developer_balance(&unrestricted, &100i128, &None)
            .is_ok());
        assert_eq!(client.get_developer_balance(&restricted), 100i128);
        assert_eq!(client.get_developer_balance(&unrestricted), 0i128);
    }

    #[test]
    fn test_set_claim_window_emits_event_and_getter_returns_window() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{IntoVal, Symbol};

        let (env, addr, admin, _vault, _third_party) = setup_contract();
        let client = CalloraSettlementClient::new(&env, &addr);
        let developer = Address::generate(&env);

        client
            .try_set_developer_claim_window(&admin, &developer, &100u64, &200u64)
            .unwrap()
            .unwrap();

        let window = client.get_developer_claim_window(&developer).unwrap();
        assert_eq!(window.start_ts, 100u64);
        assert_eq!(window.end_ts, 200u64);

        let events = env.events().all();
        let ev = events
            .iter()
            .find(|e| {
                !e.1.is_empty() && {
                    let t: Symbol = e.1.get(0).unwrap().into_val(&env);
                    t == Symbol::new(&env, "claim_window_changed")
                }
            })
            .expect("expected claim_window_changed event");

        let topic1: Address = ev.1.get(1).unwrap().into_val(&env);
        assert_eq!(topic1, developer);

        let data: crate::DeveloperClaimWindowChanged = ev.2.into_val(&env);
        assert_eq!(data.developer, developer);
        assert_eq!(data.start_ts, 100u64);
        assert_eq!(data.end_ts, 200u64);
        assert!(data.enabled);
    }

    // ── cursor-based pagination tests ────────────────────────────────────────

    /// First page with cursor=None returns up to `limit` records from the
    /// beginning of the sorted index and yields a non-None next_cursor when the
    /// index has more entries.
    #[test]
    fn test_cursor_first_page_returns_records_and_next_cursor() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let dev3 = Address::generate(&env);
        client.receive_payment(&vault, &100i128, &false, &Some(dev1.clone()), &token);
        client.receive_payment(&vault, &200i128, &false, &Some(dev2.clone()), &token);
        client.receive_payment(&vault, &300i128, &false, &Some(dev3.clone()), &token);

        let (page, next) = client.get_developer_balances_cursor(&admin, &None, &2u32, &token);

        assert_eq!(
            page.len(),
            2,
            "first page must contain exactly limit records"
        );
        // next_cursor must point at the last record on this page so the caller
        // can continue from there.
        assert!(
            next.is_some(),
            "next_cursor must be Some when more records exist"
        );
        assert_eq!(
            next.as_ref().unwrap(),
            &page.get(1).unwrap().address,
            "next_cursor must equal the last address on the page"
        );
    }

    /// Subsequent page retrieved via next_cursor returns the remaining records.
    #[test]
    fn test_cursor_subsequent_page_returns_remaining_records() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let dev3 = Address::generate(&env);
        client.receive_payment(&vault, &10i128, &false, &Some(dev1.clone()), &token);
        client.receive_payment(&vault, &20i128, &false, &Some(dev2.clone()), &token);
        client.receive_payment(&vault, &30i128, &false, &Some(dev3.clone()), &token);

        // Page 1
        let (page1, next1) = client.get_developer_balances_cursor(&admin, &None, &2u32, &token);
        assert_eq!(page1.len(), 2);
        assert!(next1.is_some());

        // Page 2 — use next_cursor from page 1
        let (page2, next2) = client.get_developer_balances_cursor(&admin, &next1, &2u32);
        assert_eq!(
            page2.len(),
            1,
            "last page must contain the remaining record"
        );
        // Reached the end of the index.
        assert!(next2.is_none(), "next_cursor must be None on the last page");

        // Together the two pages must cover all three developers exactly once.
        let mut all_addrs: std::vec::Vec<Address> = std::vec::Vec::new();
        for r in page1.iter() {
            all_addrs.push(r.address.clone());
        }
        for r in page2.iter() {
            all_addrs.push(r.address.clone());
        }
        assert_eq!(all_addrs.len(), 3);
        assert!(all_addrs.contains(&dev1));
        assert!(all_addrs.contains(&dev2));
        assert!(all_addrs.contains(&dev3));
    }

    /// Cursor pointing past the last entry returns an empty page and None.
    #[test]
    fn test_cursor_past_last_entry_returns_empty_page() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        client.receive_payment(&vault, &1i128, &false, &Some(dev1.clone()), &token);
        client.receive_payment(&vault, &2i128, &false, &Some(dev2.clone()), &token);

        // Exhaust the index with a large limit to find the last cursor.
        let (full_page, last_cursor) = client.get_developer_balances_cursor(&admin, &None, &100u32, &token);
        assert_eq!(full_page.len(), 2);
        assert!(last_cursor.is_none());

        // Use the address of the last record as the cursor — nothing should follow.
        let last_addr = full_page.get(full_page.len() - 1).unwrap().address;
        let (empty_page, next) =
            client.get_developer_balances_cursor(&admin, &Some(last_addr), &10u32, &token);
        assert_eq!(empty_page.len(), 0, "page after last cursor must be empty");
        assert!(next.is_none());
    }

    /// Cursor stability: credits to developers that sort **after** the cursor do
    /// not disturb already-returned pages.
    #[test]
    fn test_cursor_stable_across_interleaved_credits() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Pre-populate three developers so the sorted index is stable.
        let dev_a = Address::generate(&env);
        let dev_b = Address::generate(&env);
        let dev_c = Address::generate(&env);
        client.receive_payment(&vault, &1i128, &false, &Some(dev_a.clone()), &token);
        client.receive_payment(&vault, &1i128, &false, &Some(dev_b.clone()), &token);
        client.receive_payment(&vault, &1i128, &false, &Some(dev_c.clone()), &token);

        // Fetch first page (limit=1) to get the cursor.
        let (page1, cursor_after_first) =
            client.get_developer_balances_cursor(&admin, &None, &1u32, &token);
        assert_eq!(page1.len(), 1);
        let first_addr = page1.get(0).unwrap().address.clone();

        // Credit the first developer again — this must NOT shift remaining pages.
        client.receive_payment(&vault, &999i128, &false, &Some(first_addr.clone()), &token);

        // Continue pagination from the saved cursor.
        let (page2, _) = client.get_developer_balances_cursor(&admin, &cursor_after_first, &10u32);
        assert_eq!(page2.len(), 2, "two records must remain after the cursor");

        // The first developer must not appear again in page2.
        for rec in page2.iter() {
            assert_ne!(
                rec.address, first_addr,
                "already-paged developer must not re-appear"
            );
        }
    }

    /// Limit is capped at MAX_DEVELOPER_BALANCES_PAGE_SIZE even if caller
    /// passes a larger value.
    #[test]
    fn test_cursor_limit_is_capped_at_max_page_size() {
        use crate::MAX_DEVELOPER_BALANCES_PAGE_SIZE;
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Insert more developers than MAX_DEVELOPER_BALANCES_PAGE_SIZE.
        for _ in 0..(MAX_DEVELOPER_BALANCES_PAGE_SIZE + 10) {
            let dev = Address::generate(&env);
            client.receive_payment(&vault, &1i128, &false, &Some(dev), &token);
        }

        // Request more than the cap.
        let (page, _) = client.get_developer_balances_cursor(
            &admin,
            &None,
            &(MAX_DEVELOPER_BALANCES_PAGE_SIZE + 50),
        );
        assert_eq!(
            page.len(),
            MAX_DEVELOPER_BALANCES_PAGE_SIZE,
            "page must be capped at MAX_DEVELOPER_BALANCES_PAGE_SIZE"
        );
    }

    /// limit=0 returns an empty page and None immediately.
    #[test]
    fn test_cursor_zero_limit_returns_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev = Address::generate(&env);
        client.receive_payment(&vault, &1i128, &false, &Some(dev), &token);

        let (page, next) = client.get_developer_balances_cursor(&admin, &None, &0u32, &token);
        assert_eq!(page.len(), 0);
        assert!(next.is_none());
    }

    /// Cursor on an empty index returns an empty page and None.
    #[test]
    fn test_cursor_empty_index_returns_empty_page() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let (page, next) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
        assert_eq!(page.len(), 0);
        assert!(next.is_none());
    }

    /// Non-admin caller is rejected with Unauthorized.
    #[test]
    fn test_cursor_unauthorized_caller_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let result = client.try_get_developer_balances_cursor(&vault, &None, &10u32, &token);
        assert!(
            is_error(result, SettlementError::Unauthorized),
            "non-admin caller must be rejected with Unauthorized"
        );
    }

    // ── settlement drain queue tests ────────────────────────────────────────

    #[test]
    fn test_queue_enqueue_and_drain_fifo_order() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let dev3 = Address::generate(&env);

        client.enqueue_withdrawal(&vault, &dev1, &100i128, &token);
        client.enqueue_withdrawal(&vault, &dev2, &200i128, &token);
        client.enqueue_withdrawal(&vault, &dev3, &300i128, &token);

        assert_eq!(client.get_queue_len(), 3);

        let drained = client.drain_queue(&admin, &2u32);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained.get(0).unwrap().developer, dev1);
        assert_eq!(drained.get(0).unwrap().amount, 100i128);
        assert_eq!(drained.get(1).unwrap().developer, dev2);
        assert_eq!(drained.get(1).unwrap().amount, 200i128);
        assert_eq!(client.get_queue_len(), 1);
    }

    #[test]
    fn test_queue_partial_drain_preserves_remaining_order() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        let dev1 = Address::generate(&env);
        let dev2 = Address::generate(&env);
        let dev3 = Address::generate(&env);

        client.enqueue_withdrawal(&vault, &dev1, &10i128, &token);
        client.enqueue_withdrawal(&vault, &dev2, &20i128, &token);
        client.enqueue_withdrawal(&vault, &dev3, &30i128, &token);

        let first = client.drain_queue(&admin, &1u32);
        assert_eq!(first.len(), 1);
        assert_eq!(first.get(0).unwrap().developer, dev1);
        assert_eq!(client.get_queue_len(), 2);

        let rest = client.drain_queue(&admin, &10u32);
        assert_eq!(rest.len(), 2);
        assert_eq!(rest.get(0).unwrap().developer, dev2);
        assert_eq!(rest.get(1).unwrap().developer, dev3);
        assert_eq!(client.get_queue_len(), 0);
    }

    #[test]
    fn test_queue_drain_bounded_per_call() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        for _ in 0..(crate::MAX_QUEUE_DRAIN_PER_CALL + 5) {
            let dev = Address::generate(&env);
            client.enqueue_withdrawal(&vault, &dev, &1i128, &token);
        }

        let drained = client.drain_queue(&admin, &u32::MAX);
        assert_eq!(drained.len(), crate::MAX_QUEUE_DRAIN_PER_CALL);
        assert_eq!(
            client.get_queue_len(),
            5,
            "remaining items must stay queued"
        );
    }

    #[test]
    fn test_queue_drain_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);
        let dev = Address::generate(&env);

        client.enqueue_withdrawal(&vault, &dev, &1i128, &token);

        let result = client.try_drain_queue(&vault, &1u32);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    #[test]
    fn test_queue_drain_empty_returns_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);

        let drained = client.drain_queue(&admin, &10u32);
        assert_eq!(drained.len(), 0);
        assert_eq!(client.get_queue_len(), 0);
    }

    #[test]
    fn test_queue_enqueue_rejects_zero_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);
        let dev = Address::generate(&env);

        let result = client.try_enqueue_withdrawal(&vault, &dev, &0i128, &token);
        assert!(is_error(result, SettlementError::AmountNotPositive));
    }

    #[test]
    fn test_queue_enqueue_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let third_party = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);
        let dev = Address::generate(&env);

        let result = client.try_enqueue_withdrawal(&third_party, &dev, &1i128, &token);
        assert!(is_error(result, SettlementError::Unauthorized));
    }

    /// Sorted order: DeveloperIndex stays sorted; cursor pages come out in the
    /// same deterministic order regardless of credit sequence.
    #[test]
    fn test_cursor_sorted_order_is_deterministic() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        let token = Address::generate(&env);

        // Generate addresses in arbitrary order and credit them.
        let mut devs: std::vec::Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
        for dev in &devs {
            client.receive_payment(&vault, &1i128, &false, &Some(dev.clone()), &token);
        }

        // Collect all balances via cursor pagination.
        let mut cursor_pages: std::vec::Vec<Address> = std::vec::Vec::new();
        let mut next: Option<Address> = None;
        loop {
            let (page, nc) = client.get_developer_balances_cursor(&admin, &next, &2u32, &token);
            for r in page.iter() {
                cursor_pages.push(r.address.clone());
            }
            next = nc;
            if next.is_none() {
                break;
            }
        }

#[test]
fn deposit_event_emitted_once_per_batch_item() {
    // items with 3 developers -> assert exactly 3 `deposit` events, one per developer,
    // and that each event's amount matches the corresponding batch item
}

#[test]
fn deposit_event_not_emitted_for_pool_credit() {
    // to_pool = true -> assert NO `deposit` event is published
}

#[test]
fn deposit_event_amount_matches_balance_credited_amount() {
    // amount field in DepositEvent must equal amount field in BalanceCreditedEvent
    // for the same call
}