extern crate std;

use crate::{
    CalloraSettlement, CalloraSettlementClient, StorageKey, PERSISTENT_BUMP_AMOUNT,
    PERSISTENT_BUMP_THRESHOLD,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token as token_mod;
use soroban_sdk::{Address, Env};

fn create_token<'a>(
    env: &'a Env,
    admin: &Address,
) -> (
    Address,
    token_mod::Client<'a>,
    token_mod::StellarAssetClient<'a>,
) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let address = contract_address.address();
    let client = token_mod::Client::new(env, &address);
    let admin_client = token_mod::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

/// Acceptance: two different tokens can be tracked independently for the same developer.
#[test]
fn test_two_tokens_independent_balances() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (token_a, _, _) = create_token(&env, &admin);
    let (token_b, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.add_supported_token(&admin, &token_a);
    client.add_supported_token(&admin, &token_b);

    // Credit token_a to developer
    client.receive_payment(
        &vault,
        &1000i128,
        &false,
        &Some(developer.clone()),
        &token_a,
        &1u32,
    );
    // Credit token_b to developer
    client.receive_payment(
        &vault,
        &2000i128,
        &false,
        &Some(developer.clone()),
        &token_b,
        &2u32,
    );

    // Balances are independent per token
    assert_eq!(client.get_developer_balance(&developer, &token_a), 1000i128);
    assert_eq!(client.get_developer_balance(&developer, &token_b), 2000i128);

    // get_all_developer_balances filters by token
    let all_a = client.get_all_developer_balances(&admin, &token_a);
    assert_eq!(all_a.len(), 1);
    assert_eq!(all_a.get(0).unwrap().balance, 1000i128);
    assert_eq!(all_a.get(0).unwrap().token, token_a);

    let all_b = client.get_all_developer_balances(&admin, &token_b);
    assert_eq!(all_b.len(), 1);
    assert_eq!(all_b.get(0).unwrap().balance, 2000i128);
    assert_eq!(all_b.get(0).unwrap().token, token_b);
}

/// Acceptance: receives independently for two developers across two tokens.
#[test]
fn test_two_tokens_two_developers() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (token_a, _, _) = create_token(&env, &admin);
    let (token_b, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.add_supported_token(&admin, &token_a);
    client.add_supported_token(&admin, &token_b);

    // dev1 gets token_a, dev2 gets token_b
    client.receive_payment(
        &vault,
        &100i128,
        &false,
        &Some(dev1.clone()),
        &token_a,
        &1u32,
    );
    client.receive_payment(
        &vault,
        &200i128,
        &false,
        &Some(dev2.clone()),
        &token_b,
        &1u32,
    );

    assert_eq!(client.get_developer_balance(&dev1, &token_a), 100i128);
    assert_eq!(client.get_developer_balance(&dev2, &token_b), 200i128);
    // Cross-token queries return zero
    assert_eq!(client.get_developer_balance(&dev1, &token_b), 0i128);
    assert_eq!(client.get_developer_balance(&dev2, &token_a), 0i128);
}

/// Acceptance: withdraw asserts token — can only withdraw the same token as credited.
#[test]
fn test_withdraw_asserts_token() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let recipient = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (token_a, _, token_a_sac) = create_token(&env, &admin);
    let (token_b, token_b_client, token_b_sac) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.add_supported_token(&admin, &token_a);
    client.add_supported_token(&admin, &token_b);

    // Credit both tokens to developer
    client.receive_payment(
        &vault,
        &500i128,
        &false,
        &Some(developer.clone()),
        &token_a,
        &1u32,
    );
    client.receive_payment(
        &vault,
        &300i128,
        &false,
        &Some(developer.clone()),
        &token_b,
        &2u32,
    );

    // Fund the settlement contract with both tokens so withdrawal succeeds.
    token_a_sac.mint(&addr, &1000i128);
    token_b_sac.mint(&addr, &1000i128);

    // withdraw_developer_balance always moves the currently-configured USDC
    // token; switching that configuration is how an operator moves which
    // per-developer balance a withdrawal draws from.
    client.set_usdc_token(&admin, &token_a);

    // Withdraw token_a — succeeds, uses token_a's contract
    let result =
        client.try_withdraw_developer_balance(&developer, &200i128, &Some(recipient.clone()));
    assert!(result.is_ok());
    assert_eq!(client.get_developer_balance(&developer, &token_a), 300i128);
    assert_eq!(token_b_client.balance(&recipient), 0i128); // token_b not touched

    client.set_usdc_token(&admin, &token_b);

    // Withdraw token_b — succeeds, uses token_b's contract
    let result =
        client.try_withdraw_developer_balance(&developer, &100i128, &Some(recipient.clone()));
    assert!(result.is_ok());
    assert_eq!(client.get_developer_balance(&developer, &token_b), 200i128);

    // Configured token is still token_b (200 remaining); withdrawing 300
    // exceeds that balance and is rejected.
    let result =
        client.try_withdraw_developer_balance(&developer, &300i128, &Some(recipient.clone()));
    assert!(result.is_err()); // InsufficientDeveloperBalance for token_b

    client.set_usdc_token(&admin, &token_a);

    // Configured token is token_a (300 remaining); withdrawing 301 exceeds it.
    let result =
        client.try_withdraw_developer_balance(&developer, &301i128, &Some(recipient.clone()));
    assert!(result.is_err()); // InsufficientDeveloperBalance for token_a
}

/// Acceptance: migration helper converts legacy single-USDC entry to per-token format.
#[test]
fn test_migrate_developer_balance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (usdc, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc);

    // Write a legacy single-token balance using the old storage key directly
    env.as_contract(&addr, || {
        let legacy_key = StorageKey::DeveloperBalanceV1(developer.clone());
        env.storage().persistent().set(&legacy_key, &999i128);
        env.storage().persistent().extend_ttl(
            &legacy_key,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    });

    // Before migration, new per-token read returns 0
    assert_eq!(
        client.get_developer_balance(&developer, &usdc),
        0i128,
        "legacy balance not yet migrated"
    );

    // Run migration
    let result = client.try_migrate_single_dev_v2(&admin, &developer);
    assert!(result.is_ok(), "migration should succeed");

    // After migration, new per-token read returns the migrated value
    assert_eq!(
        client.get_developer_balance(&developer, &usdc),
        999i128,
        "migrated balance should be 999"
    );

    // Legacy entry is removed
    let legacy_exists = env.as_contract(&addr, || {
        env.storage()
            .persistent()
            .has(&StorageKey::DeveloperBalanceV1(developer.clone()))
    });
    assert!(!legacy_exists, "legacy entry should be removed");
}

/// Migration is idempotent — running twice is a no-op.
#[test]
fn test_migrate_developer_balance_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (usdc, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc);

    // Write legacy balance
    env.as_contract(&addr, || {
        env.storage()
            .persistent()
            .set(&StorageKey::DeveloperBalanceV1(developer.clone()), &555i128);
        env.storage().persistent().extend_ttl(
            &StorageKey::DeveloperBalanceV1(developer.clone()),
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    });

    // First migration
    assert!(client
        .try_migrate_developer_balance(&admin, &developer)
        .is_ok());
    assert_eq!(client.get_developer_balance(&developer, &usdc), 555i128);

    // Second migration — idempotent, no error, balance unchanged
    assert!(client
        .try_migrate_developer_balance(&admin, &developer)
        .is_ok());
    assert_eq!(client.get_developer_balance(&developer, &usdc), 555i128);
}

/// Migration requires admin authorization.
#[test]
fn test_migrate_developer_balance_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (usdc, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc);

    // Non-admin tries to migrate
    let attacker = Address::generate(&env);
    let result = client.try_migrate_single_dev_v2(&attacker, &developer);
    assert!(result.is_err());
}

/// Migration fails if USDC token not configured.
#[test]
fn test_migrate_developer_balance_no_usdc() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    client.init(&admin, &vault);
    // Do NOT set USDC token

    let result = client.try_migrate_single_dev_v2(&admin, &developer);
    assert!(result.is_err());
}

/// Batch receive payment per-token works correctly.
#[test]
fn test_batch_receive_payment_with_token() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);

    let (token, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);

    let mut items: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&env);
    client.add_supported_token(&admin, &token);
    items.push_back((dev1.clone(), 100i128));
    items.push_back((dev2.clone(), 200i128));

    client.batch_receive_payment(&vault, &items, &token, &1u32);

    assert_eq!(client.get_developer_balance(&dev1, &token), 100i128);
    assert_eq!(client.get_developer_balance(&dev2, &token), 200i128);
}

// Append tests proving that an unregistered token is rejected by both payment entry points without changing balances, and that configured USDC is backfilled by migrate_v1_to_v2 even when the storage version is already V2.
#[test]
fn test_unsupported_tokens_rejected_for_single_and_batch_payments() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let (token, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);

    let single = client.try_receive_payment(
        &vault,
        &100i128,
        &false,
        &Some(developer.clone()),
        &token,
        &1u32,
    );
    assert!(single.is_err());
    assert_eq!(client.get_developer_balance(&developer, &token), 0i128);

    let items = soroban_sdk::vec![&env, (developer.clone(), 100i128)];
    let batch = client.try_batch_receive_payment(&vault, &items, &token, &1u32);
    assert!(batch.is_err());
    assert_eq!(client.get_developer_balance(&developer, &token), 0i128);
}

#[test]
fn test_migration_registers_configured_usdc_for_existing_instances() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let (usdc, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &usdc);
    client.remove_supported_token(&admin, &usdc);
    env.as_contract(&addr, || {
        env.storage()
            .instance()
            .set(&StorageKey::StorageVersion, &crate::STORAGE_VERSION_V2);
    });

    client.migrate_v1_to_v2(&admin);
    assert!(client.is_supported_token(&usdc));
}

#[test]
fn test_removed_token_blocks_future_credits_and_keeps_existing_balance() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let developer = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let (token, _, _) = create_token(&env, &admin);

    client.init(&admin, &vault);
    client.add_supported_token(&admin, &token);
    client.receive_payment(
        &vault,
        &100i128,
        &false,
        &Some(developer.clone()),
        &token,
        &1u32,
    );
    client.remove_supported_token(&admin, &token);

    let result = client.try_receive_payment(
        &vault,
        &50i128,
        &false,
        &Some(developer.clone()),
        &token,
        &2u32,
    );
    assert!(result.is_err());
    assert_eq!(client.get_developer_balance(&developer, &token), 100i128);
}

#[test]
fn test_supported_token_updates_require_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let vault = Address::generate(&env);
    let addr = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &addr);
    let token = Address::generate(&env);

    client.init(&admin, &vault);
    assert!(client.try_add_supported_token(&attacker, &token).is_err());

    client.add_supported_token(&admin, &token);
    assert!(client.try_remove_supported_token(&attacker, &token).is_err());
    assert!(client.is_supported_token(&token));
}
