#![cfg(test)]

use crate::{
    CalloraSettlement, CalloraSettlementClient, StorageKey, INSTANCE_BUMP_AMOUNT,
    INSTANCE_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol, Vec};

fn setup() -> (Env, Address, Address, CalloraSettlementClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(&env, &contract_id);
    client.init(&admin, &vault);
    (env, admin, vault, client)
}

#[test]
fn test_get_admin_bumps_instance_ttl() {
    let (env, admin, _vault, client) = setup();

    // Advance sequence number to reduce instance TTL below threshold.
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.storage().instance().get_ttl();
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    let res = client.get_admin();
    assert_eq!(res, admin);

    let ttl_after = env.storage().instance().get_ttl();
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_vault_bumps_instance_ttl() {
    let (env, _admin, vault, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let res = client.get_vault();
    assert_eq!(res, vault);

    let ttl_after = env.storage().instance().get_ttl();
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_global_pool_bumps_instance_ttl() {
    let (env, _admin, vault, client) = setup();

    // Credit into the pool so total_balance is non-zero.
    let token = Address::generate(&env);
    client.add_supported_token(&_admin, &token);
    client.receive_payment(&vault, &1000i128, &true, &None, &token, &1u32);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let pool = client.get_global_pool();
    assert_eq!(pool.total_balance, 1000i128);

    let ttl_after = env.storage().instance().get_ttl();
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_total_received_bumps_instance_ttl() {
    let (env, _admin, _vault, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let total = client.get_total_received();
    assert_eq!(total, 0);

    let ttl_after = env.storage().instance().get_ttl();
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_developer_balance_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev = Address::generate(&env);
    let token = Address::generate(&env);
    let reason = Symbol::new(&env, "test");

    client.force_credit_developer(&admin, &dev, &500i128, &token, &reason);

    let key = StorageKey::DeveloperBalance(dev.clone(), token.clone());

    // Advance sequence number to decrease persistent TTL
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let ttl_before = env.storage().persistent().get_ttl(&key);
    assert!(ttl_before < PERSISTENT_BUMP_THRESHOLD);

    let bal = client.get_developer_balance(&dev, &token);
    assert_eq!(bal, 500);

    let ttl_after = env.storage().persistent().get_ttl(&key);
    assert_eq!(ttl_after, PERSISTENT_BUMP_AMOUNT);

    let inst_ttl = env.storage().instance().get_ttl();
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_developer_min_balance_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev = Address::generate(&env);

    client.set_developer_min_balance(&admin, &dev, &100i128);

    let key = StorageKey::DeveloperMinBalance(dev.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let ttl_before = env.storage().persistent().get_ttl(&key);
    assert!(ttl_before < PERSISTENT_BUMP_THRESHOLD);

    let min_bal = client.get_developer_min_balance(&dev);
    assert_eq!(min_bal, 100);

    let ttl_after = env.storage().persistent().get_ttl(&key);
    assert_eq!(ttl_after, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_developer_claim_window_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev = Address::generate(&env);

    client.set_developer_claim_window(&admin, &dev, &1000u64, &2000u64);

    let key = StorageKey::DeveloperClaimWindow(dev.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let ttl_before = env.storage().persistent().get_ttl(&key);
    assert!(ttl_before < PERSISTENT_BUMP_THRESHOLD);

    let win = client.get_developer_claim_window(&dev).unwrap();
    assert_eq!(win.start_ts, 1000);
    assert_eq!(win.end_ts, 2000);

    let ttl_after = env.storage().persistent().get_ttl(&key);
    assert_eq!(ttl_after, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_daily_withdraw_cap_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev = Address::generate(&env);

    client.set_daily_withdraw_cap(&admin, &dev, &5000i128);

    let key = StorageKey::DailyWithdrawCap(dev.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let cap = client.get_daily_withdraw_cap(&dev);
    assert_eq!(cap, 5000);

    let ttl_after = env.storage().persistent().get_ttl(&key);
    assert_eq!(ttl_after, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_withdrawal_today_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev = Address::generate(&env);
    let usdc = Address::generate(&env);

    client.set_usdc_token(&admin, &usdc);
    client.force_credit_developer(&admin, &dev, &1000i128, &usdc, &Symbol::new(&env, "credit"));

    // We check get_withdrawal_today after setting up key
    let key = StorageKey::WithdrawalToday(dev.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let today_withdrawn = client.get_withdrawal_today(&dev);
    assert_eq!(today_withdrawn, 0);

    // If key existed, it would bump TTL; calling get_withdrawal_today on empty key doesn't crash
    let inst_ttl = env.storage().instance().get_ttl();
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_all_developer_balances_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);
    let token = Address::generate(&env);

    client.force_credit_developer(&admin, &dev1, &100i128, &token, &Symbol::new(&env, "c1"));
    client.force_credit_developer(&admin, &dev2, &200i128, &token, &Symbol::new(&env, "c2"));

    let key1 = StorageKey::DeveloperBalance(dev1.clone(), token.clone());
    let key2 = StorageKey::DeveloperBalance(dev2.clone(), token.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let balances = client.get_all_developer_balances(&admin, &token);
    assert_eq!(balances.len(), 2);

    let ttl1 = env.storage().persistent().get_ttl(&key1);
    let ttl2 = env.storage().persistent().get_ttl(&key2);
    assert_eq!(ttl1, PERSISTENT_BUMP_AMOUNT);
    assert_eq!(ttl2, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_developer_balances_page_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev1 = Address::generate(&env);
    let token = Address::generate(&env);

    client.force_credit_developer(&admin, &dev1, &100i128, &token, &Symbol::new(&env, "c1"));

    let key1 = StorageKey::DeveloperBalance(dev1.clone(), token.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let page = client.get_developer_balances_page(&admin, &0u32, &10u32, &token);
    assert_eq!(page.len(), 1);

    let ttl1 = env.storage().persistent().get_ttl(&key1);
    assert_eq!(ttl1, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_developer_balances_cursor_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev1 = Address::generate(&env);
    let token = Address::generate(&env);

    client.force_credit_developer(&admin, &dev1, &100i128, &token, &Symbol::new(&env, "c1"));

    let key1 = StorageKey::DeveloperBalance(dev1.clone(), token.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let (items, _) = client.get_developer_balances_cursor(&admin, &None, &10u32, &token);
    assert_eq!(items.len(), 1);

    let ttl1 = env.storage().persistent().get_ttl(&key1);
    assert_eq!(ttl1, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_get_pending_admin_bumps_instance_ttl() {
    let (env, admin, _vault, client) = setup();
    let new_admin = Address::generate(&env);

    client.set_admin(&admin, &new_admin);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let pending = client.get_pending_admin();
    assert_eq!(pending, Some(new_admin));

    let inst_ttl = env.storage().instance().get_ttl();
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_version_bumps_instance_ttl() {
    let (env, _admin, _vault, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ver = client.get_version();
    assert_eq!(ver, None);

    let inst_ttl = env.storage().instance().get_ttl();
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_balance_migration_bumps_ttl() {
    let (env, admin, _vault, client) = setup();
    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);

    client.propose_balance_migration(&admin, &dev1, &dev2);

    let key = StorageKey::PendingDeveloperMigration(dev1.clone());

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let mig = client.get_balance_migration(&dev1);
    assert!(mig.is_some());

    let ttl = env.storage().persistent().get_ttl(&key);
    assert_eq!(ttl, PERSISTENT_BUMP_AMOUNT);
}

#[test]
fn test_migration_storage_version_bumps_instance_ttl() {
    let (env, _admin, _vault, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ver = client.migration_storage_version();
    assert_eq!(ver, 1);

    let inst_ttl = env.storage().instance().get_ttl();
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

/// #1131 — every settlement **write** path extends its persistent entry to the
/// shared `PERSISTENT_BUMP_AMOUNT`, for every key family (balances, replay
/// high-water marks, withdrawal counters, caps, claim windows, minimum
/// balances, pending migrations, V1→V2 migration targets, price-registry
/// write ledgers).
///
/// TTLs are read inside `env.as_contract` (the SDK only exposes `get_ttl` from
/// a contract frame) and the ledger is never advanced past an entry's
/// lifetime, so reads never hit archived state.
mod write_paths {
    use crate::replay_guard::{HWM_LIVE, HWM_THRESHOLD};
    use crate::{
        timelock, CalloraSettlement, CalloraSettlementClient, StorageKey, LEDGERS_PER_DAY,
        PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD,
    };
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{token, Address, Env, IntoVal, String, Symbol, Val, Vec};

    struct Ctx {
        env: Env,
        id: Address,
        admin: Address,
        vault: Address,
        usdc: Address,
    }

    impl Ctx {
        fn client(&self) -> CalloraSettlementClient<'_> {
            CalloraSettlementClient::new(&self.env, &self.id)
        }
    }

    fn setup() -> Ctx {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let vault = Address::generate(&env);
        let id = env.register(CalloraSettlement, ());
        let client = CalloraSettlementClient::new(&env, &id);
        client.init(&admin, &vault);
        let usdc = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        client.set_usdc_token(&admin, &usdc);
        token::StellarAssetClient::new(&env, &usdc).mint(&id, &1_000_000);
        Ctx {
            env,
            id,
            admin,
            vault,
            usdc,
        }
    }

    fn ttl<K: IntoVal<Env, Val>>(ctx: &Ctx, key: &K) -> u32 {
        ctx.env
            .as_contract(&ctx.id, || ctx.env.storage().persistent().get_ttl(key))
    }

    /// Keep the contract instance alive while the ledger is advanced.
    fn keep_instance_alive(ctx: &Ctx) {
        ctx.env.as_contract(&ctx.id, || {
            ctx.env
                .storage()
                .instance()
                .extend_ttl(PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_AMOUNT);
        });
    }

    fn balance_key(ctx: &Ctx, dev: &Address) -> StorageKey {
        StorageKey::DeveloperBalance(dev.clone(), ctx.usdc.clone())
    }

    fn credit(ctx: &Ctx, dev: &Address, amount: i128, seq: u32) {
        ctx.client().receive_payment(
            &ctx.vault,
            &amount,
            &false,
            &Some(dev.clone()),
            &ctx.usdc,
            &seq,
        );
    }

    #[test]
    fn ttl_constants_are_named_and_sized_for_retention() {
        assert_ne!(PERSISTENT_BUMP_AMOUNT, 50_000, "old ~2.9-day literal");
        // Compile-time checks: a mis-sized constant fails the build.
        const { assert!(PERSISTENT_BUMP_THRESHOLD < PERSISTENT_BUMP_AMOUNT) };
        const { assert!(PERSISTENT_BUMP_AMOUNT >= LEDGERS_PER_DAY * 90) };
        // Stays under the network max entry TTL (~180 days on pubnet).
        const { assert!(PERSISTENT_BUMP_AMOUNT <= LEDGERS_PER_DAY * 180) };
        assert_eq!(HWM_LIVE, PERSISTENT_BUMP_AMOUNT);
        assert_eq!(HWM_THRESHOLD, PERSISTENT_BUMP_THRESHOLD);
    }

    #[test]
    fn ttl_receive_payment_extends_balance_and_hwm() {
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        credit(&ctx, &dev, 500, 10);
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &dev)), PERSISTENT_BUMP_AMOUNT);
        assert_eq!(
            ttl(&ctx, &StorageKey::HighWaterMark(dev)),
            PERSISTENT_BUMP_AMOUNT
        );
    }

    #[test]
    fn ttl_batch_receive_payment_extends_every_balance_and_hwm() {
        let ctx = setup();
        let a = Address::generate(&ctx.env);
        let b = Address::generate(&ctx.env);
        let mut items = Vec::new(&ctx.env);
        items.push_back((a.clone(), 100_i128));
        items.push_back((b.clone(), 200_i128));
        ctx.client()
            .batch_receive_payment(&ctx.vault, &items, &ctx.usdc, &20);
        for dev in [a, b] {
            assert_eq!(ttl(&ctx, &balance_key(&ctx, &dev)), PERSISTENT_BUMP_AMOUNT);
            assert_eq!(
                ttl(&ctx, &StorageKey::HighWaterMark(dev)),
                PERSISTENT_BUMP_AMOUNT
            );
        }
    }

    #[test]
    fn ttl_withdraw_extends_balance_and_daily_counter() {
        // Not aged: advancing ~90 days would also archive the USDC token
        // contract's own storage used by the transfer. Re-extension below the
        // threshold is covered by `ttl_writes_only_re_extend_below_the_threshold`.
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        credit(&ctx, &dev, 1_000, 30);

        ctx.client().withdraw_developer_balance(&dev, &100, &None);
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &dev)), PERSISTENT_BUMP_AMOUNT);
        assert_eq!(
            ttl(&ctx, &StorageKey::WithdrawalToday(dev)),
            PERSISTENT_BUMP_AMOUNT
        );
    }

    #[test]
    fn ttl_admin_config_writes_extend_cap_window_and_min_balance() {
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        let client = ctx.client();
        client.set_daily_withdraw_cap(&ctx.admin, &dev, &5_000);
        client.set_developer_claim_window(&ctx.admin, &dev, &0, &u64::MAX);
        client.set_developer_min_balance(&ctx.admin, &dev, &10);
        assert_eq!(
            ttl(&ctx, &StorageKey::DailyWithdrawCap(dev.clone())),
            PERSISTENT_BUMP_AMOUNT
        );
        assert_eq!(
            ttl(&ctx, &StorageKey::DeveloperClaimWindow(dev.clone())),
            PERSISTENT_BUMP_AMOUNT
        );
        assert_eq!(
            ttl(&ctx, &StorageKey::DeveloperMinBalance(dev)),
            PERSISTENT_BUMP_AMOUNT
        );
    }

    #[test]
    fn ttl_force_credit_extends_balance() {
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        ctx.client().force_credit_developer(
            &ctx.admin,
            &dev,
            &250,
            &ctx.usdc,
            &Symbol::new(&ctx.env, "refund"),
        );
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &dev)), PERSISTENT_BUMP_AMOUNT);
    }

    #[test]
    fn ttl_balance_migration_extends_pending_proposal_and_both_balances() {
        let ctx = setup();
        let from = Address::generate(&ctx.env);
        let to = Address::generate(&ctx.env);
        credit(&ctx, &from, 700, 40);

        ctx.client()
            .propose_balance_migration(&ctx.admin, &from, &to);
        assert_eq!(
            ttl(&ctx, &StorageKey::PendingDeveloperMigration(from.clone())),
            PERSISTENT_BUMP_AMOUNT
        );

        ctx.env.ledger().with_mut(|l| {
            l.timestamp += timelock::DEVELOPER_MIGRATION_TIMELOCK_SECONDS + 1;
        });
        ctx.client().execute_balance_migration(&ctx.admin, &from);
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &from)), PERSISTENT_BUMP_AMOUNT);
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &to)), PERSISTENT_BUMP_AMOUNT);
    }

    #[test]
    fn ttl_v1_to_v2_migration_extends_the_v2_balance() {
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        ctx.env.as_contract(&ctx.id, || {
            let v1 = StorageKey::DeveloperBalanceV1(dev.clone());
            ctx.env.storage().persistent().set(&v1, &321_i128);
        });
        ctx.client().migrate_single_dev_v2(&ctx.admin, &dev);
        assert_eq!(ttl(&ctx, &balance_key(&ctx, &dev)), PERSISTENT_BUMP_AMOUNT);
    }

    #[test]
    fn ttl_price_registry_write_extends_last_write_marker() {
        let ctx = setup();
        ctx.client().set_price(
            &ctx.admin,
            &String::from_str(&ctx.env, "api-basic"),
            &String::from_str(&ctx.env, "1.00"),
        );
        assert_eq!(
            ttl(&ctx, &StorageKey::PriceRegistryLastWrite(ctx.admin.clone())),
            PERSISTENT_BUMP_AMOUNT
        );
    }

    #[test]
    fn ttl_writes_only_re_extend_below_the_threshold() {
        // threshold < amount: a write while the TTL is still above the
        // threshold does not pay for another extension; once below, it does.
        let ctx = setup();
        let dev = Address::generate(&ctx.env);
        let key = StorageKey::DailyWithdrawCap(dev.clone());
        keep_instance_alive(&ctx);

        ctx.client().set_daily_withdraw_cap(&ctx.admin, &dev, &1);
        let step = LEDGERS_PER_DAY * 10;
        ctx.env
            .ledger()
            .set_sequence_number(ctx.env.ledger().sequence() + step);
        ctx.client().set_daily_withdraw_cap(&ctx.admin, &dev, &2);
        assert_eq!(
            ttl(&ctx, &key),
            PERSISTENT_BUMP_AMOUNT - step,
            "no extension while above the threshold"
        );

        let to_below = PERSISTENT_BUMP_AMOUNT - step - PERSISTENT_BUMP_THRESHOLD + 1;
        ctx.env
            .ledger()
            .set_sequence_number(ctx.env.ledger().sequence() + to_below);
        ctx.client().set_daily_withdraw_cap(&ctx.admin, &dev, &3);
        assert_eq!(
            ttl(&ctx, &key),
            PERSISTENT_BUMP_AMOUNT,
            "extended once below the threshold"
        );
    }
}
