//! GrantFox FWC26 — Per-Entrypoint Access-Control Matrix (Stellar Wave)
//!
//! Asserts every auth-guarded entrypoint across Vault, Settlement, and
//! Revenue Pool enforces `require_auth`:
//!
//! * **authorized** — `mock_all_auths()` active; expected role succeeds.
//! * **unauthorized** — `set_auths(&[])` strips all sigs; outsider / wrong
//!   role gets `Err`, proving `require_auth` fired before any state change.
//!
//! # Entrypoint coverage (tasks 4 & 5)
//!
//! ## Settlement (task 4)
//! `receive_payment`, `batch_receive_payment`,
//! `set_admin`, `accept_admin`, `cancel_admin_transfer`,
//! `propose_vault`, `accept_vault`,
//! `set_usdc_token`, `set_developer_claim_window`,
//! `clear_developer_claim_window`, `get_all_developer_balances`,
//! `force_credit_developer`, `set_daily_withdraw_cap`,
//! `set_developer_min_balance`, `withdraw_developer_balance`,
//! `propose_balance_migration`, `broadcast`, `upgrade`
//!
//! ## Revenue Pool (task 5)
//! `distribute`, `batch_distribute`,
//! `set_admin`, `accept_admin`, `claim_admin`, `cancel_admin_transfer`,
//! `set_pause_guardian`, `clear_pause_guardian`,
//! `pause` (admin path + guardian path), `unpause`,
//! `receive_payment`, `deposit_yield`, `set_max_distribute`,
//! `broadcast`, `upgrade`,
//! `propose_emergency_drain`, `cancel_emergency_drain`,
//! `execute_emergency_drain` (after 24 h timelock)

#![no_std]

extern crate std;

use soroban_sdk::token as soroban_token;
use soroban_sdk::{
    testutils::Address as _, testutils::Ledger as _, Address, BytesN, Env, Symbol, Vec,
};

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
    let sa = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = sa.address();
    (
        addr.clone(),
        soroban_token::Client::new(env, &addr),
        soroban_token::StellarAssetClient::new(env, &addr),
    )
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let addr = env.register(callora_vault::CalloraVault, ());
    (addr.clone(), CalloraVaultClient::new(env, &addr))
}

fn create_settlement(env: &Env) -> (Address, CalloraSettlementClient<'_>) {
    let addr = env.register(callora_settlement::CalloraSettlement, ());
    (addr.clone(), CalloraSettlementClient::new(env, &addr))
}

fn create_pool(env: &Env) -> (Address, RevenuePoolClient<'_>) {
    let addr = env.register(callora_revenue_pool::RevenuePool, ());
    (addr.clone(), RevenuePoolClient::new(env, &addr))
}

// ---------------------------------------------------------------------------
// Shared context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: &'a Env,
    vault_addr: Address,
    vault: CalloraVaultClient<'a>,
    settlement_addr: Address,
    settlement: CalloraSettlementClient<'a>,
    pool_addr: Address,
    pool: RevenuePoolClient<'a>,
    usdc_addr: Address,
    usdc: soroban_token::Client<'a>,
    usdc_admin: soroban_token::StellarAssetClient<'a>,
    owner: Address,
    admin: Address,
    authorized_caller: Address,
    developer: Address,
    pending_admin: Address,
    outsider: Address,
}

fn setup<'a>(env: &'a Env) -> Ctx<'a> {
    let owner = Address::generate(&env);
    let admin = Address::generate(&env);
    let authorized_caller = Address::generate(&env);
    let developer = Address::generate(&env);
    let pending_admin = Address::generate(&env);
    let outsider = Address::generate(&env);

    let (vault_addr, vault) = create_vault(&env);
    let (settlement_addr, settlement) = create_settlement(&env);
    let (pool_addr, pool) = create_pool(&env);
    let (usdc_addr, usdc, usdc_admin) = create_usdc(&env, &admin);

    usdc_admin.mint(&vault_addr, &1_000_000_000);
    usdc_admin.mint(&pool_addr, &1_000_000_000);
    usdc_admin.mint(&settlement_addr, &1_000_000_000);

    vault.init(
        &owner,
        &usdc_addr,
        &1_000_000,
        &authorized_caller,
        &1,
        &Some(pool_addr.clone()),
        &100_000_000,
        &settlement_addr,
    );

    settlement.init(&admin, &vault_addr);
    pool.init(&admin, &usdc_addr);

    Ctx {
        env,
        vault_addr,
        vault,
        settlement_addr,
        settlement,
        pool_addr,
        pool,
        usdc_addr,
        usdc,
        usdc_admin,
        owner,
        admin,
        authorized_caller,
        developer,
        pending_admin,
        outsider,
    }
}

// ===========================================================================
// VAULT — per-entrypoint access-control matrix
// ===========================================================================

mod vault {
    use super::*;

    // -----------------------------------------------------------------------
    // deposit — owner or allowed depositor
    // -----------------------------------------------------------------------

    #[test]
    fn deposit_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.owner, &500);
        ctx.usdc
            .approve(&ctx.owner, &ctx.vault_addr, &500, &999_999);
        assert!(ctx.vault.try_deposit(&ctx.owner, &100).is_ok());
    }

    #[test]
    fn deposit_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.outsider, &500);
        ctx.usdc
            .approve(&ctx.outsider, &ctx.vault_addr, &500, &999_999);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_deposit(&ctx.outsider, &100).is_err());
    }

    #[test]
    fn deposit_authorized_caller_rejected() {
        // authorized_caller has no deposit right even with valid auth
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.authorized_caller, &500);
        ctx.usdc
            .approve(&ctx.authorized_caller, &ctx.vault_addr, &500, &999_999);
        assert!(ctx.vault.try_deposit(&ctx.authorized_caller, &100).is_err());
    }

    // -----------------------------------------------------------------------
    // deduct — authorized_caller only
    // -----------------------------------------------------------------------

    #[test]
    fn deduct_authorized_caller_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_deduct(&ctx.authorized_caller, &100, &1)
            .is_ok());
    }

    #[test]
    fn deduct_owner_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_deduct(&ctx.owner, &100, &2).is_err());
    }

    #[test]
    fn deduct_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_deduct(&ctx.outsider, &100, &3).is_err());
    }

    // -----------------------------------------------------------------------
    // batch_deduct — authorized_caller only
    // -----------------------------------------------------------------------

    #[test]
    fn batch_deduct_authorized_caller_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(50i128, 10u64), (50i128, 11u64)]);
        assert!(ctx
            .vault
            .try_batch_deduct(&ctx.authorized_caller, &items)
            .is_ok());
    }

    #[test]
    fn batch_deduct_owner_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(50i128, 20u64)]);
        assert!(ctx.vault.try_batch_deduct(&ctx.owner, &items).is_err());
    }

    #[test]
    fn batch_deduct_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(50i128, 30u64)]);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_batch_deduct(&ctx.outsider, &items).is_err());
    }

    // -----------------------------------------------------------------------
    // set_authorized_caller — owner only (single-arg: caller sets itself)
    // -----------------------------------------------------------------------

    #[test]
    fn set_authorized_caller_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_set_authorized_caller(&ctx.owner).is_ok());
    }

    #[test]
    fn set_authorized_caller_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_set_authorized_caller(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // pause — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_pause(&ctx.owner).is_ok());
    }

    #[test]
    fn pause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_pause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // unpause — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn unpause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.vault.pause(&ctx.owner);
        assert!(ctx.vault.try_unpause(&ctx.owner).is_ok());
    }

    #[test]
    fn unpause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.vault.pause(&ctx.owner);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_unpause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // set_max_deduct — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_max_deduct_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_set_max_deduct(&ctx.owner, &200_000_000)
            .is_ok());
    }

    #[test]
    fn set_max_deduct_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_set_max_deduct(&ctx.outsider, &200_000_000)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_settlement — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_settlement_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_addr = Address::generate(&ctx.env);
        assert!(ctx.vault.try_set_settlement(&ctx.owner, &new_addr).is_ok());
    }

    #[test]
    fn set_settlement_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_addr = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_set_settlement(&ctx.outsider, &new_addr)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_reserve_cap — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn set_reserve_cap_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_set_reserve_cap(&ctx.owner, &ctx.usdc_addr, &5_000_000)
            .is_ok());
    }

    #[test]
    fn set_reserve_cap_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_set_reserve_cap(&ctx.outsider, &ctx.usdc_addr, &5_000_000)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // prune_processed_requests — owner only
    // -----------------------------------------------------------------------

    #[test]
    fn prune_processed_requests_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let ids: Vec<u64> = Vec::new(&ctx.env);
        assert!(ctx
            .vault
            .try_prune_processed_requests(&ctx.owner, &ids)
            .is_ok());
    }

    #[test]
    fn prune_processed_requests_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let ids: Vec<u64> = Vec::new(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_prune_processed_requests(&ctx.outsider, &ids)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_admin — admin (owner by default) only
    // -----------------------------------------------------------------------

    #[test]
    fn set_admin_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_set_admin(&ctx.owner, &ctx.pending_admin)
            .is_ok());
    }

    #[test]
    fn set_admin_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_set_admin(&ctx.outsider, &ctx.pending_admin)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // accept_admin — pending admin only (no explicit caller arg)
    // -----------------------------------------------------------------------

    #[test]
    fn accept_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.vault.set_admin(&ctx.owner, &ctx.pending_admin);
        // accept_admin requires auth from the pending admin; mock_all_auths covers it.
        assert!(ctx.vault.try_accept_admin().is_ok());
    }

    #[test]
    fn accept_admin_without_pending_admin_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        // No pending admin stored — must return an error regardless of auth.
        assert!(ctx.vault.try_accept_admin().is_err());
    }

    // -----------------------------------------------------------------------
    // set_timelock_window — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_timelock_window_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_set_timelock_window(&ctx.owner, &86_400)
            .is_ok());
    }

    #[test]
    fn set_timelock_window_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_set_timelock_window(&ctx.outsider, &86_400)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // propose_pause — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_propose_pause(&ctx.owner).is_ok());
    }

    #[test]
    fn propose_pause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_propose_pause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // cancel_pause — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_pause_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_cancel_pause(&ctx.owner).is_ok());
    }

    #[test]
    fn cancel_pause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_cancel_pause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // propose_upgrade — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_upgrade_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        assert!(ctx.vault.try_propose_upgrade(&ctx.owner, &hash).is_ok());
    }

    #[test]
    fn propose_upgrade_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_propose_upgrade(&ctx.outsider, &hash).is_err());
    }

    // -----------------------------------------------------------------------
    // cancel_upgrade — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_upgrade_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_cancel_upgrade(&ctx.owner).is_ok());
    }

    #[test]
    fn cancel_upgrade_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_cancel_upgrade(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // propose_sweep — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_sweep_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let recipient = Address::generate(&ctx.env);
        assert!(ctx
            .vault
            .try_propose_sweep(&ctx.owner, &recipient, &1000)
            .is_ok());
    }

    #[test]
    fn propose_sweep_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let recipient = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .vault
            .try_propose_sweep(&ctx.outsider, &recipient, &1000)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // cancel_sweep — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_sweep_owner_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.vault.try_cancel_sweep(&ctx.owner).is_ok());
    }

    #[test]
    fn cancel_sweep_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.vault.try_cancel_sweep(&ctx.outsider).is_err());
    }
}

// ===========================================================================
// SETTLEMENT — per-entrypoint access-control matrix
// ===========================================================================

mod settlement {
    use super::*;

    // -----------------------------------------------------------------------
    // receive_payment — vault or admin
    // -----------------------------------------------------------------------

    #[test]
    fn receive_payment_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_receive_payment(&ctx.admin, &100, &true, &None, &ctx.usdc_addr, &1,)
            .is_ok());
    }

    #[test]
    fn receive_payment_via_vault_deduct_succeeds() {
        // vault.deduct triggers on-ledger transfer; settlement.receive_payment
        // is indirectly authorized because the vault is the registered vault.
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .vault
            .try_deduct(&ctx.authorized_caller, &100, &42)
            .is_ok());
    }

    #[test]
    fn receive_payment_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_receive_payment(&ctx.outsider, &100, &true, &None, &ctx.usdc_addr, &2,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // batch_receive_payment — vault or admin
    // -----------------------------------------------------------------------

    #[test]
    fn batch_receive_payment_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 50i128)]);
        assert!(ctx
            .settlement
            .try_batch_receive_payment(&ctx.admin, &items, &ctx.usdc_addr, &1,)
            .is_ok());
    }

    #[test]
    fn batch_receive_payment_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let items = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 50i128)]);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_batch_receive_payment(&ctx.outsider, &items, &ctx.usdc_addr, &2,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_admin — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_admin_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_set_admin(&ctx.admin, &ctx.pending_admin)
            .is_ok());
    }

    #[test]
    fn set_admin_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_set_admin(&ctx.outsider, &ctx.pending_admin)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // accept_admin — pending admin only (no explicit caller arg)
    // -----------------------------------------------------------------------

    #[test]
    fn accept_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(ctx.settlement.try_accept_admin().is_ok());
        assert_eq!(ctx.settlement.get_admin(), ctx.pending_admin);
    }

    // -----------------------------------------------------------------------
    // cancel_admin_transfer — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_admin_transfer_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(ctx.settlement.try_cancel_admin_transfer(&ctx.admin).is_ok());
    }

    #[test]
    fn cancel_admin_transfer_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.settlement.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_cancel_admin_transfer(&ctx.outsider)
            .is_err());
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
        assert!(ctx
            .settlement
            .try_propose_vault(&ctx.admin, &new_vault)
            .is_ok());
    }

    #[test]
    fn propose_vault_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_propose_vault(&ctx.outsider, &new_vault)
            .is_err());
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
        assert!(ctx.settlement.try_accept_vault(&new_vault).is_ok());
    }

    #[test]
    fn accept_vault_admin_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.settlement.propose_vault(&ctx.admin, &new_vault);
        assert!(ctx.settlement.try_accept_vault(&ctx.admin).is_err());
    }

    #[test]
    fn accept_vault_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_vault = Address::generate(&ctx.env);
        ctx.settlement.propose_vault(&ctx.admin, &new_vault);
        ctx.env.set_auths(&[]);
        assert!(ctx.settlement.try_accept_vault(&ctx.outsider).is_err());
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
        assert!(ctx
            .settlement
            .try_set_usdc_token(&ctx.admin, &new_usdc)
            .is_ok());
    }

    #[test]
    fn set_usdc_token_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_usdc = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_set_usdc_token(&ctx.outsider, &new_usdc)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_developer_claim_window — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_developer_claim_window_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_set_developer_claim_window(&ctx.admin, &ctx.developer, &100, &200,)
            .is_ok());
    }

    #[test]
    fn set_developer_claim_window_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_set_developer_claim_window(&ctx.outsider, &ctx.developer, &100, &200,)
            .is_err());
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
        assert!(ctx
            .settlement
            .try_clear_developer_claim_window(&ctx.admin, &ctx.developer,)
            .is_ok());
    }

    #[test]
    fn clear_developer_claim_window_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_clear_developer_claim_window(&ctx.outsider, &ctx.developer,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // get_all_developer_balances — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn get_all_developer_balances_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_get_all_developer_balances(&ctx.admin, &ctx.usdc_addr,)
            .is_ok());
    }

    #[test]
    fn get_all_developer_balances_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_get_all_developer_balances(&ctx.outsider, &ctx.usdc_addr,)
            .is_err());
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
        assert!(ctx
            .settlement
            .try_force_credit_developer(&ctx.admin, &ctx.developer, &200, &ctx.usdc_addr, &reason,)
            .is_ok());
    }

    #[test]
    fn force_credit_developer_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "test");
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_force_credit_developer(
                &ctx.outsider,
                &ctx.developer,
                &200,
                &ctx.usdc_addr,
                &reason,
            )
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_daily_withdraw_cap — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_daily_withdraw_cap_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_set_daily_withdraw_cap(&ctx.admin, &ctx.developer, &10_000,)
            .is_ok());
    }

    #[test]
    fn set_daily_withdraw_cap_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_set_daily_withdraw_cap(&ctx.outsider, &ctx.developer, &10_000,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_developer_min_balance — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_developer_min_balance_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .settlement
            .try_set_developer_min_balance(&ctx.admin, &ctx.developer, &100,)
            .is_ok());
    }

    #[test]
    fn set_developer_min_balance_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_set_developer_min_balance(&ctx.outsider, &ctx.developer, &100,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // withdraw_developer_balance — developer (own balance) only
    // -----------------------------------------------------------------------

    #[test]
    fn withdraw_developer_balance_developer_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "pay");
        ctx.settlement.force_credit_developer(
            &ctx.admin,
            &ctx.developer,
            &500,
            &ctx.usdc_addr,
            &reason,
        );
        ctx.settlement.set_usdc_token(&ctx.admin, &ctx.usdc_addr);
        assert!(ctx
            .settlement
            .try_withdraw_developer_balance(&ctx.developer, &100, &None,)
            .is_ok());
    }

    #[test]
    fn withdraw_developer_balance_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let reason = Symbol::new(&ctx.env, "pay");
        ctx.settlement.force_credit_developer(
            &ctx.admin,
            &ctx.developer,
            &500,
            &ctx.usdc_addr,
            &reason,
        );
        ctx.settlement.set_usdc_token(&ctx.admin, &ctx.usdc_addr);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_withdraw_developer_balance(&ctx.outsider, &100, &None,)
            .is_err());
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
        assert!(ctx
            .settlement
            .try_propose_balance_migration(&ctx.admin, &ctx.developer, &new_dev,)
            .is_ok());
    }

    #[test]
    fn propose_balance_migration_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let new_dev = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_propose_balance_migration(&ctx.outsider, &ctx.developer, &new_dev,)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // broadcast — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn broadcast_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_settlement::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "hello");
        assert!(ctx
            .settlement
            .try_broadcast(&ctx.admin, &severity, &msg)
            .is_ok());
    }

    #[test]
    fn broadcast_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_settlement::Severity::Warn;
        let msg = soroban_sdk::String::from_str(&ctx.env, "bad");
        ctx.env.set_auths(&[]);
        assert!(ctx
            .settlement
            .try_broadcast(&ctx.outsider, &severity, &msg)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // upgrade — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn upgrade_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        assert!(ctx.settlement.try_upgrade(&ctx.admin, &hash).is_ok());
    }

    #[test]
    fn upgrade_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        ctx.env.set_auths(&[]);
        assert!(ctx.settlement.try_upgrade(&ctx.outsider, &hash).is_err());
    }
}

// ===========================================================================
// REVENUE POOL — per-entrypoint access-control matrix (task 5)
// ===========================================================================

mod revenue_pool {
    use super::*;

    // -----------------------------------------------------------------------
    // distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .pool
            .try_distribute(&ctx.admin, &ctx.developer, &100)
            .is_ok());
    }

    #[test]
    fn distribute_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_distribute(&ctx.outsider, &ctx.developer, &100)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // batch_distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn batch_distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        assert!(ctx.pool.try_batch_distribute(&ctx.admin, &payments).is_ok());
    }

    #[test]
    fn batch_distribute_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_batch_distribute(&ctx.outsider, &payments)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_admin — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_admin_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .pool
            .try_set_admin(&ctx.admin, &ctx.pending_admin)
            .is_ok());
    }

    #[test]
    fn set_admin_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_set_admin(&ctx.outsider, &ctx.pending_admin)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // accept_admin — pending admin only
    // -----------------------------------------------------------------------

    #[test]
    fn accept_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(ctx.pool.try_accept_admin(&ctx.pending_admin).is_ok());
        assert_eq!(ctx.pool.get_admin(), ctx.pending_admin);
    }

    #[test]
    fn accept_admin_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_accept_admin(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // claim_admin — pending admin only (alias)
    // -----------------------------------------------------------------------

    #[test]
    fn claim_admin_pending_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(ctx.pool.try_claim_admin(&ctx.pending_admin).is_ok());
        assert_eq!(ctx.pool.get_admin(), ctx.pending_admin);
    }

    #[test]
    fn claim_admin_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_claim_admin(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // cancel_admin_transfer — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_admin_transfer_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(ctx.pool.try_cancel_admin_transfer(&ctx.admin).is_ok());
    }

    #[test]
    fn cancel_admin_transfer_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_cancel_admin_transfer(&ctx.outsider).is_err());
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
        assert!(ctx
            .pool
            .try_set_pause_guardian(&ctx.admin, &guardian)
            .is_ok());
    }

    #[test]
    fn set_pause_guardian_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_set_pause_guardian(&ctx.outsider, &guardian)
            .is_err());
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
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        assert!(ctx.pool.try_clear_pause_guardian(&ctx.admin).is_ok());
    }

    #[test]
    fn clear_pause_guardian_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_clear_pause_guardian(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // pause — admin or pause guardian
    // -----------------------------------------------------------------------

    #[test]
    fn pause_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.pool.try_pause(&ctx.admin).is_ok());
    }

    #[test]
    fn pause_guardian_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        assert!(ctx.pool.try_pause(&guardian).is_ok());
    }

    #[test]
    fn pause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_pause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // unpause — admin only (guardian cannot unpause)
    // -----------------------------------------------------------------------

    #[test]
    fn unpause_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.pause(&ctx.admin);
        assert!(ctx.pool.try_unpause(&ctx.admin).is_ok());
    }

    #[test]
    fn unpause_guardian_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        ctx.pool.pause(&guardian);
        // Guardian must not be able to unpause — business-logic rejection.
        assert!(ctx.pool.try_unpause(&guardian).is_err());
    }

    #[test]
    fn unpause_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.pause(&ctx.admin);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_unpause(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // receive_payment — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn receive_payment_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx
            .pool
            .try_receive_payment(&ctx.admin, &100, &true)
            .is_ok());
    }

    #[test]
    fn receive_payment_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_receive_payment(&ctx.outsider, &100, &true)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // deposit_yield — admin (treasury) only
    // -----------------------------------------------------------------------

    #[test]
    fn deposit_yield_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.admin, &1000);
        ctx.usdc
            .approve(&ctx.admin, &ctx.pool_addr, &1000, &999_999);
        let source = Symbol::new(&ctx.env, "yield");
        assert!(ctx
            .pool
            .try_deposit_yield(&ctx.admin, &200, &source)
            .is_ok());
    }

    #[test]
    fn deposit_yield_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.outsider, &1000);
        ctx.usdc
            .approve(&ctx.outsider, &ctx.pool_addr, &1000, &999_999);
        let source = Symbol::new(&ctx.env, "yield");
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_deposit_yield(&ctx.outsider, &200, &source)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // set_max_distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn set_max_distribute_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(ctx.pool.try_set_max_distribute(&ctx.admin, &50_000).is_ok());
    }

    #[test]
    fn set_max_distribute_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_set_max_distribute(&ctx.outsider, &50_000)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // broadcast — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn broadcast_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "all good");
        assert!(ctx.pool.try_broadcast(&ctx.admin, &severity, &msg).is_ok());
    }

    #[test]
    fn broadcast_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Crit;
        let msg = soroban_sdk::String::from_str(&ctx.env, "bad");
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_broadcast(&ctx.outsider, &severity, &msg)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // upgrade — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn upgrade_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        assert!(ctx.pool.try_upgrade(&ctx.admin, &hash).is_ok());
    }

    #[test]
    fn upgrade_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_upgrade(&ctx.outsider, &hash).is_err());
    }

    // -----------------------------------------------------------------------
    // propose_emergency_drain — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn propose_emergency_drain_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        assert!(ctx
            .pool
            .try_propose_emergency_drain(&ctx.admin, &treasury, &500)
            .is_ok());
    }

    #[test]
    fn propose_emergency_drain_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(ctx
            .pool
            .try_propose_emergency_drain(&ctx.outsider, &treasury, &500)
            .is_err());
    }

    // -----------------------------------------------------------------------
    // cancel_emergency_drain — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_emergency_drain_admin_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        assert!(ctx.pool.try_cancel_emergency_drain(&ctx.admin).is_ok());
    }

    #[test]
    fn cancel_emergency_drain_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_cancel_emergency_drain(&ctx.outsider).is_err());
    }

    // -----------------------------------------------------------------------
    // execute_emergency_drain — admin only, after 24 h timelock
    // -----------------------------------------------------------------------

    #[test]
    fn execute_emergency_drain_admin_after_timelock_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        // Advance clock past the 24-hour emergency drain timelock.
        ctx.env.ledger().with_mut(|l| l.timestamp += 86_401);
        assert!(ctx.pool.try_execute_emergency_drain(&ctx.admin).is_ok());
    }

    #[test]
    fn execute_emergency_drain_outsider_rejected_without_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.ledger().with_mut(|l| l.timestamp += 86_401);
        ctx.env.set_auths(&[]);
        assert!(ctx.pool.try_execute_emergency_drain(&ctx.outsider).is_err());
    }
}

// ===========================================================================
// REVENUE POOL — Issue #730 focused auth audit (GrantFox FWC26 Stellar Wave)
// ===========================================================================
//
// Every state-changing entrypoint on `callora-revenue-pool` must call
// `require_auth` on the acting `Address` before touching storage or emitting
// events.  This sub-module is the canonical living record of that audit:
//
// | Entrypoint               | Auth on      | Verified by test(s) below          |
// |--------------------------|--------------|-------------------------------------|
// | `set_admin`              | `caller`     | audit_set_admin_*                   |
// | `accept_admin`           | `caller`     | audit_accept_admin_*                |
// | `claim_admin`            | `caller`     | audit_claim_admin_*                 |
// | `cancel_admin_transfer`  | `caller`     | audit_cancel_admin_transfer_*       |
// | `set_pause_guardian`     | `caller`     | audit_set_pause_guardian_*          |
// | `clear_pause_guardian`   | `caller`     | audit_clear_pause_guardian_*        |
// | `pause`                  | `caller`     | audit_pause_*                       |
// | `unpause`                | `caller`     | audit_unpause_*                     |
// | `receive_payment`        | `caller`     | audit_receive_payment_*             |
// | `deposit_yield`          | `treasury`   | audit_deposit_yield_*               |
// | `set_max_distribute`     | `caller`     | audit_set_max_distribute_*          |
// | `distribute`             | `caller`     | audit_distribute_*                  |
// | `batch_distribute`       | `caller`     | audit_batch_distribute_*            |
// | `upgrade`                | `caller`     | audit_upgrade_*                     |
// | `broadcast`              | `caller`     | audit_broadcast_*                   |
// | `propose_emergency_drain`| `caller`     | audit_propose_emergency_drain_*     |
// | `execute_emergency_drain`| `caller`     | audit_execute_emergency_drain_*     |
// | `cancel_emergency_drain` | `caller`     | audit_cancel_emergency_drain_*      |
//
// Each entrypoint gets three test variants:
//   _no_auth   — `set_auths(&[])` strips every sig; the call must be `Err`.
//   _wrong_role — correct auth present but caller is not the required role;
//                 the call must be `Err` (business-logic rejection).
//   _authorized — correct role + auth; the call must be `Ok`.
mod revenue_pool_audit {
    use super::*;

    // -----------------------------------------------------------------------
    // set_admin — admin only, two-step rotation
    // -----------------------------------------------------------------------

    #[test]
    fn audit_set_admin_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_set_admin(&ctx.admin, &ctx.pending_admin)
                .is_err(),
            "set_admin must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_set_admin_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        // outsider has auth but is not the current admin
        assert!(
            ctx.pool
                .try_set_admin(&ctx.outsider, &ctx.pending_admin)
                .is_err(),
            "set_admin must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_set_admin_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool
                .try_set_admin(&ctx.admin, &ctx.pending_admin)
                .is_ok(),
            "set_admin must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_pending_admin(), Some(ctx.pending_admin));
    }

    // -----------------------------------------------------------------------
    // accept_admin — pending admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_accept_admin_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_accept_admin(&ctx.pending_admin).is_err(),
            "accept_admin must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_accept_admin_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        // outsider has auth but is not the pending admin
        assert!(
            ctx.pool.try_accept_admin(&ctx.outsider).is_err(),
            "accept_admin must reject caller who is not the pending admin"
        );
    }

    #[test]
    fn audit_accept_admin_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(
            ctx.pool.try_accept_admin(&ctx.pending_admin).is_ok(),
            "accept_admin must succeed for pending admin with auth"
        );
        assert_eq!(ctx.pool.get_admin(), ctx.pending_admin);
    }

    // -----------------------------------------------------------------------
    // claim_admin — pending admin only (alias for accept_admin)
    // -----------------------------------------------------------------------

    #[test]
    fn audit_claim_admin_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_claim_admin(&ctx.pending_admin).is_err(),
            "claim_admin must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_claim_admin_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(
            ctx.pool.try_claim_admin(&ctx.outsider).is_err(),
            "claim_admin must reject caller who is not the pending admin"
        );
    }

    #[test]
    fn audit_claim_admin_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(
            ctx.pool.try_claim_admin(&ctx.pending_admin).is_ok(),
            "claim_admin must succeed for pending admin with auth"
        );
        assert_eq!(ctx.pool.get_admin(), ctx.pending_admin);
    }

    // -----------------------------------------------------------------------
    // cancel_admin_transfer — current admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_cancel_admin_transfer_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_cancel_admin_transfer(&ctx.admin).is_err(),
            "cancel_admin_transfer must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_cancel_admin_transfer_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(
            ctx.pool.try_cancel_admin_transfer(&ctx.outsider).is_err(),
            "cancel_admin_transfer must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_cancel_admin_transfer_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.set_admin(&ctx.admin, &ctx.pending_admin);
        assert!(
            ctx.pool.try_cancel_admin_transfer(&ctx.admin).is_ok(),
            "cancel_admin_transfer must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_pending_admin(), None);
    }

    // -----------------------------------------------------------------------
    // set_pause_guardian — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_set_pause_guardian_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_set_pause_guardian(&ctx.admin, &guardian)
                .is_err(),
            "set_pause_guardian must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_set_pause_guardian_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        assert!(
            ctx.pool
                .try_set_pause_guardian(&ctx.outsider, &guardian)
                .is_err(),
            "set_pause_guardian must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_set_pause_guardian_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        assert!(
            ctx.pool
                .try_set_pause_guardian(&ctx.admin, &guardian)
                .is_ok(),
            "set_pause_guardian must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_pause_guardian(), Some(guardian));
    }

    // -----------------------------------------------------------------------
    // clear_pause_guardian — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_clear_pause_guardian_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_clear_pause_guardian(&ctx.admin).is_err(),
            "clear_pause_guardian must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_clear_pause_guardian_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        assert!(
            ctx.pool.try_clear_pause_guardian(&ctx.outsider).is_err(),
            "clear_pause_guardian must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_clear_pause_guardian_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        assert!(
            ctx.pool.try_clear_pause_guardian(&ctx.admin).is_ok(),
            "clear_pause_guardian must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_pause_guardian(), None);
    }

    // -----------------------------------------------------------------------
    // pause — admin or pause guardian; outsider always rejected
    // -----------------------------------------------------------------------

    #[test]
    fn audit_pause_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_pause(&ctx.admin).is_err(),
            "pause must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_pause_wrong_role_outsider_with_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        // outsider has auth but is neither admin nor guardian
        assert!(
            ctx.pool.try_pause(&ctx.outsider).is_err(),
            "pause must reject caller who is neither admin nor guardian"
        );
    }

    #[test]
    fn audit_pause_admin_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool.try_pause(&ctx.admin).is_ok(),
            "pause must succeed for admin with auth"
        );
        assert!(ctx.pool.is_paused());
    }

    #[test]
    fn audit_pause_guardian_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        assert!(
            ctx.pool.try_pause(&guardian).is_ok(),
            "pause must succeed for pause guardian with auth"
        );
        assert!(ctx.pool.is_paused());
    }

    // -----------------------------------------------------------------------
    // unpause — admin only; guardian cannot unpause
    // -----------------------------------------------------------------------

    #[test]
    fn audit_unpause_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.pause(&ctx.admin);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_unpause(&ctx.admin).is_err(),
            "unpause must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_unpause_wrong_role_outsider() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.pause(&ctx.admin);
        assert!(
            ctx.pool.try_unpause(&ctx.outsider).is_err(),
            "unpause must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_unpause_guardian_cannot_unpause() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let guardian = Address::generate(&ctx.env);
        ctx.pool.set_pause_guardian(&ctx.admin, &guardian);
        ctx.pool.pause(&guardian);
        // The guardian holds auth but must be rejected by require_admin
        assert!(
            ctx.pool.try_unpause(&guardian).is_err(),
            "unpause must reject pause guardian — guardian cannot unpause"
        );
    }

    #[test]
    fn audit_unpause_admin_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.pool.pause(&ctx.admin);
        assert!(
            ctx.pool.try_unpause(&ctx.admin).is_ok(),
            "unpause must succeed for admin with auth"
        );
        assert!(!ctx.pool.is_paused());
    }

    // -----------------------------------------------------------------------
    // receive_payment — admin only (event-only logging helper)
    // -----------------------------------------------------------------------

    #[test]
    fn audit_receive_payment_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_receive_payment(&ctx.admin, &500, &true)
                .is_err(),
            "receive_payment must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_receive_payment_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool
                .try_receive_payment(&ctx.outsider, &500, &true)
                .is_err(),
            "receive_payment must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_receive_payment_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool
                .try_receive_payment(&ctx.admin, &500, &true)
                .is_ok(),
            "receive_payment must succeed for admin with auth"
        );
    }

    // -----------------------------------------------------------------------
    // deposit_yield — treasury (must equal admin) only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_deposit_yield_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.admin, &1_000);
        let source = Symbol::new(&ctx.env, "fees");
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_deposit_yield(&ctx.admin, &400, &source)
                .is_err(),
            "deposit_yield must require auth on treasury: no-auth call must fail"
        );
    }

    #[test]
    fn audit_deposit_yield_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.outsider, &1_000);
        let source = Symbol::new(&ctx.env, "fees");
        // outsider has auth but is not the current admin (treasury check)
        assert!(
            ctx.pool
                .try_deposit_yield(&ctx.outsider, &400, &source)
                .is_err(),
            "deposit_yield must reject non-admin treasury caller even with auth"
        );
    }

    #[test]
    fn audit_deposit_yield_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.usdc_admin.mint(&ctx.admin, &1_000);
        let source = Symbol::new(&ctx.env, "fees");
        assert!(
            ctx.pool
                .try_deposit_yield(&ctx.admin, &400, &source)
                .is_ok(),
            "deposit_yield must succeed for admin (treasury) with auth"
        );
        assert_eq!(ctx.pool.get_cumulative_yield_deposited(), 400);
    }

    // -----------------------------------------------------------------------
    // set_max_distribute — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_set_max_distribute_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_set_max_distribute(&ctx.admin, &5_000).is_err(),
            "set_max_distribute must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_set_max_distribute_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool
                .try_set_max_distribute(&ctx.outsider, &5_000)
                .is_err(),
            "set_max_distribute must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_set_max_distribute_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool.try_set_max_distribute(&ctx.admin, &5_000).is_ok(),
            "set_max_distribute must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_max_distribute(), 5_000);
    }

    // -----------------------------------------------------------------------
    // distribute — admin only, unpaused
    // -----------------------------------------------------------------------

    #[test]
    fn audit_distribute_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_distribute(&ctx.admin, &ctx.developer, &100)
                .is_err(),
            "distribute must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_distribute_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        assert!(
            ctx.pool
                .try_distribute(&ctx.outsider, &ctx.developer, &100)
                .is_err(),
            "distribute must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_distribute_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let balance_before = ctx.pool.balance();
        assert!(
            ctx.pool
                .try_distribute(&ctx.admin, &ctx.developer, &100)
                .is_ok(),
            "distribute must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.balance(), balance_before - 100);
    }

    // -----------------------------------------------------------------------
    // batch_distribute — admin only, unpaused
    // -----------------------------------------------------------------------

    #[test]
    fn audit_batch_distribute_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_batch_distribute(&ctx.admin, &payments)
                .is_err(),
            "batch_distribute must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_batch_distribute_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let payments = Vec::from_array(&ctx.env, [(ctx.developer.clone(), 100i128)]);
        assert!(
            ctx.pool
                .try_batch_distribute(&ctx.outsider, &payments)
                .is_err(),
            "batch_distribute must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_batch_distribute_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let dev2 = Address::generate(&ctx.env);
        let payments = Vec::from_array(
            &ctx.env,
            [(ctx.developer.clone(), 100i128), (dev2, 200i128)],
        );
        let balance_before = ctx.pool.balance();
        assert!(
            ctx.pool.try_batch_distribute(&ctx.admin, &payments).is_ok(),
            "batch_distribute must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.balance(), balance_before - 300);
    }

    // -----------------------------------------------------------------------
    // upgrade — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_upgrade_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_upgrade(&ctx.admin, &hash).is_err(),
            "upgrade must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_upgrade_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
        assert!(
            ctx.pool.try_upgrade(&ctx.outsider, &hash).is_err(),
            "upgrade must reject non-admin caller even with auth"
        );
    }

    // -----------------------------------------------------------------------
    // broadcast — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_broadcast_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "audit test");
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_broadcast(&ctx.admin, &severity, &msg).is_err(),
            "broadcast must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_broadcast_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Warn;
        let msg = soroban_sdk::String::from_str(&ctx.env, "audit test");
        assert!(
            ctx.pool
                .try_broadcast(&ctx.outsider, &severity, &msg)
                .is_err(),
            "broadcast must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_broadcast_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let severity = callora_revenue_pool::Severity::Info;
        let msg = soroban_sdk::String::from_str(&ctx.env, "audit test ok");
        assert!(
            ctx.pool.try_broadcast(&ctx.admin, &severity, &msg).is_ok(),
            "broadcast must succeed for admin with auth"
        );
    }

    // -----------------------------------------------------------------------
    // propose_emergency_drain — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_propose_emergency_drain_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool
                .try_propose_emergency_drain(&ctx.admin, &treasury, &500)
                .is_err(),
            "propose_emergency_drain must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_propose_emergency_drain_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        assert!(
            ctx.pool
                .try_propose_emergency_drain(&ctx.outsider, &treasury, &500)
                .is_err(),
            "propose_emergency_drain must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_propose_emergency_drain_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        assert!(
            ctx.pool
                .try_propose_emergency_drain(&ctx.admin, &treasury, &500)
                .is_ok(),
            "propose_emergency_drain must succeed for admin with auth"
        );
        let pending = ctx.pool.get_pending_emergency_drain();
        assert!(pending.is_some());
        assert_eq!(pending.unwrap().to, treasury);
    }

    // -----------------------------------------------------------------------
    // cancel_emergency_drain — admin only
    // -----------------------------------------------------------------------

    #[test]
    fn audit_cancel_emergency_drain_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_cancel_emergency_drain(&ctx.admin).is_err(),
            "cancel_emergency_drain must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_cancel_emergency_drain_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        assert!(
            ctx.pool.try_cancel_emergency_drain(&ctx.outsider).is_err(),
            "cancel_emergency_drain must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_cancel_emergency_drain_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        assert!(
            ctx.pool.try_cancel_emergency_drain(&ctx.admin).is_ok(),
            "cancel_emergency_drain must succeed for admin with auth"
        );
        assert_eq!(ctx.pool.get_pending_emergency_drain(), None);
    }

    // -----------------------------------------------------------------------
    // execute_emergency_drain — admin only, after 24 h timelock
    // -----------------------------------------------------------------------

    #[test]
    fn audit_execute_emergency_drain_no_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.ledger().with_mut(|l| l.timestamp += 86_401);
        ctx.env.set_auths(&[]);
        assert!(
            ctx.pool.try_execute_emergency_drain(&ctx.admin).is_err(),
            "execute_emergency_drain must require auth: no-auth call must fail"
        );
    }

    #[test]
    fn audit_execute_emergency_drain_wrong_role() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.ledger().with_mut(|l| l.timestamp += 86_401);
        assert!(
            ctx.pool.try_execute_emergency_drain(&ctx.outsider).is_err(),
            "execute_emergency_drain must reject non-admin caller even with auth"
        );
    }

    #[test]
    fn audit_execute_emergency_drain_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup(&env);
        let treasury = Address::generate(&ctx.env);
        ctx.pool
            .propose_emergency_drain(&ctx.admin, &treasury, &500);
        ctx.env.ledger().with_mut(|l| l.timestamp += 86_401);
        assert!(
            ctx.pool.try_execute_emergency_drain(&ctx.admin).is_ok(),
            "execute_emergency_drain must succeed for admin with auth after timelock"
        );
        assert_eq!(ctx.pool.get_pending_emergency_drain(), None);
    }
}
