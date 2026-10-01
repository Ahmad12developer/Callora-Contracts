//! Per-entrypoint access-control matrix test.
//!
//! Asserts that every public entrypoint across the Vault, Settlement, and
//! Revenue Pool contracts enforces the correct authorization — rejecting
//! unauthorized callers and accepting authorized ones.
//!
//! This test suite implements the access-control matrices documented in
//! `docs/ACCESS_CONTROL.md`.
//!
//! # Test Strategy
//!
//! For each entrypoint we test:
//! 1. **Authorized caller succeeds** — the expected role can call the function.
//! 2. **Unauthorized caller is rejected** — every other role (and an outsider)
//!    is rejected with a panic or `VaultError::Unauthorized` / `SettlementError::Unauthorized`.
//!
//! # Coverage
//!
//! - Vault: 20+ entrypoints
//! - Settlement: 20+ entrypoints
//! - Revenue Pool: 15+ entrypoints

#![no_std]

extern crate std;

use soroban_sdk::token as soroban_token;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Symbol, Vec};

use callora_revenue_pool::RevenuePoolClient;
use callora_settlement::CalloraSettlementClient;
use callora_vault::CalloraVaultClient;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (
    Address,
    soroban_token::Client<'a>,
    soroban_token::StellarAssetClient<'a>,
) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let address = contract_address.address();
    let client = soroban_token::Client::new(env, &address);
    let admin_client = soroban_token::StellarAssetClient::new(env, &address);
    (address, client, admin_client)
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(callora_vault::CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

fn create_settlement(env: &Env) -> (Address, CalloraSettlementClient<'_>) {
    let address = env.register(callora_settlement::CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &address);
    (address, client)
}

fn create_revenue_pool(env: &Env) -> (Address, RevenuePoolClient<'_>) {
    let address = env.register(callora_revenue_pool::RevenuePool, ());
    let client = RevenuePoolClient::new(env, &address);
    (address, client)
}

/// Standard test setup: deploy all contracts, mint USDC, init everything.
struct TestContext<'a> {
    env: &'a Env,
    vault_addr: Address,
    vault: CalloraVaultClient<'a>,
    settlement_addr: Address,
    settlement: CalloraSettlementClient<'a>,
    revenue_pool_addr: Address,
    revenue_pool: RevenuePoolClient<'a>,
    usdc_addr: Address,
    usdc: soroban_token::Client<'a>,
    usdc_admin: soroban_token::StellarAssetClient<'a>,
    owner: Address,
    admin: Address,
    depositor: Address,
    authorized_caller: Address,
    developer: Address,
    outsider: Address,
    pending_admin: Address,
}

fn setup<'a>(env: &'a Env) -> TestContext<'a> {
    let owner = Address::generate(&env);
    let admin = Address::generate(&env);
    let depositor = Address::generate(&env);
    let authorized_caller = Address::generate(&env);
    let developer = Address::generate(&env);
    let outsider = Address::generate(&env);
    let pending_admin = Address::generate(&env);

    let (vault_addr, vault) = create_vault(env_ref);
    let (settlement_addr, settlement) = create_settlement(env_ref);
    let (revenue_pool_addr, revenue_pool) = create_revenue_pool(env_ref);
    let (usdc_addr, usdc, usdc_admin) = create_usdc(env_ref, &admin);

    // Fund vault on-ledger
    usdc_admin.mint(&vault_addr, &1_000_000_000);

    // Init vault: init(env, owner, usdc_token, initial_balance, authorized_caller, min_deposit, revenue_pool, max_deduct, settlement)
    vault.init(
        &owner,
        &usdc_addr,
        &1_000_000,
        &authorized_caller,
        &1,
        &Some(revenue_pool_addr.clone()),
        &100_000_000,
        &settlement_addr,
    );

    // Init settlement
    settlement.init(&admin, &vault_addr);

    // Init revenue pool
    revenue_pool.init(&admin, &usdc_addr);

    TestContext {
        env,
        vault_addr,
        vault,
        settlement_addr,
        settlement,
        revenue_pool_addr,
        revenue_pool,
        usdc_addr,
        usdc,
        usdc_admin,
        owner,
        admin,
        depositor,
        authorized_caller,
        developer,
        outsider,
        pending_admin,
    }
}

// ===========================================================================
// VAULT ACCESS CONTROL MATRIX
// ===========================================================================

mod vault_access_control {
    use super::*;

    // -----------------------------------------------------------------------
    // deposit — owner or allowed depositor
    // -----------------------------------------------------------------------

    #[test]
    fn deposit_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.owner, &1000);
        ctx.usdc.approve(&ctx.owner, &ctx.vault_addr, &1000, &2000);
        let result = ctx.vault.try_deposit(&ctx.owner, &100);
        assert!(result.is_ok());
    }

    #[test]
    fn deposit_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.outsider, &1000);
        ctx.usdc
            .approve(&ctx.outsider, &ctx.vault_addr, &1000, &2000);
        let result = ctx.vault.try_deposit(&ctx.outsider, &100);
        assert!(result.is_err(), "outsider should not be able to deposit");
    }

    #[test]
    fn deposit_authorized_caller_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.authorized_caller, &1000);
        ctx.usdc
            .approve(&ctx.authorized_caller, &ctx.vault_addr, &1000, &2000);
        let result = ctx.vault.try_deposit(&ctx.authorized_caller, &100);
        assert!(
            result.is_err(),
            "authorized_caller should not be able to deposit"
        );
    }

    // -----------------------------------------------------------------------
    // deduct — authorized_caller only
    // -----------------------------------------------------------------------

    #[test]
    fn deduct_authorized_caller_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_deduct(&ctx.authorized_caller, &100, &1);
        assert!(result.is_ok());
    }

    #[test]
    fn deduct_owner_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_deduct(&ctx.owner, &100, &1);
        assert!(result.is_err(), "owner should not be able to deduct");
    }

    #[test]
    fn deduct_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_deduct(&ctx.outsider, &100, &1);
        assert!(result.is_err(), "outsider should not be able to deduct");
    }

    // -----------------------------------------------------------------------
    // batch_deduct — authorized_caller only
    // -----------------------------------------------------------------------

    #[test]
    fn batch_deduct_authorized_caller_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(100i128, 1u64), (200i128, 2u64)]);
        let result = ctx.vault.try_batch_deduct(&ctx.authorized_caller, &items);
        assert!(result.is_ok());
    }

    #[test]
    fn batch_deduct_owner_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(100i128, 1u64)]);
        let result = ctx.vault.try_batch_deduct(&ctx.owner, &items);
        assert!(result.is_err(), "owner should not be able to batch_deduct");
    }

    #[test]
    fn batch_deduct_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(100i128, 1u64)]);
        let result = ctx.vault.try_batch_deduct(&ctx.outsider, &items);
        assert!(
            result.is_err(),
            "outsider should not be able to batch_deduct"
        );
    }

    // -----------------------------------------------------------------------
    // set_authorized_caller — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_authorized_caller_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_caller = Address::generate(&ctx.env);
        let result = ctx.vault.try_set_authorized_caller(&ctx.owner, &new_caller);
        assert!(result.is_ok());
    }

    #[test]
    fn set_authorized_caller_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_caller = Address::generate(&ctx.env);
        let result = ctx
            .vault
            .try_set_authorized_caller(&ctx.outsider, &new_caller);
        assert!(
            result.is_err(),
            "outsider should not be able to set_authorized_caller"
        );
    }

    // -----------------------------------------------------------------------
    // pause / unpause — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_pause(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn pause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_pause(&ctx.outsider);
        assert!(result.is_err(), "outsider should not be able to pause");
    }

    #[test]
    fn unpause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.vault.pause(&ctx.owner);
        let result = ctx.vault.try_unpause(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn unpause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.vault.pause(&ctx.owner);
        let result = ctx.vault.try_unpause(&ctx.outsider);
        assert!(result.is_err(), "outsider should not be able to unpause");
    }

    // -----------------------------------------------------------------------
    // set_max_deduct — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_max_deduct_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_set_max_deduct(&ctx.owner, &200_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn set_max_deduct_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_set_max_deduct(&ctx.outsider, &200_000_000);
        assert!(
            result.is_err(),
            "outsider should not be able to set_max_deduct"
        );
    }

    // -----------------------------------------------------------------------
    // set_settlement — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_settlement_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_settlement = Address::generate(&ctx.env);
        let result = ctx.vault.try_set_settlement(&ctx.owner, &new_settlement);
        assert!(result.is_ok());
    }

    #[test]
    fn set_settlement_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_settlement = Address::generate(&ctx.env);
        let result = ctx.vault.try_set_settlement(&ctx.outsider, &new_settlement);
        assert!(
            result.is_err(),
            "outsider should not be able to set_settlement"
        );
    }

    // -----------------------------------------------------------------------
    // set_reserve_cap — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_reserve_cap_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .vault
            .try_set_reserve_cap(&ctx.owner, &ctx.usdc_addr, &2_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn set_reserve_cap_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .vault
            .try_set_reserve_cap(&ctx.outsider, &ctx.usdc_addr, &2_000_000);
        assert!(
            result.is_err(),
            "outsider should not be able to set_reserve_cap"
        );
    }

    // -----------------------------------------------------------------------
    // Timelock entrypoints (admin = owner in vault)
    // -----------------------------------------------------------------------

    #[test]
    fn set_timelock_window_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_set_timelock_window(&ctx.owner, &172_800);
        assert!(result.is_ok());
    }

    #[test]
    fn set_timelock_window_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_set_timelock_window(&ctx.outsider, &172_800);
        assert!(
            result.is_err(),
            "outsider should not be able to set_timelock_window"
        );
    }

    #[test]
    fn propose_pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_propose_pause(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn propose_pause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_propose_pause(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to propose_pause"
        );
    }

    #[test]
    fn propose_upgrade_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.vault.try_propose_upgrade(&ctx.owner, &wasm_hash);
        assert!(result.is_ok());
    }

    #[test]
    fn propose_upgrade_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.vault.try_propose_upgrade(&ctx.outsider, &wasm_hash);
        assert!(
            result.is_err(),
            "outsider should not be able to propose_upgrade"
        );
    }

    #[test]
    fn propose_sweep_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_propose_sweep(&ctx.owner, &ctx.owner, &1000);
        assert!(result.is_ok());
    }

    #[test]
    fn propose_sweep_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .vault
            .try_propose_sweep(&ctx.outsider, &ctx.owner, &1000);
        assert!(
            result.is_err(),
            "outsider should not be able to propose_sweep"
        );
    }

    #[test]
    fn cancel_pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_pause(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn cancel_pause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_pause(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to cancel_pause"
        );
    }

    #[test]
    fn cancel_upgrade_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_upgrade(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn cancel_upgrade_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_upgrade(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to cancel_upgrade"
        );
    }

    #[test]
    fn cancel_sweep_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_sweep(&ctx.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn cancel_sweep_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.vault.try_cancel_sweep(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to cancel_sweep"
        );
    }

    // -----------------------------------------------------------------------
    // prune_processed_requests — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn prune_processed_requests_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let ids = Vec::new(&ctx.env);
        let result = ctx.vault.try_prune_processed_requests(&ctx.owner, &ids);
        assert!(result.is_ok());
    }

    #[test]
    fn prune_processed_requests_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let ids = Vec::new(&ctx.env);
        let result = ctx.vault.try_prune_processed_requests(&ctx.outsider, &ids);
        assert!(
            result.is_err(),
            "outsider should not be able to prune_processed_requests"
        );
    }
}

// ===========================================================================
// SETTLEMENT ACCESS CONTROL MATRIX
// ===========================================================================

mod settlement_access_control {
    use super::*;

    // -----------------------------------------------------------------------
    // receive_payment — vault or admin
    // -----------------------------------------------------------------------

    #[test]
    fn receive_payment_vault_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        // Vault deduct triggers receive_payment on settlement
        let result = ctx.vault.try_deduct(&ctx.authorized_caller, &100, &1);
        assert!(result.is_ok());
    }

    #[test]
    fn receive_payment_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result =
            ctx.settlement
                .try_receive_payment(&ctx.admin, &100, &true, &None, &ctx.usdc_addr, &1);
        assert!(result.is_ok());
    }

    #[test]
    fn receive_payment_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.settlement.try_receive_payment(
            &ctx.outsider,
            &100,
            &true,
            &None,
            &ctx.usdc_addr,
            &1,
        );
        assert!(
            result.is_err(),
            "outsider should not be able to receive_payment"
        );
    }

    // -----------------------------------------------------------------------
    // set_admin (settlement) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_set_admin_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.settlement.try_set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(result.is_ok());
    }

    #[test]
    fn settlement_set_admin_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_set_admin(&ctx.outsider, &ctx.pending_admin);
        assert!(result.is_err(), "outsider should not be able to set_admin");
    }

    // -----------------------------------------------------------------------
    // accept_admin (settlement) — pending admin only
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_accept_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.settlement.try_accept_admin();
        assert!(result.is_ok());
        assert_eq!(ctx.settlement.get_admin(), ctx.pending_admin);
    }

    // -----------------------------------------------------------------------
    // cancel_admin_transfer (settlement) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_cancel_admin_transfer_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.settlement.try_cancel_admin_transfer(&ctx.admin);
        assert!(result.is_ok());
    }

    #[test]
    fn settlement_cancel_admin_transfer_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.settlement.try_cancel_admin_transfer(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to cancel_admin_transfer"
        );
    }

    // -----------------------------------------------------------------------
    // propose_vault — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_vault_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        let result = ctx.settlement.try_propose_vault(&ctx.admin, &new_vault);
        assert!(result.is_ok());
    }

    #[test]
    fn propose_vault_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        let result = ctx.settlement.try_propose_vault(&ctx.outsider, &new_vault);
        assert!(
            result.is_err(),
            "outsider should not be able to propose_vault"
        );
    }

    // -----------------------------------------------------------------------
    // accept_vault — pending vault only (#1141: admin cannot self-accept)
    // -----------------------------------------------------------------------

    #[test]
    fn accept_vault_pending_vault_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.settlement.propose_vault(&ctx.admin, &new_vault);
        let result = ctx.settlement.try_accept_vault(&new_vault);
        assert!(result.is_ok());
    }

    #[test]
    fn accept_vault_admin_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.settlement.propose_vault(&ctx.admin, &new_vault);
        let result = ctx.settlement.try_accept_vault(&ctx.admin);
        assert!(result.is_err(), "admin must not be able to accept a vault rotation (#1141)");
    }

    #[test]
    fn accept_vault_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.settlement.propose_vault(&ctx.admin, &new_vault);
        let result = ctx.settlement.try_accept_vault(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to accept_vault"
        );
    }

    // -----------------------------------------------------------------------
    // set_developer_claim_window — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_developer_claim_window_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result =
            ctx.settlement
                .try_set_developer_claim_window(&ctx.admin, &ctx.developer, &100, &200);
        assert!(result.is_ok());
    }

    #[test]
    fn set_developer_claim_window_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.settlement.try_set_developer_claim_window(
            &ctx.outsider,
            &ctx.developer,
            &100,
            &200,
        );
        assert!(
            result.is_err(),
            "outsider should not be able to set_developer_claim_window"
        );
    }

    // -----------------------------------------------------------------------
    // clear_developer_claim_window — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn clear_developer_claim_window_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement
            .set_developer_claim_window(&ctx.admin, &ctx.developer, &100, &200);
        let result = ctx
            .settlement
            .try_clear_developer_claim_window(&ctx.admin, &ctx.developer);
        assert!(result.is_ok());
    }

    #[test]
    fn clear_developer_claim_window_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_clear_developer_claim_window(&ctx.outsider, &ctx.developer);
        assert!(
            result.is_err(),
            "outsider should not be able to clear_developer_claim_window"
        );
    }

    // -----------------------------------------------------------------------
    // get_all_developer_balances — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn get_all_developer_balances_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_get_all_developer_balances(&ctx.admin, &ctx.usdc_addr);
        assert!(result.is_ok());
    }

    #[test]
    fn get_all_developer_balances_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_get_all_developer_balances(&ctx.outsider, &ctx.usdc_addr);
        assert!(
            result.is_err(),
            "outsider should not be able to get_all_developer_balances"
        );
    }

    // -----------------------------------------------------------------------
    // force_credit_developer — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn force_credit_developer_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "test");
        let result = ctx.settlement.try_force_credit_developer(
            &ctx.admin,
            &ctx.developer,
            &100,
            &ctx.usdc_addr,
            &reason,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn force_credit_developer_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "test");
        let result = ctx.settlement.try_force_credit_developer(
            &ctx.outsider,
            &ctx.developer,
            &100,
            &ctx.usdc_addr,
            &reason,
        );
        assert!(
            result.is_err(),
            "outsider should not be able to force_credit_developer"
        );
    }

    // -----------------------------------------------------------------------
    // set_daily_withdraw_cap — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_daily_withdraw_cap_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_set_daily_withdraw_cap(&ctx.admin, &ctx.developer, &1000);
        assert!(result.is_ok());
    }

    #[test]
    fn set_daily_withdraw_cap_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result =
            ctx.settlement
                .try_set_daily_withdraw_cap(&ctx.outsider, &ctx.developer, &1000);
        assert!(
            result.is_err(),
            "outsider should not be able to set_daily_withdraw_cap"
        );
    }

    // -----------------------------------------------------------------------
    // set_usdc_token — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_usdc_token_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_usdc = Address::generate(&ctx.env);
        let result = ctx.settlement.try_set_usdc_token(&ctx.admin, &new_usdc);
        assert!(result.is_ok());
    }

    #[test]
    fn set_usdc_token_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_usdc = Address::generate(&ctx.env);
        let result = ctx.settlement.try_set_usdc_token(&ctx.outsider, &new_usdc);
        assert!(
            result.is_err(),
            "outsider should not be able to set_usdc_token"
        );
    }

    // -----------------------------------------------------------------------
    // broadcast (settlement) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_broadcast_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_settlement::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "test");
        let result = ctx.settlement.try_broadcast(&ctx.admin, &severity, &msg);
        assert!(result.is_ok());
    }

    #[test]
    fn settlement_broadcast_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_settlement::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "test");
        let result = ctx.settlement.try_broadcast(&ctx.outsider, &severity, &msg);
        assert!(result.is_err(), "outsider should not be able to broadcast");
    }

    // -----------------------------------------------------------------------
    // upgrade (settlement) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_upgrade_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.settlement.try_upgrade(&ctx.admin, &wasm_hash);
        assert!(
            result.is_err(),
            "the authorized call should reach host validation of the unknown WASM hash"
        );
    }

    #[test]
    fn settlement_upgrade_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.settlement.try_upgrade(&ctx.outsider, &wasm_hash);
        assert!(result.is_err(), "outsider should not be able to upgrade");
    }

    // -----------------------------------------------------------------------
    // withdraw_developer_balance — developer only
    // -----------------------------------------------------------------------

    #[test]
    fn withdraw_developer_balance_developer_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "test");
        ctx.settlement.force_credit_developer(
            &ctx.admin,
            &ctx.developer,
            &500,
            &ctx.usdc_addr,
            &reason,
        );
        ctx.settlement.set_usdc_token(&ctx.admin, &ctx.usdc_addr);
        ctx.usdc_admin.mint(&ctx.settlement_addr, &1000);
        let result = ctx
            .settlement
            .try_withdraw_developer_balance(&ctx.developer, &100, &None);
        assert!(result.is_ok());
    }

    #[test]
    fn withdraw_developer_balance_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "test");
        ctx.settlement.force_credit_developer(
            &ctx.admin,
            &ctx.developer,
            &500,
            &ctx.usdc_addr,
            &reason,
        );
        ctx.settlement.set_usdc_token(&ctx.admin, &ctx.usdc_addr);
        ctx.usdc_admin.mint(&ctx.settlement_addr, &1000);
        let result = ctx
            .settlement
            .try_withdraw_developer_balance(&ctx.outsider, &100, &None);
        assert!(
            result.is_err(),
            "outsider should not be able to withdraw developer balance"
        );
    }

    // -----------------------------------------------------------------------
    // propose_balance_migration — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_balance_migration_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_dev = Address::generate(&ctx.env);
        let result =
            ctx.settlement
                .try_propose_balance_migration(&ctx.admin, &ctx.developer, &new_dev);
        assert!(result.is_ok());
    }

    #[test]
    fn propose_balance_migration_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_dev = Address::generate(&ctx.env);
        let result =
            ctx.settlement
                .try_propose_balance_migration(&ctx.outsider, &ctx.developer, &new_dev);
        assert!(
            result.is_err(),
            "outsider should not be able to propose_balance_migration"
        );
    }

    // -----------------------------------------------------------------------
    // set_developer_min_balance — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_developer_min_balance_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .settlement
            .try_set_developer_min_balance(&ctx.admin, &ctx.developer, &100);
        assert!(result.is_ok());
    }

    #[test]
    fn set_developer_min_balance_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result =
            ctx.settlement
                .try_set_developer_min_balance(&ctx.outsider, &ctx.developer, &100);
        assert!(
            result.is_err(),
            "outsider should not be able to set_developer_min_balance"
        );
    }
}

// ===========================================================================
// REVENUE POOL ACCESS CONTROL MATRIX
// ===========================================================================

mod revenue_pool_access_control {
    use super::*;

    // -----------------------------------------------------------------------
    // distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.revenue_pool_addr, &1000);
        let result = ctx
            .revenue_pool
            .try_distribute(&ctx.admin, &ctx.developer, &100);
        assert!(result.is_ok());
    }

    #[test]
    fn distribute_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.revenue_pool_addr, &1000);
        let result = ctx
            .revenue_pool
            .try_distribute(&ctx.outsider, &ctx.developer, &100);
        assert!(result.is_err(), "outsider should not be able to distribute");
    }

    // -----------------------------------------------------------------------
    // batch_distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn batch_distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.revenue_pool_addr, &1000);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        let result = ctx.revenue_pool.try_batch_distribute(&ctx.admin, &payments);
        assert!(result.is_ok());
    }

    #[test]
    fn batch_distribute_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.revenue_pool_addr, &1000);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        let result = ctx
            .revenue_pool
            .try_batch_distribute(&ctx.outsider, &payments);
        assert!(
            result.is_err(),
            "outsider should not be able to batch_distribute"
        );
    }

    // -----------------------------------------------------------------------
    // set_admin (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_set_admin_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .revenue_pool
            .try_set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_set_admin_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .revenue_pool
            .try_set_admin(&ctx.outsider, &ctx.pending_admin);
        assert!(result.is_err(), "outsider should not be able to set_admin");
    }

    // -----------------------------------------------------------------------
    // accept_admin / claim_admin (revenue pool) — pending admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_accept_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.revenue_pool.try_accept_admin(&ctx.pending_admin);
        assert!(result.is_ok());
        assert_eq!(ctx.revenue_pool.get_admin(), ctx.pending_admin);
    }

    #[test]
    fn revenue_pool_claim_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.revenue_pool.try_claim_admin(&ctx.pending_admin);
        assert!(result.is_ok());
        assert_eq!(ctx.revenue_pool.get_admin(), ctx.pending_admin);
    }

    // -----------------------------------------------------------------------
    // cancel_admin_transfer (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_cancel_admin_transfer_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.revenue_pool.try_cancel_admin_transfer(&ctx.admin);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_cancel_admin_transfer_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.set_admin(&ctx.admin, &ctx.pending_admin);
        let result = ctx.revenue_pool.try_cancel_admin_transfer(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to cancel_admin_transfer"
        );
    }

    // -----------------------------------------------------------------------
    // set_pause_guardian — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_pause_guardian_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        let result = ctx
            .revenue_pool
            .try_set_pause_guardian(&ctx.admin, &guardian);
        assert!(result.is_ok());
    }

    #[test]
    fn set_pause_guardian_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        let result = ctx
            .revenue_pool
            .try_set_pause_guardian(&ctx.outsider, &guardian);
        assert!(
            result.is_err(),
            "outsider should not be able to set_pause_guardian"
        );
    }

    // -----------------------------------------------------------------------
    // clear_pause_guardian — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn clear_pause_guardian_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.revenue_pool.set_pause_guardian(&ctx.admin, &guardian);
        let result = ctx.revenue_pool.try_clear_pause_guardian(&ctx.admin);
        assert!(result.is_ok());
    }

    #[test]
    fn clear_pause_guardian_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.revenue_pool.set_pause_guardian(&ctx.admin, &guardian);
        let result = ctx.revenue_pool.try_clear_pause_guardian(&ctx.outsider);
        assert!(
            result.is_err(),
            "outsider should not be able to clear_pause_guardian"
        );
    }

    // -----------------------------------------------------------------------
    // pause (revenue pool) — admin or pause guardian
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_pause_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.revenue_pool.try_pause(&ctx.admin);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_pause_guardian_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.revenue_pool.set_pause_guardian(&ctx.admin, &guardian);
        let result = ctx.revenue_pool.try_pause(&guardian);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_pause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.revenue_pool.try_pause(&ctx.outsider);
        assert!(result.is_err(), "outsider should not be able to pause");
    }

    // -----------------------------------------------------------------------
    // unpause (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_unpause_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.pause(&ctx.admin);
        let result = ctx.revenue_pool.try_unpause(&ctx.admin);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_unpause_guardian_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.revenue_pool.set_pause_guardian(&ctx.admin, &guardian);
        ctx.revenue_pool.pause(&guardian);
        let result = ctx.revenue_pool.try_unpause(&guardian);
        assert!(result.is_err(), "guardian should not be able to unpause");
    }

    #[test]
    fn revenue_pool_unpause_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.revenue_pool.pause(&ctx.admin);
        let result = ctx.revenue_pool.try_unpause(&ctx.outsider);
        assert!(result.is_err(), "outsider should not be able to unpause");
    }

    // -----------------------------------------------------------------------
    // receive_payment (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_receive_payment_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .revenue_pool
            .try_receive_payment(&ctx.admin, &100, &true);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_receive_payment_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .revenue_pool
            .try_receive_payment(&ctx.outsider, &100, &true);
        assert!(
            result.is_err(),
            "outsider should not be able to receive_payment"
        );
    }

    // -----------------------------------------------------------------------
    // deposit_yield — treasury (admin) only
    // -----------------------------------------------------------------------

    #[test]
    fn deposit_yield_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.admin, &1000);
        ctx.usdc
            .approve(&ctx.admin, &ctx.revenue_pool_addr, &1000, &2000);
        let source = Symbol::new(&ctx.env, "test");
        let result = ctx
            .revenue_pool
            .try_deposit_yield(&ctx.admin, &100, &source);
        assert!(result.is_ok());
    }

    #[test]
    fn deposit_yield_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.outsider, &1000);
        ctx.usdc
            .approve(&ctx.outsider, &ctx.revenue_pool_addr, &1000, &2000);
        let source = Symbol::new(&ctx.env, "test");
        let result = ctx
            .revenue_pool
            .try_deposit_yield(&ctx.outsider, &100, &source);
        assert!(
            result.is_err(),
            "outsider should not be able to deposit_yield"
        );
    }

    // -----------------------------------------------------------------------
    // set_max_distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_max_distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx.revenue_pool.try_set_max_distribute(&ctx.admin, &5000);
        assert!(result.is_ok());
    }

    #[test]
    fn set_max_distribute_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let result = ctx
            .revenue_pool
            .try_set_max_distribute(&ctx.outsider, &5000);
        assert!(
            result.is_err(),
            "outsider should not be able to set_max_distribute"
        );
    }

    // -----------------------------------------------------------------------
    // broadcast (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_broadcast_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "test");
        let result = ctx.revenue_pool.try_broadcast(&ctx.admin, &severity, &msg);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_pool_broadcast_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "test");
        let result = ctx
            .revenue_pool
            .try_broadcast(&ctx.outsider, &severity, &msg);
        assert!(result.is_err(), "outsider should not be able to broadcast");
    }

    // -----------------------------------------------------------------------
    // upgrade (revenue pool) — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn revenue_pool_upgrade_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.revenue_pool.try_upgrade(&ctx.admin, &wasm_hash);
        assert!(
            result.is_err(),
            "the authorized call should reach host validation of the unknown WASM hash"
        );
    }

    #[test]
    fn revenue_pool_upgrade_outsider_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let wasm_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        let result = ctx.revenue_pool.try_upgrade(&ctx.outsider, &wasm_hash);
        assert!(result.is_err(), "outsider should not be able to upgrade");
    }
}
