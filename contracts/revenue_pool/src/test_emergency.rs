extern crate std;

use super::*;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token;
use soroban_sdk::{Address, Env, Symbol, TryFromVal};

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

fn init_pool<'a>(
    env: &'a Env,
    admin: &Address,
    usdc_address: &Address,
) -> (Address, RevenuePoolClient<'a>) {
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(env, &pool_addr);
    client.init(admin, usdc_address);
    (pool_addr, client)
}

fn fund_pool(usdc_admin_client: &token::StellarAssetClient, pool_address: &Address, amount: i128) {
    usdc_admin_client.mint(pool_address, &amount);
}

// ---------------------------------------------------------------------------
// propose_emergency_drain
// ---------------------------------------------------------------------------

#[test]
fn propose_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let pending = client.get_pending_emergency_drain().unwrap();
    assert_eq!(pending.to, treasury);
    assert_eq!(pending.amount, 5_000);
    assert_eq!(pending.proposed_at, 1_700_000_000);
    assert_eq!(
        pending.execute_after,
        1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS
    );
}

#[test]
fn propose_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topic: Symbol = Symbol::try_from_val(&env, &event.1.get(0).unwrap()).unwrap();
    assert_eq!(topic, Symbol::new(&env, "emergency_drain_proposed"));
    let caller: Address = Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap();
    assert_eq!(caller, admin);
    let data: emergency::PendingEmergencyDrain =
        emergency::PendingEmergencyDrain::try_from_val(&env, &event.2).unwrap();
    assert_eq!(data.to, treasury);
    assert_eq!(data.amount, 5_000);
}

#[test]
#[should_panic]
fn propose_non_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    let outsider = Address::generate(&env);
    client.propose_emergency_drain(&outsider, &treasury, &5_000);
}

#[test]
#[should_panic]
fn propose_zero_amount_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    client.propose_emergency_drain(&admin, &treasury, &0);
}

#[test]
#[should_panic]
fn propose_self_drain_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);
    client.propose_emergency_drain(&admin, &pool, &5_000);
}

#[test]
#[should_panic]
fn propose_not_initialized_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let pool_addr = env.register(RevenuePool, ());
    let client = RevenuePoolClient::new(&env, &pool_addr);
    client.propose_emergency_drain(&admin, &treasury, &1_000);
}

// ---------------------------------------------------------------------------
// execute_emergency_drain
// ---------------------------------------------------------------------------

#[test]
fn execute_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&admin);
    assert!(client.get_pending_emergency_drain().is_none());
}

#[test]
fn execute_transfers_usdc() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&admin);
    assert_eq!(usdc_client.balance(&pool), 5_000);
    assert_eq!(usdc_client.balance(&treasury), 5_000);
}

#[test]
fn execute_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let executed_at = 1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS;
    env.ledger().set_timestamp(executed_at);
    client.execute_emergency_drain(&admin);
    let events = env.events().all();
    let event = events
        .iter()
        .find(|ev| {
            let topic: Symbol = Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap();
            topic == Symbol::new(&env, "emergency_drain_executed")
        })
        .expect("emergency_drain_executed event not emitted");
    let caller: Address = Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap();
    assert_eq!(caller, admin);
    let data: (Address, i128, u64, u64) =
        <(Address, i128, u64, u64)>::try_from_val(&env, &event.2).unwrap();
    assert_eq!(data.0, treasury);
    assert_eq!(data.1, 5_000);
    assert_eq!(data.2, 1_700_000_000);
    assert_eq!(data.3, executed_at);
}

#[test]
#[should_panic]
fn execute_before_timelock_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    client.execute_emergency_drain(&admin);
}

#[test]
#[should_panic]
fn execute_no_proposal_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&admin);
}

#[test]
#[should_panic]
fn execute_non_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    let outsider = Address::generate(&env);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&outsider);
}

#[test]
#[should_panic]
fn execute_insufficient_balance_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &10_001);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&admin);
}

#[test]
fn execute_replay_protected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.execute_emergency_drain(&admin);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.execute_emergency_drain(&admin);
    }));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// cancel_emergency_drain
// ---------------------------------------------------------------------------

#[test]
fn cancel_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    assert!(client.get_pending_emergency_drain().is_some());
    client.cancel_emergency_drain(&admin);
    assert!(client.get_pending_emergency_drain().is_none());
}

#[test]
fn cancel_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    client.cancel_emergency_drain(&admin);
    let events = env.events().all();
    let event = events
        .iter()
        .find(|ev| {
            let topic: Symbol = Symbol::try_from_val(&env, &ev.1.get(0).unwrap()).unwrap();
            topic == Symbol::new(&env, "emergency_drain_cancelled")
        })
        .expect("emergency_drain_cancelled event not emitted");
    let caller: Address = Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap();
    assert_eq!(caller, admin);
    let data: emergency::PendingEmergencyDrain =
        emergency::PendingEmergencyDrain::try_from_val(&env, &event.2).unwrap();
    assert_eq!(data.to, treasury);
    assert_eq!(data.amount, 5_000);
}

#[test]
#[should_panic]
fn cancel_non_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    let outsider = Address::generate(&env);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    client.cancel_emergency_drain(&outsider);
}

#[test]
#[should_panic]
fn cancel_no_proposal_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.cancel_emergency_drain(&admin);
}

// ---------------------------------------------------------------------------
// reproposal
// ---------------------------------------------------------------------------

#[test]
fn reproposal_replaces_existing() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let other = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger().set_timestamp(1_700_000_100);
    client.propose_emergency_drain(&admin, &other, &3_000);
    let pending = client.get_pending_emergency_drain().unwrap();
    assert_eq!(pending.to, other);
    assert_eq!(pending.amount, 3_000);
    assert_eq!(pending.proposed_at, 1_700_000_100);
    assert_eq!(
        pending.execute_after,
        1_700_000_100 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS
    );
}

// ---------------------------------------------------------------------------
// get_pending_emergency_drain
// ---------------------------------------------------------------------------

#[test]
fn get_pending_returns_none_when_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);
    assert!(client.get_pending_emergency_drain().is_none());
}

#[test]
fn get_pending_returns_proposal_after_propose() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &_pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &3_000);
    let pending = client.get_pending_emergency_drain().unwrap();
    assert_eq!(pending.to, treasury);
    assert_eq!(pending.amount, 3_000);
}

// ---------------------------------------------------------------------------
// edge cases
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn timelock_overflow_at_max_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(u64::MAX);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, _) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);
    client.propose_emergency_drain(&admin, &treasury, &1_000);
}

// ---------------------------------------------------------------------------
// emergency pause recovery
// ---------------------------------------------------------------------------

#[test]
fn guardian_can_enter_emergency_pause_but_only_admin_can_recover() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let (usdc_address, _, _) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);

    client.set_pause_guardian(&admin, &guardian);
    client.emergency_pause(&guardian);

    assert!(client.is_emergency_paused());
    assert!(client.is_paused());
    assert_eq!(
        client.try_recover_from_emergency(&guardian),
        Err(Ok(RevenuePoolError::Unauthorized.into()))
    );

    client.recover_from_emergency(&admin);
    assert!(!client.is_emergency_paused());
    assert!(!client.is_paused());
}

#[test]
fn emergency_pause_blocks_drain_execution_but_allows_cancellation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, _, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    env.ledger()
        .set_timestamp(1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS);
    client.emergency_pause(&admin);

    assert_eq!(
        client.try_execute_emergency_drain(&admin),
        Err(Ok(RevenuePoolError::EmergencyPaused.into()))
    );
    assert!(client.get_pending_emergency_drain().is_some());

    client.cancel_emergency_drain(&admin);
    assert!(client.get_pending_emergency_drain().is_none());
}

#[test]
fn emergency_pause_event_excludes_operational_details() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let (usdc_address, _, _) = create_usdc(&env, &admin);
    let (_pool, client) = init_pool(&env, &admin, &usdc_address);

    client.emergency_pause(&admin);
    let events = env.events().all();
    let event = events.last().unwrap();
    let topic: Symbol = Symbol::try_from_val(&env, &event.1.get(0).unwrap()).unwrap();
    let caller: Address = Address::try_from_val(&env, &event.1.get(1).unwrap()).unwrap();
    let state: bool = bool::try_from_val(&env, &event.2).unwrap();

    assert_eq!(topic, Symbol::new(&env, "emergency_pause_set"));
    assert_eq!(caller, admin);
    assert!(state);
    assert_eq!(event.1.len(), 2);
}

#[test]
fn execute_fails_one_second_before_timelock() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    let treasury_balance_before = usdc_client.balance(&treasury);
    let pool_balance_before = usdc_client.balance(&pool);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let execute_after = 1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS;
    env.ledger().set_timestamp(execute_after - 1);

    assert_eq!(
        client.try_execute_emergency_drain(&admin),
        Err(Ok(RevenuePoolError::TimelockNotExpired.into()))
    );

    let pending = client.get_pending_emergency_drain().unwrap();
    assert_eq!(pending.to, treasury);
    assert_eq!(pending.amount, 5_000);
    assert_eq!(pending.proposed_at, 1_700_000_000);
    assert_eq!(pending.execute_after, execute_after);
    assert_eq!(usdc_client.balance(&treasury), treasury_balance_before);
    assert_eq!(usdc_client.balance(&pool), pool_balance_before);
}

#[test]
fn execute_succeeds_exactly_at_timelock() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    let treasury_balance_before = usdc_client.balance(&treasury);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let execute_after = 1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS;
    env.ledger().set_timestamp(execute_after);

    assert_eq!(client.try_execute_emergency_drain(&admin), Ok(Ok(())));
    assert!(client.get_pending_emergency_drain().is_none());
    assert_eq!(
        usdc_client.balance(&treasury),
        treasury_balance_before + 5_000
    );
}

#[test]
fn pending_drain_removed_after_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let execute_after = 1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS;
    env.ledger().set_timestamp(execute_after);

    client.execute_emergency_drain(&admin);
    assert!(client.get_pending_emergency_drain().is_none());
    assert_eq!(
        client.try_execute_emergency_drain(&admin),
        Err(Ok(RevenuePoolError::NoPendingEmergencyDrain.into()))
    );
    assert_eq!(usdc_client.balance(&pool), 5_000);
    assert_eq!(usdc_client.balance(&treasury), 5_000);
}

#[test]
fn cancel_succeeds_after_timelock_before_execution() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (usdc_address, usdc_client, usdc_admin) = create_usdc(&env, &admin);
    let (pool, client) = init_pool(&env, &admin, &usdc_address);
    fund_pool(&usdc_admin, &pool, 10_000);

    let treasury_balance_before = usdc_client.balance(&treasury);
    let pool_balance_before = usdc_client.balance(&pool);

    client.propose_emergency_drain(&admin, &treasury, &5_000);
    let execute_after = 1_700_000_000 + emergency::EMERGENCY_DRAIN_TIMELOCK_SECONDS;
    env.ledger().set_timestamp(execute_after + 1);

    assert_eq!(client.try_cancel_emergency_drain(&admin), Ok(Ok(())));
    assert!(client.get_pending_emergency_drain().is_none());
    assert_eq!(usdc_client.balance(&treasury), treasury_balance_before);
    assert_eq!(usdc_client.balance(&pool), pool_balance_before);
}
