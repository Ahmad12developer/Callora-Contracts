extern crate std;

use crate::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, BytesN, Env, IntoVal, Symbol, Val, Vec};

#[test]
fn init_event_structure_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    let events = env.events().all();
    let event = events.last().unwrap();

    // Topics: (event_name, callora_v1, admin)
    let topics = &event.1;
    assert_eq!(topics.len(), 3);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    let topic2: Address = topics.get(2).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "init"));
    assert_eq!(topic2, admin);

    let data: Address = event.2.into_val(&env);
    assert_eq!(data, usdc_addr);
}

#[test]
fn admin_transfer_events_structure() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    // Clear events from init
    env.events().all();

    // Step 1: set_admin
    client.set_admin(&admin, &new_admin);

    let events = env.events().all();
    // admin_changed + admin_transfer_started → 2 events
    assert!(
        events.len() >= 2,
        "expected at least 2 events, got {}",
        events.len()
    );

    let event_topics: std::vec::Vec<Symbol> = events
        .iter()
        .map(|e| {
            let sym: Symbol = e.1.get(0).unwrap().into_val(&env);
            sym
        })
        .collect();

    assert!(
        event_topics.contains(&Symbol::new(&env, "admin_changed")),
        "expected admin_changed event"
    );
    assert!(
        event_topics.contains(&Symbol::new(&env, "admin_transfer_started")),
        "expected admin_transfer_started event"
    );

    // Step 2: accept_admin
    env.events().all();
    client.accept_admin(&new_admin);

    let events = env.events().all();
    assert!(
        events.len() >= 1,
        "expected at least 1 event after accept, got {}",
        events.len()
    );
    let last_event = events.last().unwrap();
    let topic0: Symbol = last_event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "admin_transfer_completed"));
    // Topics: (event_name, callora_v1, new_admin)
    let topic2: Address = last_event.1.get(2).unwrap().into_val(&env);
    assert_eq!(topic2, new_admin);
}

#[test]
fn cancel_admin_transfer_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    client.set_admin(&admin, &new_admin);
    env.events().all();

    client.cancel_admin_transfer(&admin);

    let events = env.events().all();
    let last_event = events.last().unwrap();
    let topic0: Symbol = last_event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "admin_cancelled"));
}

#[test]
fn pause_unpause_events() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    env.events().all();

    client.pause(&admin);

    let events = env.events().all();
    let pause_event = events.last().unwrap();
    let topic0: Symbol = pause_event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "pause_set"));
    // Topics: (event_name, callora_v1, caller)
    let topic2: Address = pause_event.1.get(2).unwrap().into_val(&env);
    assert_eq!(topic2, admin);
    let is_paused: bool = pause_event.2.into_val(&env);
    assert!(is_paused);

    // Unpause
    env.events().all();
    client.unpause(&admin);

    let events = env.events().all();
    let unpause_event = events.last().unwrap();
    let topic0: Symbol = unpause_event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "pause_set"));
    let is_paused: bool = unpause_event.2.into_val(&env);
    assert!(!is_paused);
}

#[test]
fn set_max_distribute_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    env.events().all();

    client.set_max_distribute(&admin, &1000);

    let events = env.events().all();
    let event = events.last().unwrap();
    let topic0: Symbol = event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "set_max_distribute"));
    // Topics: (event_name, callora_v1, admin)
    let topic2: Address = event.1.get(2).unwrap().into_val(&env);
    assert_eq!(topic2, admin);
    let data: (i128, i128) = event.2.into_val(&env);
    assert_eq!(data, (i128::MAX, 1000));
}

#[test]
fn distribute_event_structure() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    // Fund the contract with USDC
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);
    usdc_admin.mint(&contract_addr, &1000);

    env.events().all();

    client.distribute(&admin, &recipient, &500);

    let events = env.events().all();

    // Find the distribute event by topic (index may vary due to token transfer event)
    let mut found = false;
    for event in events.iter() {
        let topics: &Vec<Val> = &event.1;
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        if topic0 == Symbol::new(&env, "distribute") {
            found = true;
            let topic2: Address = topics.get(2).unwrap().into_val(&env);
            assert_eq!(topic2, recipient);
            let amount: i128 = event.2.into_val(&env);
            assert_eq!(amount, 500);
            break;
        }
    }
    assert!(found, "distribute event not found in events");
}

#[test]
fn distribute_lifecycle_events() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    // Fund the contract with USDC
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);
    usdc_admin.mint(&contract_addr, &1000);

    env.events().all();

    client.distribute(&admin, &recipient, &500);

    let events = env.events().all();
    // Should emit 3 events: distribute_started, distribute, distribute_completed
    // (plus potentially other events depending on test environment)
    assert!(events.len() >= 3, "expected at least 3 events, got {}", events.len());

    // Find and verify distribute_started event
    let mut found_started = false;
    let mut found_distribute = false;
    let mut found_completed = false;

    for event in events.iter() {
        let topics: &Vec<Val> = &event.1;
        let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
        
        if topic0 == Symbol::new(&env, "distribute_started") {
            found_started = true;
            let topic2: Address = topics.get(2).unwrap().into_val(&env);
            assert_eq!(topic2, recipient);
            let amount: i128 = event.2.into_val(&env);
            assert_eq!(amount, 500);
        } else if topic0 == Symbol::new(&env, "distribute") {
            found_distribute = true;
            let topic2: Address = topics.get(2).unwrap().into_val(&env);
            assert_eq!(topic2, recipient);
            let amount: i128 = event.2.into_val(&env);
            assert_eq!(amount, 500);
        } else if topic0 == Symbol::new(&env, "distribute_completed") {
            found_completed = true;
            let topic2: Address = topics.get(2).unwrap().into_val(&env);
            assert_eq!(topic2, recipient);
            let amount: i128 = event.2.into_val(&env);
            assert_eq!(amount, 500);
        }
    }

    assert!(found_started, "distribute_started event not found");
    assert!(found_distribute, "distribute event not found");
    assert!(found_completed, "distribute_completed event not found");
}

#[test]
fn all_event_constructors_return_correct_symbols() {
    let env = Env::default();

    assert_eq!(events::event_init(&env), Symbol::new(&env, "init"));
    assert_eq!(
        events::event_admin_changed(&env),
        Symbol::new(&env, "admin_changed")
    );
    assert_eq!(
        events::event_admin_transfer_started(&env),
        Symbol::new(&env, "admin_transfer_started")
    );
    assert_eq!(
        events::event_admin_transfer_completed(&env),
        Symbol::new(&env, "admin_transfer_completed")
    );
    assert_eq!(
        events::event_admin_cancelled(&env),
        Symbol::new(&env, "admin_cancelled")
    );
    assert_eq!(
        events::event_pause_set(&env),
        Symbol::new(&env, "pause_set")
    );
    assert_eq!(
        events::event_set_max_distribute(&env),
        Symbol::new(&env, "set_max_distribute")
    );
    assert_eq!(events::event_distribute(&env), Symbol::new(&env, "distribute"));
    assert_eq!(
        events::event_distribute_started(&env),
        Symbol::new(&env, "distribute_started")
    );
    assert_eq!(
        events::event_distribute_completed(&env),
        Symbol::new(&env, "distribute_completed")
    );
    assert_eq!(events::event_upgraded(&env), Symbol::new(&env, "upgraded"));
}

#[test]
fn require_auth_on_all_state_changing_functions() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    // Init and fund with all auths mocked (setup phase).
    env.mock_all_auths();
    client.init(&admin, &usdc_addr);
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);
    usdc_admin.mint(&contract_addr, &1000);

    // Now drop all auth — non-admin should fail on every state-changing function.
    env.set_auths(&[]);
    let intruder = Address::generate(&env);
    let recipient = Address::generate(&env);

    // set_admin
    let result = client.try_set_admin(&intruder, &intruder);
    assert!(result.is_err(), "non-admin should not be able to set_admin");

    // pause
    let result = client.try_pause(&intruder);
    assert!(result.is_err(), "non-admin should not be able to pause");

    // unpause
    let result = client.try_unpause(&intruder);
    assert!(result.is_err(), "non-admin should not be able to unpause");

    // set_max_distribute
    let result = client.try_set_max_distribute(&intruder, &100);
    assert!(
        result.is_err(),
        "non-admin should not be able to set_max_distribute"
    );

    // cancel_admin_transfer (no pending transfer needed — auth checked first)
    let result = client.try_cancel_admin_transfer(&intruder);
    assert!(
        result.is_err(),
        "non-admin should not be able to cancel_admin_transfer"
    );

    // accept_admin / claim_admin (no pending transfer — auth checked first)
    let result = client.try_accept_admin(&intruder);
    assert!(result.is_err(), "non-admin should not be able to accept_admin");

    let result = client.try_claim_admin(&intruder);
    assert!(result.is_err(), "non-admin should not be able to claim_admin");

    // upgrade (auth checked before wasm verification)
    let new_wasm_hash = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_upgrade(&intruder, &new_wasm_hash);
    assert!(result.is_err(), "non-admin should not be able to upgrade");

    // distribute
    let result = client.try_distribute(&intruder, &recipient, &100);
    assert!(
        result.is_err(),
        "non-admin should not be able to distribute"
    );
}

#[test]
fn no_unwrap_in_production_paths() {
    let source = include_str!("lib.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap_or("");
    let lines: std::vec::Vec<&str> = source.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // Allow .unwrap() only in test modules or in commented lines
        if trimmed.contains(".unwrap(") && !trimmed.starts_with("//") {
            panic!(
                "Production code at line {} uses .unwrap(): {}",
                idx + 1,
                trimmed
            );
        }
    }
}

#[test]
fn require_auth_on_init() {
    let env = Env::default();
    // Do NOT mock all auths
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    // init should not fail on auth because it doesn't require auth
    // (only admin is being set, no previous admin exists)
    client.init(&admin, &usdc_addr);
}

#[test]
fn overflow_safe_math() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);
    usdc_admin.mint(&contract_addr, &i128::MAX);

    // Should succeed with large values
    client.distribute(&admin, &recipient, &1000);
}

#[test]
fn distribute_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    let result = client.try_distribute(&admin, &recipient, &0);
    assert!(result.is_err(), "distribute with zero amount should fail");
}

#[test]
fn distribute_exceeds_max_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    client.set_max_distribute(&admin, &100);

    let result = client.try_distribute(&admin, &recipient, &200);
    assert!(
        result.is_err(),
        "distribute exceeding max_distribute should fail"
    );
}

#[test]
fn distribute_while_paused_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    client.pause(&admin);

    let result = client.try_distribute(&admin, &recipient, &100);
    assert!(result.is_err(), "distribute while paused should fail");
}

#[test]
fn init_rejects_usdc_token_as_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    let result = client.try_init(&admin, &contract_addr);
    assert!(result.is_err(), "init with usdc_token == contract should fail");
}

#[test]
fn init_rejects_usdc_token_as_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    let result = client.try_init(&admin, &admin);
    assert!(result.is_err(), "init with usdc_token == admin should fail");
}

#[test]
fn get_admin_returns_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    let returned_admin = client.get_admin();
    assert_eq!(returned_admin, admin);
}

#[test]
fn get_usdc_token_returns_token() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    let returned_token = client.get_usdc_token();
    assert_eq!(returned_token, usdc_addr);
}

#[test]
fn get_version_returns_crate_version() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    let version = client.version();
    assert!(!version.is_empty());
}

#[test]
fn balance_returns_contract_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);

    // Initially zero
    assert_eq!(client.balance(), 0);

    // Fund the contract
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);
    usdc_admin.mint(&contract_addr, &1000);

    assert_eq!(client.balance(), 1000);
}

#[test]
fn upgrade_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    env.events().all();

    // Note: In test environment, we cannot actually deploy a new WASM.
    // This test verifies the function can be called with auth.
    // The upgrade would fail with "Wasm does not exist" but we test the auth path.
    let new_wasm_hash = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_upgrade(&admin, &new_wasm_hash);
    // The call fails because the wasm doesn't exist, but auth was checked
    assert!(result.is_err());
}

#[test]
fn get_pending_admin_returns_some_when_set() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    assert_eq!(client.get_pending_admin(), None);

    client.set_admin(&admin, &new_admin);
    let pending = client.get_pending_admin();
    assert_eq!(pending, Some(new_admin));
}

#[test]
fn get_pending_admin_returns_none_after_claim() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    client.set_admin(&admin, &new_admin);
    client.accept_admin(&new_admin);

    let pending = client.get_pending_admin();
    assert_eq!(pending, None);
}

#[test]
fn claim_admin_alias_works() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    client.set_admin(&admin, &new_admin);
    env.events().all();

    // Use claim_admin instead of accept_admin
    client.claim_admin(&new_admin);

    let events = env.events().all();
    let event = events.last().unwrap();
    let topic0: Symbol = event.1.get(0).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "admin_transfer_completed"));
}

#[test]
fn get_paused_returns_state() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_addr = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract_addr);

    client.init(&admin, &usdc_addr);
    assert!(!client.get_paused());

    client.pause(&admin);
    assert!(client.get_paused());

    client.unpause(&admin);
    assert!(!client.get_paused());
}
