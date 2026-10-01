#[cfg(test)]
mod claim_window_edge_tests {
    extern crate std;

    use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError, StorageKey};
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::{Address, Env, Symbol};

    fn setup() -> (Env, CalloraSettlementClient, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let developer = Address::generate(&env);
        let addr = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &addr);
        client.init(&admin, &vault);
        (env, client, admin, developer, vault)
    }

    fn create_usdc(env: &Env, admin: &Address) -> (Address, soroban_sdk::token::Client, soroban_sdk::token::StellarAssetClient) {
        let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
        let address = contract_address.address();
        let client = soroban_sdk::token::Client::new(env, &address);
        let admin_client = soroban_sdk::token::StellarAssetClient::new(env, &address);
        (address, client, admin_client)
    }

    #[test]
    fn test_claim_window_edges() {
        let (env, client, admin, developer, _vault) = setup();
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(
            &_vault,
            100i128,
            false,
            Some(developer.clone()),
            &usdc_address,
            100,
        );
        usdc_admin_client.mint(&client.env().contract_address(), 100i128);
        client.set_developer_claim_window(&admin, &developer, 1_000u64, 2_000u64);

        // At start_ts
        env.ledger().set_timestamp(1_000);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).is_ok());
        // At end_ts
        env.ledger().set_timestamp(2_000);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).is_ok());
        // One second before start
        env.ledger().set_timestamp(999);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).err().unwrap().into().get_code() == SettlementError::ClaimWindowClosed as u32);
        // One second after end
        env.ledger().set_timestamp(2_001);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).err().unwrap().into().get_code() == SettlementError::ClaimWindowClosed as u32);
    }

    #[test]
    fn test_clearing_claim_window_allows_unrestricted_withdrawals() {
        let (env, client, admin, developer, vault) = setup();
        let (usdc_address, _, usdc_admin_client) = create_usdc(&env, &admin);
        client.set_usdc_token(&admin, &usdc_address);
        client.receive_payment(
            &vault,
            100i128,
            false,
            Some(developer.clone()),
            &usdc_address,
            100,
        );
        usdc_admin_client.mint(&client.env().contract_address(), 100i128);
        // Set restrictive window
        client.set_developer_claim_window(&admin, &developer, 0u64, 10u64);
        env.ledger().set_timestamp(5);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).is_ok());
        // Clear window
        client.clear_developer_claim_window(&admin, &developer);
        // Withdraw at any timestamp
        env.ledger().set_timestamp(100);
        assert!(client.try_withdraw_developer_balance(&developer, 10i128, &None).is_ok());
    }
}
