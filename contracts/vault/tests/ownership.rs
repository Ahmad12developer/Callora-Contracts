//! Integration tests for Callora Vault ownership handover and event assertions.
//!
//! Covers GitHub Issue #1126 acceptance criteria:
//! 1. Nominate then accept: `transfer_ownership` followed by `accept_ownership`
//!    updates `get_owner` and clears pending owner.
//! 2. Accept without nomination: `accept_ownership` panics with
//!    `"no ownership transfer pending"`.
//! 3. Auth enforcement: `accept_ownership` fails if not authorized by the nominee.
//! 4. Event assertions: `ownership_nominated` and `ownership_accepted` events carry
//!    the canonical `"callora_v1"` version topic at index 1 and valid address payloads.

extern crate std;

use callora_vault::{CalloraVault, CalloraVaultClient};
use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Symbol, Val};

/// Helper to initialize the CalloraVault contract for testing.
fn setup(env: &Env) -> (Address, Address, CalloraVaultClient<'_>) {
    env.mock_all_auths_allowing_non_root_auth();
    let owner = Address::generate(env);
    let vault_address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_address);
    let usdc = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
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
    // Drain events emitted during init
    env.events().all();
    (owner, vault_address, client)
}

/// Helper to filter events emitted specifically by the vault contract.
fn vault_events(
    env: &Env,
    vault_addr: &Address,
) -> std::vec::Vec<(Address, soroban_sdk::Vec<Val>, Val)> {
    env.events()
        .all()
        .iter()
        .filter(|(addr, _, _)| addr == vault_addr)
        .collect()
}

/// Case 1: Nominate then accept.
///
/// `transfer_ownership(nominee)` followed by `accept_ownership()` updates
/// `get_owner()` to the nominee, and clears pending owner such that a second
/// `accept_ownership()` call fails deterministically.
#[test]
fn test_nominate_then_accept_transfers_ownership_and_clears_pending() {
    let env = Env::default();
    let (owner, _vault, client) = setup(&env);
    let nominee = Address::generate(&env);

    assert_eq!(client.get_owner(), owner);

    // 1. Owner nominates nominee
    client.transfer_ownership(&owner, &nominee);

    // Owner is still the original owner until accepted
    assert_eq!(client.get_owner(), owner);

    // 2. Nominee accepts ownership
    client.accept_ownership();

    // 3. get_owner returns nominee
    assert_eq!(client.get_owner(), nominee);

    // 4. Pending owner is cleared: second accept_ownership must fail
    let second_accept = client.try_accept_ownership();
    assert!(
        second_accept.is_err(),
        "Second accept_ownership must fail because pending owner is cleared"
    );
}

/// Case 2: Accept without nomination.
///
/// `accept_ownership()` called with no prior nomination must fail deterministically
/// with the exact panic message `"no ownership transfer pending"`.
#[test]
#[should_panic(expected = "no ownership transfer pending")]
fn test_accept_without_nomination_fails() {
    let env = Env::default();
    let (_owner, _vault, client) = setup(&env);

    client.accept_ownership();
}

/// Case 3: Auth enforcement.
///
/// `accept_ownership()` requires the pending nominee's authorization.
/// Invocations authorized by the wrong address or without nominee auth fail.
#[test]
fn test_accept_ownership_auth_enforcement() {
    let env = Env::default();
    let (owner, vault_address, client) = setup(&env);
    let nominee = Address::generate(&env);
    let intruder = Address::generate(&env);

    client.transfer_ownership(&owner, &nominee);

    // 1. Invoking accept_ownership with no authorization must fail
    env.set_auths(&[]);
    let unauthed_result = client.try_accept_ownership();
    assert!(
        unauthed_result.is_err(),
        "accept_ownership must fail without authorization"
    );

    // 2. Invoking accept_ownership with intruder authorization must fail
    env.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &vault_address,
            fn_name: "accept_ownership",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let intruder_result = client.try_accept_ownership();
    assert!(
        intruder_result.is_err(),
        "accept_ownership must reject auth from addresses other than the nominee"
    );

    // 3. Invoking accept_ownership with nominee authorization must succeed
    env.mock_auths(&[MockAuth {
        address: &nominee,
        invoke: &MockAuthInvoke {
            contract: &vault_address,
            fn_name: "accept_ownership",
            args: ().into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.accept_ownership();
    assert_eq!(client.get_owner(), nominee);
}

/// Case 4: Event assertions.
///
/// Both `ownership_nominated` and `ownership_accepted` events must carry the
/// canonical `"callora_v1"` version topic at index 1 and the correct address data
/// in each event payload.
#[test]
fn test_ownership_events_and_version_topic() {
    let env = Env::default();
    let (owner, vault_address, client) = setup(&env);
    let nominee = Address::generate(&env);

    // 1. Trigger ownership nomination and assert event
    client.transfer_ownership(&owner, &nominee);

    let events = vault_events(&env, &vault_address);
    assert_eq!(
        events.len(),
        1,
        "transfer_ownership must emit exactly one vault event"
    );

    let (contract_addr, topics, data) = events.last().unwrap();
    assert_eq!(contract_addr, &vault_address);
    assert_eq!(
        topics.len(),
        3,
        "ownership_nominated must emit 3 topics: (name, version, caller)"
    );

    let topic_name: Symbol = topics.get(0).unwrap().into_val(&env);
    let topic_version: Symbol = topics.get(1).unwrap().into_val(&env);
    let topic_caller: Address = topics.get(2).unwrap().into_val(&env);
    let payload_nominee: Address = data.into_val(&env);

    assert_eq!(
        topic_name,
        Symbol::new(&env, "ownership_nominated"),
        "Topic 0 must be 'ownership_nominated'"
    );
    assert_eq!(
        topic_version,
        Symbol::new(&env, "callora_v1"),
        "Topic 1 must be the 'callora_v1' version topic"
    );
    assert_eq!(
        topic_caller, owner,
        "Topic 2 must be the nominating owner address"
    );
    assert_eq!(
        payload_nominee, nominee,
        "Event data payload must be the nominee address"
    );

    // 2. Trigger ownership acceptance and assert event
    client.accept_ownership();

    let events_after = vault_events(&env, &vault_address);
    assert_eq!(
        events_after.len(),
        2,
        "accept_ownership must emit the second vault event"
    );

    let (contract_addr_acc, topics_acc, data_acc) = events_after.last().unwrap();
    assert_eq!(contract_addr_acc, &vault_address);
    assert_eq!(
        topics_acc.len(),
        3,
        "ownership_accepted must emit 3 topics: (name, version, new_owner)"
    );

    let acc_name: Symbol = topics_acc.get(0).unwrap().into_val(&env);
    let acc_version: Symbol = topics_acc.get(1).unwrap().into_val(&env);
    let acc_new_owner: Address = topics_acc.get(2).unwrap().into_val(&env);

    assert_eq!(
        acc_name,
        Symbol::new(&env, "ownership_accepted"),
        "Topic 0 must be 'ownership_accepted'"
    );
    assert_eq!(
        acc_version,
        Symbol::new(&env, "callora_v1"),
        "Topic 1 must be the 'callora_v1' version topic"
    );
    assert_eq!(
        acc_new_owner, nominee,
        "Topic 2 must be the new owner address"
    );
    // Data payload for ownership_accepted is unit ()
    let is_void = data_acc.is_void();
    assert!(is_void, "ownership_accepted data payload must be void ()");
}
