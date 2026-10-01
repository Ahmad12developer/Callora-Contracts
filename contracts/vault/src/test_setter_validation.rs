extern crate std;
use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, String, Symbol};

fn create_usdc<'a>(env: &'a Env, admin: &'a Address) -> (Address, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (addr.clone(), token::StellarAssetClient::new(env, &addr))
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient) {
    let addr = env.register(CalloraVault, ());
    (addr.clone(), CalloraVaultClient::new(env, &addr))
}

fn setup(env: &Env) -> (Address, CalloraVaultClient, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, _) = create_usdc(env, &admin);
    client.init(
        &admin,
        &usdc,
        &0,
        &admin,
        &1,
        &None,
        &10000000000,
        &soroban_sdk::Address::generate(&env),
    );
    (vault_addr, client, usdc, admin)
}

#[test]
fn set_price_offering_id_too_long() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let long_id = "a".repeat((MAX_OFFERING_ID_LEN + 1) as usize);
    client.set_price(
        &admin,
        &String::from_str(&env, &long_id),
        &String::from_str(&env, "100"),
    );
}

#[test]
fn set_price_zero_price() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    client.set_price(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "0"),
    );
}

#[test]
fn set_price_successful() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    client.set_price(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "1000"),
    );
    // Verify readback
    let stored = client.get_price(&String::from_str(&env, "off1"));
    assert_eq!(stored, Some(String::from_str(&env, "1000")));
    // Verify event emitted (using try call to capture events)
    let events = env.events().all();
    // Find price_set event
    let price_set = events.iter().find(|e| {
        let s: Symbol = e.1.get(0).unwrap().into_val(&env);
        s == Symbol::new(&env, "price_set")
    });
    assert!(price_set.is_some(), "price_set event not emitted");
}

#[test]
fn set_settlement_vault_address_fails() {
    let env = Env::default();
    let (vault_addr, client, _, admin) = setup(&env);
    let result = client.try_set_settlement(&admin, &vault_addr);
    assert_eq!(result, Err(Ok(VaultError::SettlementCannotBeVault)));
}

#[test]
fn set_settlement_usdc_address_fails() {
    let env = Env::default();
    let (_, client, usdc, admin) = setup(&env);
    let result = client.try_set_settlement(&admin, &usdc);
    assert_eq!(result, Err(Ok(VaultError::SettlementCannotBeToken)));
}

#[test]
fn set_settlement_equals_revenue_pool_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let pool = Address::generate(&env);
    // Use propose/accept two-step flow to set revenue pool
    client.propose_revenue_pool(&Some(pool.clone()));
    client.accept_revenue_pool();
    let result = client.try_set_settlement(&admin, &pool);
    assert!(result.is_err());
}

#[test]
fn set_settlement_valid_address_succeeds() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let new_settlement = Address::generate(&env);
    client.set_settlement(&admin, &new_settlement);
    assert_eq!(client.get_settlement(), new_settlement);
}

#[test]
fn set_settlement_emits_event_with_old_and_new() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (vault_addr, vault_client) = {
        let addr = env.register(CalloraVault, ());
        (addr.clone(), CalloraVaultClient::new(&env, &addr))
    };
    let (usdc, _) = {
        let ca = env.register_stellar_asset_contract_v2(admin.clone());
        let addr = ca.address();
        (addr.clone(), soroban_sdk::token::StellarAssetClient::new(&env, &addr))
    };
    let initial_settlement = Address::generate(&env);
    vault_client.init(
        &admin,
        &usdc,
        &0,
        &admin,
        &1,
        &None,
        &10_000_000_000,
        &initial_settlement,
    );

    let new_settlement = Address::generate(&env);
    vault_client.set_settlement(&admin, &new_settlement);

    // Inspect all events and find the `set_settlement` one.
    let all_events = env.events().all();
    let set_settlement_sym = Symbol::new(&env, "set_settlement");
    let version_sym = Symbol::new(&env, "callora.v1");

    let matched = all_events.iter().find(|(_, topics, _)| {
        topics.len() >= 2
            && topics.get(0).map(|t| t == set_settlement_sym.into_val(&env)).unwrap_or(false)
            && topics.get(1).map(|t| t == version_sym.into_val(&env)).unwrap_or(false)
    });
    assert!(matched.is_some(), "set_settlement event not emitted");

    let (_, _, data) = matched.unwrap();
    // data should be (Option<Address>, Address) == (Some(initial_settlement), new_settlement)
    let (old_val, new_val): (Option<Address>, Address) = data.into_val(&env);
    assert_eq!(old_val, Some(initial_settlement));
    assert_eq!(new_val, new_settlement);
}

#[test]
fn set_settlement_exactly_one_event_emitted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (_, vault_client) = {
        let addr = env.register(CalloraVault, ());
        (addr.clone(), CalloraVaultClient::new(&env, &addr))
    };
    let (usdc, _) = {
        let ca = env.register_stellar_asset_contract_v2(admin.clone());
        let addr = ca.address();
        (addr.clone(), soroban_sdk::token::StellarAssetClient::new(&env, &addr))
    };
    let initial_settlement = Address::generate(&env);
    vault_client.init(
        &admin,
        &usdc,
        &0,
        &admin,
        &1,
        &None,
        &10_000_000_000,
        &initial_settlement,
    );

    // Clear events accumulated during init.
    let events_before = env.events().all().len();
    let new_settlement = Address::generate(&env);
    vault_client.set_settlement(&admin, &new_settlement);

    let set_settlement_sym = Symbol::new(&env, "set_settlement");
    let count = env
        .events()
        .all()
        .iter()
        .skip(events_before as usize)
        .filter(|(_, topics, _)| {
            topics
                .get(0)
                .map(|t| t == set_settlement_sym.into_val(&env))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(count, 1, "expected exactly one set_settlement event");
}

#[test]
fn set_settlement_unauthorized_caller_fails() {
    let env = Env::default();
    let (_, client, _, _admin) = setup(&env);
    let not_owner = Address::generate(&env);
    let new_settlement = Address::generate(&env);
    let result = client.try_set_settlement(&not_owner, &new_settlement);
    assert_eq!(result, Err(Ok(VaultError::Unauthorized)));
}

#[test]
fn set_revenue_pool_vault_address_fails() {
    let env = Env::default();
    let (vault_addr, client, _, admin) = setup(&env);
    let result = client.try_set_revenue_pool(&admin, &Some(vault_addr));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// set_metadata input validation (length / charset hardening)
// ---------------------------------------------------------------------------

#[test]
fn set_metadata_empty_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, ""),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_null_byte_in_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off\x00ering"),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_control_char_in_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off\x01ering"),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_leading_space_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, " off1"),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_trailing_space_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1 "),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_whitespace_only_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "   "),
        &String::from_str(&env, "valid"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_metadata_empty_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, ""),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_null_byte_in_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "meta\x00data"),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_control_char_in_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "meta\x1Fdata"),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_leading_space_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, " metadata"),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_trailing_space_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "metadata "),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_whitespace_only_metadata_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_metadata(
        &admin,
        &String::from_str(&env, "off1"),
        &String::from_str(&env, "   "),
    );
    assert_eq!(result, Err(Ok(VaultError::MetadataInvalid)));
}

#[test]
fn set_metadata_exact_max_length_succeeds() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let offering_id = "a".repeat(MAX_OFFERING_ID_LEN as usize);
    let metadata = "b".repeat(MAX_METADATA_LEN as usize);
    let result = client.set_metadata(
        &admin,
        &String::from_str(&env, &offering_id),
        &String::from_str(&env, &metadata),
    );
    assert_eq!(result, String::from_str(&env, &metadata));
}

// ---------------------------------------------------------------------------
// set_price offering_id input validation (length / charset hardening)
// ---------------------------------------------------------------------------

#[test]
fn set_price_empty_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, ""),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_price_null_byte_in_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, "off\x00ering"),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_price_control_char_in_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, "off\x01ering"),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_price_leading_space_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, " off1"),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_price_trailing_space_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, "off1 "),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}

#[test]
fn set_price_whitespace_only_offering_id_fails() {
    let env = Env::default();
    let (_, client, _, admin) = setup(&env);
    let result = client.try_set_price(
        &admin,
        &String::from_str(&env, "   "),
        &String::from_str(&env, "100"),
    );
    assert_eq!(result, Err(Ok(VaultError::OfferingIdInvalid)));
}
