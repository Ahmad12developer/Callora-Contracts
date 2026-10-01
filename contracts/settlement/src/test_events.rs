//! Focused tests for the structured event emission helpers in
//! [`crate::events`].
//!
//! Each test drives the contract through the relevant public function and then
//! inspects `env.events().all()` to assert:
//!
//! - **topic\[0\]** matches the expected `Symbol` string.
//! - **topic\[1\]** (and topic\[2\] where applicable) matches the expected
//!   address or sentinel value.
//! - **data** matches the expected typed payload.
//!
//! Tests are grouped by lifecycle area: init, payments, withdrawals, admin
//! governance, vault rotation, broadcast, upgrade, force-credit, min-balance,
//! and developer balance migration.

#[cfg(test)]
mod event_tests {
    extern crate std;

    use crate::{CalloraSettlement, CalloraSettlementClient};
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::token as token_mod;
    use soroban_sdk::{Address, Env, IntoVal, Symbol};

    // ─── Helpers ─────────────────────────────────────────────────────────────
    #[test]
    fn test_supported_token_registry_events() {
        let (env, contract, admin, _vault, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let token = Address::generate(&env);

        client.add_supported_token(&admin, &token);
        let added = filter_by_topic(&env, &env.events().all(), "supported_token_added");
        assert_eq!(added.len(), 1);
        assert_eq!(topic1_addr(&env, &added[0]), admin);
        assert_eq!(topic2_addr(&env, &added[0]), token);

        env.events().all();
        client.remove_supported_token(&admin, &token);
        let removed = filter_by_topic(&env, &env.events().all(), "supported_token_removed");
        assert_eq!(removed.len(), 1);
        assert_eq!(topic1_addr(&env, &removed[0]), admin);
        assert_eq!(topic2_addr(&env, &removed[0]), token);
    }

    /// Spin up a registered, initialized settlement contract and return
    /// `(env, contract_addr, admin, vault, token)`.
    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let token = Address::generate(&env);
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &contract);
        client.init(&admin, &vault);
        client.add_supported_token(&admin, &token);
        env.events().all();
        (env, contract, admin, vault, token)
    }

    /// Register a Stellar-asset USDC token, mint `amount` to `to`, and return
    /// the token address plus admin client.
    fn create_usdc<'a>(
        env: &'a Env,
        admin: &Address,
    ) -> (Address, token_mod::StellarAssetClient<'a>) {
        let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
        let address = contract_address.address();
        let admin_client = token_mod::StellarAssetClient::new(env, &address);
        (address, admin_client)
    }

    /// Extract the first topic of an event as a `Symbol`.
    fn topic0(
        env: &Env,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Symbol {
        event.1.get(0).unwrap().into_val(env)
    }

    /// Extract the second topic of an event as an `Address`.
    fn topic1_addr(
        env: &Env,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Address {
        event.1.get(1).unwrap().into_val(env)
    }

    /// Extract the second topic of an event as a `String`.
    fn topic1_string(
        env: &Env,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> soroban_sdk::String {
        event.1.get(1).unwrap().into_val(env)
    }

    /// Extract the third topic of an event as an `Address`.
    fn topic2_addr(
        env: &Env,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Address {
        event.1.get(2).unwrap().into_val(env)
    }

    /// Find all events whose topic[0] equals `topic_name`.
    fn filter_by_topic(
        env: &Env,
        events: &soroban_sdk::Vec<(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        )>,
        topic_name: &str,
    ) -> std::vec::Vec<(
        Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )> {
        let expected = Symbol::new(env, topic_name);
        events
            .iter()
            .filter(|e| {
                !e.1.is_empty() && {
                    let sym: Symbol = e.1.get(0).unwrap().into_val(env);
                    sym == expected
                }
            })
            .collect()
    }


    // ─── Price registry ─────────────────────────────────────────────────────

    #[test]
    fn test_set_price_emits_old_and_new_values() {
        let (env, contract, admin, _, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let offering_id = soroban_sdk::String::from_str(&env, "offering-1");
        let first = soroban_sdk::String::from_str(&env, "100");
        let second = soroban_sdk::String::from_str(&env, "200");

        client.set_price(&admin, &offering_id, &first);
        let events = env.events().all();
        let price_events = filter_by_topic(&env, &events, "price_set");
        assert_eq!(price_events.len(), 1);

        let (old, new): (Option<soroban_sdk::String>, soroban_sdk::String) =
            price_events[0].2.into_val(&env);
        assert_eq!(topic1_string(&env, &price_events[0]), offering_id);
        assert!(old.is_none());
        assert_eq!(new, first);

        env.ledger().set_sequence_number(
            env.ledger().sequence() + crate::price_registry::MIN_WRITE_INTERVAL,
        );
        env.events().all();
        client.set_price(&admin, &offering_id, &second);
        let events = env.events().all();
        let price_events = filter_by_topic(&env, &events, "price_set");
        assert_eq!(price_events.len(), 1);

        let (old, new): (Option<soroban_sdk::String>, soroban_sdk::String) =
            price_events[0].2.into_val(&env);
        assert_eq!(old, Some(first));
        assert_eq!(new, second);
    }

    #[test]
    fn test_remove_price_emits_removed_value() {
        let (env, contract, admin, _, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let offering_id = soroban_sdk::String::from_str(&env, "offering-1");
        let price = soroban_sdk::String::from_str(&env, "100");

        client.set_price(&admin, &offering_id, &price);
        env.ledger().set_timestamp(1_700_000_001);
        client.remove_price(&admin, &offering_id);

        let events = env.events().all();
        let price_events = filter_by_topic(&env, &events, "price_removed");
        assert_eq!(price_events.len(), 1);
        assert_eq!(topic1_string(&env, &price_events[0]), offering_id);

        let old: soroban_sdk::String = price_events[0].2.into_val(&env);
        assert_eq!(old, price);
    }

    #[test]
    fn test_remove_missing_price_emits_no_event() {
        let (env, contract, admin, _, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let offering_id = soroban_sdk::String::from_str(&env, "missing");

        client.remove_price(&admin, &offering_id);

        let events = env.events().all();
        let price_events = filter_by_topic(&env, &events, "price_removed");
        assert!(price_events.is_empty());
    }

    // ─── Init ─────────────────────────────────────────────────────────────────

    /// `init` must emit exactly one `initialized` event whose topic[1] is the
    /// admin address and topic[2] is the vault address.
    #[test]
    fn test_init_emits_initialized_event() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_700_000_000);
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let contract = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &contract);

        client.init(&admin, &vault);

        let all = env.events().all();
        let inits = filter_by_topic(&env, &all, "initialized");
        assert_eq!(inits.len(), 1, "expected exactly one 'initialized' event");
        let ev = &inits[0];
        assert_eq!(topic0(&env, ev), Symbol::new(&env, "initialized"));
        assert_eq!(topic1_addr(&env, ev), admin);
        assert_eq!(topic2_addr(&env, ev), vault);
    }

    /// `init` must not emit `initialized` twice (the second call panics before
    /// reaching the emit, so no extra event should appear).
    #[test]
    #[should_panic]
    fn test_double_init_panics() {
        let (env, contract, admin, vault, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        client.init(&admin, &vault);
    }

    // ─── payment_received ─────────────────────────────────────────────────────

    /// Pool payment emits exactly one `payment_received` event with
    /// `to_pool = true` and no developer address.
    #[test]
    fn test_receive_payment_pool_emits_payment_received() {
        let (env, contract, _admin, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.receive_payment(&vault, &1_000i128, &true, &None, &token, &1u32);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "payment_received");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), vault);
    }

    /// Pool payment must NOT emit `balance_credited` or `deposit`.
    #[test]
    fn test_receive_payment_pool_no_credited_or_deposit() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);

        client.receive_payment(&vault, &1_000i128, &true, &None, &token, &1u32);

        let all = env.events().all();
        assert_eq!(
            filter_by_topic(&env, &all, "balance_credited").len(),
            0,
            "pool payment must not emit balance_credited"
        );
        assert_eq!(
            filter_by_topic(&env, &all, "deposit").len(),
            0,
            "pool payment must not emit deposit"
        );
    }

    /// Developer payment emits `payment_received`, `balance_credited`, and
    /// `deposit` — all with the correct subjects.
    #[test]
    fn test_receive_payment_developer_emits_three_events() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let developer = Address::generate(&env);

        client.receive_payment(
            &vault,
            &500i128,
            &false,
            &Some(developer.clone()),
            &token,
            &1u32,
        );

        let all = env.events().all();
        assert_eq!(filter_by_topic(&env, &all, "payment_received").len(), 1);
        assert_eq!(filter_by_topic(&env, &all, "balance_credited").len(), 1);
        assert_eq!(filter_by_topic(&env, &all, "deposit").len(), 1);
    }

    /// `balance_credited` topic[1] is the developer address.
    #[test]
    fn test_receive_payment_balance_credited_subject_is_developer() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let developer = Address::generate(&env);

        client.receive_payment(
            &vault,
            &500i128,
            &false,
            &Some(developer.clone()),
            &token,
            &1u32,
        );

        let all = env.events().all();
        let credited = filter_by_topic(&env, &all, "balance_credited");
        assert_eq!(topic1_addr(&env, &credited[0]), developer);
    }

    /// `deposit` topic[1] is the developer address.
    #[test]
    fn test_receive_payment_deposit_subject_is_developer() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let developer = Address::generate(&env);

        client.receive_payment(
            &vault,
            &500i128,
            &false,
            &Some(developer.clone()),
            &token,
            &1u32,
        );

        let all = env.events().all();
        let deposits = filter_by_topic(&env, &all, "deposit");
        assert_eq!(topic1_addr(&env, &deposits[0]), developer);
    }

    // ─── batch_receive_payment ────────────────────────────────────────────────

    /// Batch of N items emits N `balance_credited` events and N `deposit` events.
    #[test]
    fn test_batch_receive_emits_per_item_events() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let dev_a = Address::generate(&env);
        let dev_b = Address::generate(&env);
        let dev_c = Address::generate(&env);

        let items = soroban_sdk::vec![
            &env,
            (dev_a.clone(), 100i128),
            (dev_b.clone(), 200i128),
            (dev_c.clone(), 300i128),
        ];
        client.batch_receive_payment(&vault, &items, &token, &1u32);

        let all = env.events().all();
        assert_eq!(
            filter_by_topic(&env, &all, "balance_credited").len(),
            3,
            "expected 3 balance_credited events"
        );
        assert_eq!(
            filter_by_topic(&env, &all, "deposit").len(),
            3,
            "expected 3 deposit events"
        );
    }

    /// Batch does NOT emit `payment_received` (that is a single-payment event).
    #[test]
    fn test_batch_receive_no_payment_received_event() {
        let (env, contract, _, vault, token) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let dev = Address::generate(&env);

        let items = soroban_sdk::vec![&env, (dev.clone(), 100i128)];
        client.batch_receive_payment(&vault, &items, &token, &1u32);

        let all = env.events().all();
        assert_eq!(
            filter_by_topic(&env, &all, "payment_received").len(),
            0,
            "batch_receive_payment must not emit payment_received"
        );
    }

    // ─── developer_withdraw ───────────────────────────────────────────────────

    /// `withdraw_developer_balance` emits exactly one `developer_withdraw` event
    /// with topic[1] = developer.
    #[test]
    fn test_withdraw_emits_developer_withdraw_event() {
        let (env, contract, admin, vault, _) = setup();
        let developer = Address::generate(&env);
        let (usdc, usdc_admin) = create_usdc(&env, &admin);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_usdc_token(&admin, &usdc);
        client.receive_payment(
            &vault,
            &1_000i128,
            &false,
            &Some(developer.clone()),
            &usdc,
            &1u32,
        );
        usdc_admin.mint(&contract, &1_000i128);
        // Clear setup events
        env.events().all();

        client.withdraw_developer_balance(&developer, &500i128, &None);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "developer_withdraw");
        assert_eq!(
            evs.len(),
            1,
            "expected exactly one developer_withdraw event"
        );
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    /// A failed withdrawal (insufficient balance) must not emit `developer_withdraw`.
    #[test]
    fn test_failed_withdraw_emits_no_event() {
        let (env, contract, admin, vault, _) = setup();
        let developer = Address::generate(&env);
        let (usdc, _) = create_usdc(&env, &admin);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_usdc_token(&admin, &usdc);
        // credit 100, try to withdraw 200
        client.receive_payment(
            &vault,
            &100i128,
            &false,
            &Some(developer.clone()),
            &usdc,
            &1u32,
        );
        env.events().all(); // clear

        let result = client.try_withdraw_developer_balance(&developer, &200i128, &None);
        assert!(result.is_err(), "should fail with insufficient balance");

        let all = env.events().all();
        assert_eq!(
            filter_by_topic(&env, &all, "developer_withdraw").len(),
            0,
            "no event on failed withdrawal"
        );
    }

    // ─── daily_withdraw_cap_changed ───────────────────────────────────────────

    /// `set_daily_withdraw_cap` emits exactly one `daily_withdraw_cap_changed`
    /// event with topic[1] = caller (admin).
    #[test]
    fn test_set_daily_withdraw_cap_emits_event() {
        let (env, contract, admin, _, _) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_daily_withdraw_cap(&admin, &developer, &5_000i128);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "daily_withdraw_cap_changed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
    }

    // ─── claim_window_changed ─────────────────────────────────────────────────

    /// `set_developer_claim_window` emits `claim_window_changed` with
    /// topic[1] = developer and `enabled = true`.
    #[test]
    fn test_set_claim_window_emits_event_enabled() {
        let (env, contract, admin, _, _) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_claim_window(&admin, &developer, &1_700_000_000u64, &1_800_000_000u64);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "claim_window_changed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    /// `clear_developer_claim_window` emits `claim_window_changed` with
    /// topic[1] = developer and `enabled = false`.
    #[test]
    fn test_clear_claim_window_emits_event_disabled() {
        let (env, contract, admin, _, _) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        // Set first, then clear.
        client.set_developer_claim_window(&admin, &developer, &1_700_000_000u64, &1_800_000_000u64);
        env.events().all(); // clear

        client.clear_developer_claim_window(&admin, &developer);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "claim_window_changed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    // ─── admin_nominated / admin_accepted / admin_cancelled ───────────────────

    /// `set_admin` emits `admin_nominated` with topic[1]=current_admin,
    /// topic[2]=new_admin.
    #[test]
    fn test_set_admin_emits_admin_nominated() {
        let (env, contract, admin, _, _) = setup();
        let new_admin = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_admin(&admin, &new_admin);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_nominated");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
        assert_eq!(topic2_addr(&env, &evs[0]), new_admin);
    }

    /// `accept_admin` emits `admin_accepted` with topic[1]=old_admin,
    /// topic[2]=new_admin.
    #[test]
    fn test_accept_admin_emits_admin_accepted() {
        let (env, contract, admin, _, _) = setup();
        let new_admin = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_admin(&admin, &new_admin);
        env.events().all(); // clear nomination event

        client.accept_admin();

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_accepted");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
        assert_eq!(topic2_addr(&env, &evs[0]), new_admin);
    }

    /// `cancel_admin_transfer` emits `admin_cancelled` with topic[1]=admin.
    #[test]
    fn test_cancel_admin_transfer_emits_admin_cancelled() {
        let (env, contract, admin, _, _) = setup();
        let new_admin = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_admin(&admin, &new_admin);
        env.events().all();

        client.cancel_admin_transfer(&admin);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_cancelled");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
    }

    // ─── vault_proposed / vault_accepted ──────────────────────────────────────

    /// `propose_vault` emits `vault_proposed` with topic[1]=admin.
    #[test]
    fn test_propose_vault_emits_vault_proposed() {
        let (env, contract, admin, _, _) = setup();
        let new_vault = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.propose_vault(&admin, &new_vault);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "vault_proposed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
    }

    /// `accept_vault` emits `vault_accepted` with topic[1]=new_vault.
    #[test]
    fn test_accept_vault_emits_vault_accepted() {
        let (env, contract, admin, _, _) = setup();
        let new_vault = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.propose_vault(&admin, &new_vault);
        env.events().all();

        client.accept_vault(&new_vault);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "vault_accepted");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), new_vault);
    }

    // ─── upgraded ─────────────────────────────────────────────────────────────

    /// `upgrade` emits exactly one `upgraded` event with topic[1]=admin.
    #[test]
    fn test_upgrade_emits_upgraded_event() {
        let (env, contract, admin, _, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let fake_hash = soroban_sdk::BytesN::from_array(&env, &[0u8; 32]);

        client.upgrade(&admin, &fake_hash);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "upgraded");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
    }

    // ─── admin_broadcast ──────────────────────────────────────────────────────

    /// `broadcast` emits exactly one `admin_broadcast` event with topic[1]=admin.
    #[test]
    fn test_broadcast_emits_admin_broadcast_event() {
        use crate::Severity;
        let (env, contract, admin, _, _) = setup();
        let client = CalloraSettlementClient::new(&env, &contract);
        let msg = soroban_sdk::String::from_str(&env, "maintenance window at 03:00 UTC");

        client.broadcast(&admin, &Severity::Info, &msg);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_broadcast");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), admin);
    }

    // ─── developer_force_credited ─────────────────────────────────────────────

    /// `force_credit_developer` emits `developer_force_credited` with
    /// topic[1]=developer.
    #[test]
    fn test_force_credit_emits_developer_force_credited() {
        let (env, contract, admin, _, token) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let reason = Symbol::new(&env, "reconcile");

        client.force_credit_developer(&admin, &developer, &1_000i128, &token, &reason);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "developer_force_credited");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    /// `force_credit_developer` does not emit an event when the amount is
    /// non-positive (panics before reaching the emit).
    #[test]
    #[should_panic]
    fn test_force_credit_zero_amount_panics() {
        let (env, contract, admin, _, token) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        let reason = Symbol::new(&env, "noop");
        client.force_credit_developer(&admin, &developer, &0i128, &token, &reason);
    }

    // ─── developer_min_balance_changed ────────────────────────────────────────

    /// `set_developer_min_balance` emits `developer_min_balance_changed` with
    /// topic[1]=developer.
    #[test]
    fn test_set_min_balance_emits_event() {
        let (env, contract, admin, _, _) = setup();
        let developer = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_developer_min_balance(&admin, &developer, &2_000i128);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "developer_min_balance_changed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    // ─── admin_migration_proposed / admin_migration ───────────────────────────

    /// `propose_balance_migration` emits `admin_migration_proposed` with
    /// topic[1] = from address.
    #[test]
    fn test_propose_migration_emits_event() {
        let (env, contract, admin, vault, _) = setup();
        let (usdc, _) = create_usdc(&env, &admin);
        let developer = Address::generate(&env);
        let replacement = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_usdc_token(&admin, &usdc);
        client.receive_payment(
            &vault,
            &1_000i128,
            &false,
            &Some(developer.clone()),
            &usdc,
            &1u32,
        );
        env.events().all();

        client.propose_balance_migration(&admin, &developer, &replacement);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_migration_proposed");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
    }

    /// `execute_balance_migration` emits `admin_migration` with topic[1]=from,
    /// topic[2]=to after the timelock expires.
    #[test]
    fn test_execute_migration_emits_event() {
        use crate::DEVELOPER_MIGRATION_TIMELOCK_SECONDS;
        let (env, contract, admin, vault, _) = setup();
        let (usdc, _) = create_usdc(&env, &admin);
        let developer = Address::generate(&env);
        let replacement = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);

        client.set_usdc_token(&admin, &usdc);
        client.receive_payment(
            &vault,
            &1_000i128,
            &false,
            &Some(developer.clone()),
            &usdc,
            &1u32,
        );
        client.propose_balance_migration(&admin, &developer, &replacement);
        env.events().all();

        // Fast-forward past the timelock.
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + DEVELOPER_MIGRATION_TIMELOCK_SECONDS + 1);

        client.execute_balance_migration(&admin, &developer);

        let all = env.events().all();
        let evs = filter_by_topic(&env, &all, "admin_migration");
        assert_eq!(evs.len(), 1);
        assert_eq!(topic1_addr(&env, &evs[0]), developer);
        assert_eq!(topic2_addr(&env, &evs[0]), replacement);
    }

    // ─── Event isolation: failed calls emit nothing ────────────────────────────

    /// An unauthorized `set_admin` call panics before emitting any event.
    #[test]
    #[should_panic]
    fn test_unauthorized_set_admin_emits_no_event() {
        let (env, contract, _, _, _) = setup();
        let impostor = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.set_admin(&impostor, &new_admin);
    }

    /// An unauthorized `propose_vault` call panics before emitting any event.
    #[test]
    #[should_panic]
    fn test_unauthorized_propose_vault_emits_no_event() {
        let (env, contract, _, _, _) = setup();
        let impostor = Address::generate(&env);
        let new_vault = Address::generate(&env);
        let client = CalloraSettlementClient::new(&env, &contract);
        client.propose_vault(&impostor, &new_vault);
    }
}
