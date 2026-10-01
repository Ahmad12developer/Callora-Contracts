//! Cursor-based batch developer withdrawals (#1135).
//!
//! `batch_withdraw_balance_cursor` used to validate lengths and return
//! `(0, true)` without moving funds, and reported mismatched lengths as the
//! unrelated `AmountNotPositive`. These tests pin the real behaviour.
//!
//! Run with `cargo test -p callora-settlement batch_withdraw`.

#![cfg(test)]

extern crate std;

use crate::{CalloraSettlement, CalloraSettlementClient, SettlementError, MAX_BATCH_SIZE};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Symbol, Vec};

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

    fn balance(&self, dev: &Address) -> i128 {
        self.client().get_developer_balance(dev, &self.usdc)
    }

    fn wallet(&self, who: &Address) -> i128 {
        token::Client::new(&self.env, &self.usdc).balance(who)
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

/// `n` developers credited `1_000 * (i + 1)`, and withdrawal amounts `100 * (i + 1)`.
fn funded(ctx: &Ctx, n: u32) -> (Vec<Address>, Vec<i128>) {
    let mut devs = Vec::new(&ctx.env);
    let mut amounts = Vec::new(&ctx.env);
    for i in 0..n {
        let dev = Address::generate(&ctx.env);
        ctx.client().receive_payment(
            &ctx.vault,
            &(1_000 * (i as i128 + 1)),
            &false,
            &Some(dev.clone()),
            &ctx.usdc,
            &(i + 1),
        );
        devs.push_back(dev);
        amounts.push_back(100 * (i as i128 + 1));
    }
    (devs, amounts)
}

// ── validation ────────────────────────────────────────────────────────────────

#[test]
fn batch_withdraw_mismatched_lengths_return_length_mismatch() {
    let ctx = setup();
    let (devs, mut amounts) = funded(&ctx, 2);
    amounts.push_back(1);
    assert_eq!(
        ctx.client()
            .try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &10),
        Err(Ok(SettlementError::LengthMismatch))
    );
    // Nothing moved.
    assert_eq!(ctx.balance(&devs.get(0).unwrap()), 1_000);
}

#[test]
fn batch_withdraw_rejects_empty_oversized_and_bad_cursor() {
    let ctx = setup();
    let client = ctx.client();
    let empty_d = Vec::<Address>::new(&ctx.env);
    let empty_a = Vec::<i128>::new(&ctx.env);
    assert_eq!(
        client.try_batch_withdraw_balance_cursor(&empty_d, &empty_a, &0, &1),
        Err(Ok(SettlementError::BatchEmpty))
    );

    let mut big_d = Vec::new(&ctx.env);
    let mut big_a = Vec::new(&ctx.env);
    for _ in 0..=MAX_BATCH_SIZE {
        big_d.push_back(Address::generate(&ctx.env));
        big_a.push_back(1_i128);
    }
    assert_eq!(
        client.try_batch_withdraw_balance_cursor(&big_d, &big_a, &0, &1),
        Err(Ok(SettlementError::BatchTooLarge))
    );

    let (devs, amounts) = funded(&ctx, 2);
    assert_eq!(
        client.try_batch_withdraw_balance_cursor(&devs, &amounts, &3, &1),
        Err(Ok(SettlementError::InvalidCursor)),
        "cursor past the end"
    );
    assert_eq!(
        client.try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &0),
        Err(Ok(SettlementError::InvalidCursor)),
        "zero limit would never make progress"
    );
    // cursor == len is a completed batch: a no-op that reports done.
    assert_eq!(
        client.batch_withdraw_balance_cursor(&devs, &amounts, &2, &5),
        (2, true)
    );
    assert_eq!(ctx.balance(&devs.get(0).unwrap()), 1_000);
}

// ── effects ───────────────────────────────────────────────────────────────────

#[test]
fn batch_withdraw_debits_each_developer_and_pays_them() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 3);
    let contract_before = ctx.wallet(&ctx.id);

    let res = ctx
        .client()
        .batch_withdraw_balance_cursor(&devs, &amounts, &0, &10);
    assert_eq!(res, (3, true));

    let mut total = 0;
    for i in 0..3 {
        let dev = devs.get(i).unwrap();
        let amount = amounts.get(i).unwrap();
        let credited = 1_000 * (i as i128 + 1);
        assert_eq!(
            ctx.balance(&dev),
            credited - amount,
            "developer {i} debited"
        );
        assert_eq!(ctx.wallet(&dev), amount, "developer {i} paid");
        total += amount;
    }
    assert_eq!(ctx.wallet(&ctx.id), contract_before - total);
}

#[test]
fn batch_withdraw_cursor_resumes_exactly_where_it_stopped() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 5);
    let client = ctx.client();

    let (c1, done1) = client.batch_withdraw_balance_cursor(&devs, &amounts, &0, &2);
    assert_eq!((c1, done1), (2, false));
    // Only the first window was processed.
    assert_eq!(ctx.wallet(&devs.get(0).unwrap()), 100);
    assert_eq!(ctx.wallet(&devs.get(1).unwrap()), 200);
    assert_eq!(ctx.wallet(&devs.get(2).unwrap()), 0);

    let (c2, done2) = client.batch_withdraw_balance_cursor(&devs, &amounts, &c1, &2);
    assert_eq!((c2, done2), (4, false));
    let (c3, done3) = client.batch_withdraw_balance_cursor(&devs, &amounts, &c2, &2);
    assert_eq!((c3, done3), (5, true));

    // Every developer was paid exactly once.
    for i in 0..5 {
        let dev = devs.get(i).unwrap();
        assert_eq!(ctx.wallet(&dev), amounts.get(i).unwrap());
        assert_eq!(
            ctx.balance(&dev),
            1_000 * (i as i128 + 1) - amounts.get(i).unwrap()
        );
    }

    // Resuming after completion is a no-op.
    assert_eq!(
        client.batch_withdraw_balance_cursor(&devs, &amounts, &c3, &2),
        (5, true)
    );
    assert_eq!(ctx.wallet(&devs.get(0).unwrap()), 100);
}

#[test]
fn batch_withdraw_limit_is_capped_at_max_batch_size() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 3);
    assert_eq!(
        ctx.client()
            .batch_withdraw_balance_cursor(&devs, &amounts, &0, &u32::MAX),
        (3, true)
    );
}

// ── authorization & atomicity ─────────────────────────────────────────────────

#[test]
fn batch_withdraw_requires_every_processed_developers_auth_only() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 3);

    ctx.client()
        .batch_withdraw_balance_cursor(&devs, &amounts, &0, &2);
    let signers: std::vec::Vec<Address> = ctx.env.auths().iter().map(|(a, _)| a.clone()).collect();
    assert!(signers.contains(&devs.get(0).unwrap()));
    assert!(signers.contains(&devs.get(1).unwrap()));
    assert!(
        !signers.contains(&devs.get(2).unwrap()),
        "developers outside the window are not asked to sign"
    );
}

#[test]
fn batch_withdraw_without_developer_signatures_moves_nothing() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 2);
    ctx.env.set_auths(&[]);
    assert!(ctx
        .client()
        .try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &2)
        .is_err());
    assert_eq!(ctx.balance(&devs.get(0).unwrap()), 1_000);
    assert_eq!(ctx.wallet(&devs.get(0).unwrap()), 0);
}

#[test]
fn batch_withdraw_item_failure_rolls_back_the_whole_window() {
    let ctx = setup();
    let (devs, mut amounts) = funded(&ctx, 2);
    // Second developer asks for more than their 2_000 balance.
    amounts.set(1, 5_000);
    assert_eq!(
        ctx.client()
            .try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &2),
        Err(Ok(SettlementError::InsufficientDeveloperBalance))
    );
    // The first item was rolled back with the rest of the invocation.
    assert_eq!(ctx.balance(&devs.get(0).unwrap()), 1_000);
    assert_eq!(ctx.wallet(&devs.get(0).unwrap()), 0);
}

#[test]
fn batch_withdraw_enforces_single_withdraw_rules() {
    let ctx = setup();
    let (devs, amounts) = funded(&ctx, 1);
    ctx.client().freeze_developer(
        &ctx.admin,
        &devs.get(0).unwrap(),
        &Symbol::new(&ctx.env, "review"),
    );
    assert_eq!(
        ctx.client()
            .try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &1),
        Err(Ok(SettlementError::DeveloperFrozen))
    );

    let ctx = setup();
    let (devs, mut amounts) = funded(&ctx, 1);
    amounts.set(0, 0);
    assert_eq!(
        ctx.client()
            .try_batch_withdraw_balance_cursor(&devs, &amounts, &0, &1),
        Err(Ok(SettlementError::AmountNotPositive))
    );
}
