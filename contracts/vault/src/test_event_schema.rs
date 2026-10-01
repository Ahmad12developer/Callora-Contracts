//! Holistic audit: every `env.events().publish(...)` call site in this crate
//! must (a) actually fire when its owning function is invoked through a real
//! contract client, and (b) match the shape documented in `EVENT_SCHEMA.md`.
//!
//! Issue #1118: six previously-unversioned events (withdraw, withdraw_to,
//! distribute, admin_rescue/rescue_funds, set_reserve_cap,
//! prune_processed_requests/request_id_pruned) now carry "callora_v1" at
//! topic[1]. Per-event shape tests plus a full lifecycle sweep are added.

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol};

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(env: &'a Env, admin: &Address) -> (Address, token::StellarAssetClient<'a>) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let address = contract_address.address();
    let admin_client = token::StellarAssetClient::new(env, &address);
    (address, admin_client)
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

/// Returns `(owner, vault_address, client, usdc_admin)`.
///
/// Uses `mock_all_auths_allowing_non_root_auth` — `admin_rescue` calls
/// `require_auth` twice on the same caller (once explicitly, once inside
/// `require_admin`), which trips `mock_all_auths`'s duplicate-frame guard
/// with `Auth(ExistingValue)`.
///
/// `initial_balance` is `Some(0i128)`, not `None`: passing `None` stores
/// `Void` which the deposit function reads back as `i128` and panics with
/// `UnexpectedType`.
fn setup_full(
    env: &Env,
) -> (
    Address,
    Address,
    CalloraVaultClient<'_>,
    token::StellarAssetClient<'_>,
) {
    env.mock_all_auths_allowing_non_root_auth();
    let owner = Address::generate(env);
    let (usdc, usdc_admin) = create_usdc(env, &owner);
    let (vault_address, client) = create_vault(env);
    let settlement = Address::generate(env);
    client.init(
        &owner,
        &usdc,
        &Some(0i128),
        &None,
        &Some(1i128),
        &None,
        &None,
        &Some(settlement),
    );
    env.events().all();
    (owner, vault_address, client, usdc_admin)
}

/// Return only events emitted by `vault_addr` as a plain `std::vec::Vec`.
/// Soroban captures sub-invocation events (token transfers etc.) under the
/// token contract's address; this isolates what the vault itself published.
fn vault_events(
    env: &Env,
    vault_addr: &Address,
) -> std::vec::Vec<(Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val)> {
    env.events()
        .all()
        .iter()
        .filter(|(addr, _, _)| addr == vault_addr)
        .collect()
}

// ---------------------------------------------------------------------------
// Pre-existing shape tests (regression guard, updated to new helper)
// ---------------------------------------------------------------------------

#[test]
fn set_timelock_window_emits_exactly_one_event_with_correct_topic() {
    let env = Env::default();
    let (owner, _vault, client, _) = setup_full(&env);

    client.set_timelock_window(&owner, &(2 * 24 * 60 * 60));

    let events = env.events().all();
    assert_eq!(events.len(), 1, "set_timelock_window must emit exactly one event");
    let (_, topics, _) = events.last().unwrap();
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(topics.len(), 3, "expected (name, version, caller) topics");
    assert_eq!(t0, Symbol::new(&env, "tl_window_changed"));
}

#[test]
fn propose_pause_emits_exactly_one_event() {
    let env = Env::default();
    let (owner, _vault, client, _) = setup_full(&env);

    client.propose_pause(&owner);

    let events = env.events().all();
    assert_eq!(events.len(), 1, "propose_pause must emit exactly one event");
    let (_, topics, _) = events.last().unwrap();
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "pause_proposed"));
}

#[test]
fn execute_pause_emits_pause_executed_then_vault_paused() {
    let env = Env::default();
    let (owner, _vault, client, _) = setup_full(&env);

    client.propose_pause(&owner);
    env.events().all();
    let window = client.get_timelock_window();
    env.ledger().with_mut(|l| l.timestamp += window + 1);

    client.execute_pause(&owner);

    let events = env.events().all();
    assert_eq!(events.len(), 2, "expected pause_executed + vault_paused");
    let (_, t0, _) = &events.get(0).unwrap();
    let (_, t1, _) = &events.get(1).unwrap();
    let n0: Symbol = t0.get(0).unwrap().into_val(&env);
    let n1: Symbol = t1.get(0).unwrap().into_val(&env);
    assert_eq!(n0, Symbol::new(&env, "pause_executed"));
    assert_eq!(n1, Symbol::new(&env, "vault_paused"));
    assert!(client.is_paused());
}

#[test]
fn deposit_emits_documented_shape() {
    let env = Env::default();
    let (owner, _vault, client, usdc_admin) = setup_full(&env);
    // deposit transfers caller->vault; mint USDC to the caller.
    usdc_admin.mint(&owner, &500);

    client.deposit(&owner, &500);

    let all = env.events().all();
    let (_, topics, _) = all.last().unwrap();
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    assert!(topics.len() >= 2, "deposit must emit >= 2 topics");
    assert_eq!(t0, Symbol::new(&env, "deposit"));
}

// ---------------------------------------------------------------------------
// Issue #1118 — per-event version-topic tests
// ---------------------------------------------------------------------------

/// Filter to vault events, assert every one has "callora_v1" at topic[1].
macro_rules! assert_vault_events_versioned {
    ($env:expr, $vault:expr, $label:expr) => {{
        let version = Symbol::new(&$env, "callora_v1");
        let evts: std::vec::Vec<_> = $env
            .events()
            .all()
            .iter()
            .filter(|(addr, _, _)| addr == &$vault)
            .collect();
        assert!(!evts.is_empty(), "{}: no vault events emitted", $label);
        for (idx, (_, topics, _)) in evts.iter().enumerate() {
            assert!(
                topics.len() >= 2,
                "{}: vault event[{}] has {} topic(s); need >= 2",
                $label, idx, topics.len()
            );
            let t1: Symbol = topics.get(1).unwrap().into_val(&$env);
            assert_eq!(
                t1, version,
                "{}: vault event[{}] topic[1] = {:?}, want {:?}",
                $label, idx, t1, version
            );
        }
    }};
}

#[test]
fn event_version_shape_withdraw() {
    let env = Env::default();
    let (owner, vault, client, usdc_admin) = setup_full(&env);
    usdc_admin.mint(&owner, &1_000);
    client.deposit(&owner, &1_000);
    env.events().all();

    client.withdraw(&1_000);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "withdraw"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

#[test]
fn event_version_shape_withdraw_to() {
    let env = Env::default();
    let (owner, vault, client, usdc_admin) = setup_full(&env);
    usdc_admin.mint(&owner, &1_000);
    client.deposit(&owner, &1_000);
    env.events().all();

    let recipient = Address::generate(&env);
    client.withdraw_to(&recipient, &1_000);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "withdraw_to"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

#[test]
fn event_version_shape_distribute() {
    let env = Env::default();
    let (owner, vault, client, usdc_admin) = setup_full(&env);
    usdc_admin.mint(&vault, &5_000);
    env.events().all();

    let recipient = Address::generate(&env);
    client.distribute(&owner, &recipient, &1_000);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "distribute"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

#[test]
fn event_version_shape_admin_rescue() {
    let env = Env::default();
    let (owner, vault, client, _) = setup_full(&env);
    let (rescue_token, rescue_mint) = create_usdc(&env, &owner);
    rescue_mint.mint(&vault, &3_000);
    env.events().all();

    let to = Address::generate(&env);
    client.admin_rescue(&owner, &rescue_token, &to, &3_000);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "rescue_funds"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

#[test]
fn event_version_shape_set_reserve_cap() {
    let env = Env::default();
    let (owner, vault, client, _) = setup_full(&env);
    let dummy_token = Address::generate(&env);

    client.set_reserve_cap(&owner, &dummy_token, &100_000);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "reserve_cap_set"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

#[test]
fn event_version_shape_request_id_pruned() {
    let env = Env::default();
    let (owner, vault, client, _) = setup_full(&env);
    let req_id: u64 = 1;
    let key = StorageKey::ProcessedRequest(req_id);
    env.as_contract(&vault, || {
        env.storage().persistent().set(&key, &true);
    });

    let ids = soroban_sdk::vec![&env, req_id];
    client.prune_processed_requests(&owner, &ids);

    let evts = vault_events(&env, &vault);
    assert_eq!(evts.len(), 1);
    let (_, topics, _) = &evts[0];
    let t0: Symbol = topics.get(0).unwrap().into_val(&env);
    let t1: Symbol = topics.get(1).unwrap().into_val(&env);
    assert_eq!(t0, Symbol::new(&env, "request_id_pruned"), "topic[0]");
    assert_eq!(t1, Symbol::new(&env, "callora_v1"), "topic[1] version");
}

// ---------------------------------------------------------------------------
// Full lifecycle sweep  (Issue #1118 acceptance criterion)
//
// Every vault function that emits an event is driven here. The macro asserts
// every vault-originated event carries "callora_v1" at topic[1]. A new emit
// site without the version topic will fail with an explicit slot diff.
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_all_events_carry_version_topic() {
    let env = Env::default();
    let (owner, vault, client, usdc_admin) = setup_full(&env);

    // deposit (moves caller->vault; mint to caller)
    usdc_admin.mint(&owner, &200_000);
    client.deposit(&owner, &100_000);
    assert_vault_events_versioned!(env, vault, "deposit");

    // withdraw
    client.withdraw(&10_000);
    assert_vault_events_versioned!(env, vault, "withdraw");

    // withdraw_to
    let recipient = Address::generate(&env);
    client.withdraw_to(&recipient, &10_000);
    assert_vault_events_versioned!(env, vault, "withdraw_to");

    // set_max_deduct
    client.set_max_deduct(&owner, &1_000_000);
    assert_vault_events_versioned!(env, vault, "set_max_deduct");

    // distribute (mint on-ledger surplus to vault first)
    usdc_admin.mint(&vault, &20_000);
    env.events().all();
    client.distribute(&owner, &Address::generate(&env), &1_000);
    assert_vault_events_versioned!(env, vault, "distribute");

    // set_reserve_cap
    client.set_reserve_cap(&owner, &Address::generate(&env), &999_999);
    assert_vault_events_versioned!(env, vault, "set_reserve_cap");

    // admin_rescue (use a second token to avoid the protected-balance guard)
    let (rescue_token, rescue_admin) = create_usdc(&env, &owner);
    rescue_admin.mint(&vault, &5_000);
    env.events().all();
    client.admin_rescue(&owner, &rescue_token, &Address::generate(&env), &5_000);
    assert_vault_events_versioned!(env, vault, "admin_rescue");

    // prune_processed_requests (seed a marker manually)
    let prune_id: u64 = 2;
    env.as_contract(&vault, || {
        env.storage()
            .persistent()
            .set(&StorageKey::ProcessedRequest(prune_id), &true);
    });
    client.prune_processed_requests(&owner, &soroban_sdk::vec![&env, prune_id]);
    assert_vault_events_versioned!(env, vault, "prune_processed_requests");

    // set_timelock_window
    client.set_timelock_window(&owner, &7_200);
    assert_vault_events_versioned!(env, vault, "set_timelock_window");

    // propose_pause / cancel_pause
    client.propose_pause(&owner);
    assert_vault_events_versioned!(env, vault, "propose_pause");
    client.cancel_pause(&owner);
    assert_vault_events_versioned!(env, vault, "cancel_pause");

    // propose_pause -> execute_pause (emits pause_executed + vault_paused)
    client.propose_pause(&owner);
    env.events().all();
    let window = client.get_timelock_window();
    env.ledger().with_mut(|l| l.timestamp += window + 1);
    client.execute_pause(&owner);
    assert_vault_events_versioned!(env, vault, "execute_pause+vault_paused");

    // unpause
    client.unpause(&owner);
    assert_vault_events_versioned!(env, vault, "vault_unpaused");

    // transfer_ownership / accept_ownership
    let new_owner = Address::generate(&env);
    client.transfer_ownership(&owner, &new_owner);
    assert_vault_events_versioned!(env, vault, "transfer_ownership");
    client.accept_ownership();
    assert_vault_events_versioned!(env, vault, "accept_ownership");
}
