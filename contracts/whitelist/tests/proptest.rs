//! Deterministic state-machine tests for the whitelist contract.
//!
//! The generated traces cover successful and rejected mutations, insertion
//! order, membership queries, and clearing. A fixed linear congruential
//! generator makes every failure reproducible from its seed and step number.

extern crate std;

use callora_whitelist::{CalloraWhitelist, CalloraWhitelistClient, StorageKey, WhitelistError};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Vec};

const ADDRESS_POOL_SIZE: usize = 8;
const BENCHMARK_LIST_SIZE: usize = 32;
const SEED_COUNT: u64 = 32;
const START_TIMESTAMP: u64 = 1_000_000;
const TRACE_LENGTH: u32 = 64;

struct Prng {
    state: u64,
}

impl Prng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next_u64() as usize) % len
    }
}

#[repr(u8)]
enum Operation {
    Add = 0,
    AddDuplicate = 1,
    Remove = 2,
    RemoveMissing = 3,
    Clear = 4,
    AssertMembership = 5,
    AssertList = 6,
}

const OPERATION_COUNT: u64 = 7;

fn deploy_with_id<'a>(env: &'a Env, admin: &Address) -> (Address, CalloraWhitelistClient<'a>) {
    let contract_id = env.register(CalloraWhitelist, ());
    let client = CalloraWhitelistClient::new(env, &contract_id);
    client.init(admin);
    (contract_id, client)
}

fn deploy<'a>(env: &'a Env, admin: &Address) -> CalloraWhitelistClient<'a> {
    deploy_with_id(env, admin).1
}

fn seed_member(env: &Env, contract_id: &Address, address: &Address) {
    env.as_contract(contract_id, || {
        let mut list = Vec::new(env);
        list.push_back(address.clone());
        env.storage()
            .instance()
            .set(&StorageKey::WhitelistList, &list);
    });
}

fn advance_timestamp(env: &Env) {
    env.ledger()
        .set_timestamp(env.ledger().timestamp().saturating_add(1));
}

fn to_std_vec(addresses: Vec<Address>) -> std::vec::Vec<Address> {
    addresses.iter().collect()
}

fn assert_state(
    client: &CalloraWhitelistClient<'_>,
    pool: &[Address],
    missing: &Address,
    expected: &[Address],
    seed: u64,
    step: u32,
) {
    assert_eq!(
        to_std_vec(client.get_whitelist()),
        expected,
        "whitelist mismatch at seed={seed}, step={step}"
    );

    for address in pool {
        assert_eq!(
            client.is_whitelisted(address),
            expected.contains(address),
            "membership mismatch at seed={seed}, step={step}, address={address:?}"
        );
    }

    assert!(
        !client.is_whitelisted(missing),
        "missing address became whitelisted at seed={seed}, step={step}"
    );
}

fn next_absent_address(pool: &[Address], expected: &[Address], start: usize) -> Option<Address> {
    for offset in 0..pool.len() {
        let index = (start + offset) % pool.len();
        if !expected.contains(&pool[index]) {
            return Some(pool[index].clone());
        }
    }
    None
}

fn run_trace(seed: u64) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TIMESTAMP);

    let admin = Address::generate(&env);
    let pool: std::vec::Vec<Address> = (0..ADDRESS_POOL_SIZE)
        .map(|_| Address::generate(&env))
        .collect();
    let missing = Address::generate(&env);
    let client = deploy(&env, &admin);
    client.set_admin_cooldown(&admin, &1);

    let mut expected = std::vec::Vec::new();
    let mut rng = Prng::new(seed);

    for step in 1..=TRACE_LENGTH {
        match (rng.next_u64() % OPERATION_COUNT) as u8 {
            value if value == Operation::Add as u8 => {
                let start = rng.index(pool.len());
                if let Some(address) = next_absent_address(&pool, &expected, start) {
                    advance_timestamp(&env);
                    client.add_address(&admin, &address);
                    expected.push(address);
                }
            }
            value if value == Operation::AddDuplicate as u8 => {
                if !expected.is_empty() {
                    let address = expected[rng.index(expected.len())].clone();
                    advance_timestamp(&env);
                    assert_eq!(
                        client.try_add_address(&admin, &address).unwrap_err(),
                        Ok(WhitelistError::AddressAlreadyInWhitelist),
                        "duplicate add returned the wrong error at seed={seed}, step={step}"
                    );
                }
            }
            value if value == Operation::Remove as u8 => {
                if !expected.is_empty() {
                    let index = rng.index(expected.len());
                    let address = expected[index].clone();
                    advance_timestamp(&env);
                    client.remove_address(&admin, &address);
                    expected.remove(index);
                }
            }
            value if value == Operation::RemoveMissing as u8 => {
                advance_timestamp(&env);
                assert_eq!(
                    client.try_remove_address(&admin, &missing).unwrap_err(),
                    Ok(WhitelistError::AddressNotInWhitelist),
                    "missing remove returned the wrong error at seed={seed}, step={step}"
                );
            }
            value if value == Operation::Clear as u8 => {
                advance_timestamp(&env);
                client.clear_all(&admin);
                expected.clear();
            }
            value if value == Operation::AssertMembership as u8 => {
                let address = &pool[rng.index(pool.len())];
                assert_eq!(
                    client.is_whitelisted(address),
                    expected.contains(address),
                    "membership mismatch at seed={seed}, step={step}"
                );
            }
            value if value == Operation::AssertList as u8 => {
                assert_eq!(
                    to_std_vec(client.get_whitelist()),
                    expected,
                    "list mismatch at seed={seed}, step={step}"
                );
            }
            _ => unreachable!("operation is reduced modulo OPERATION_COUNT"),
        }

        assert_state(&client, &pool, &missing, &expected, seed, step);
    }
}

#[test]
fn deterministic_membership_and_list_invariants() {
    for seed in 0..SEED_COUNT {
        run_trace(seed);
    }
}

#[test]
fn benchmark_scenarios_execute_successfully() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(START_TIMESTAMP);

    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    client.set_admin_cooldown(&admin, &1);

    let mut members = std::vec::Vec::with_capacity(BENCHMARK_LIST_SIZE);
    for _ in 0..BENCHMARK_LIST_SIZE {
        let address = Address::generate(&env);
        client.add_address(&admin, &address);
        members.push(address);
        advance_timestamp(&env);
    }

    let missing = Address::generate(&env);
    assert_eq!(client.get_whitelist().len(), BENCHMARK_LIST_SIZE as u32);
    assert!(client.is_whitelisted(&members[0]));
    assert!(client.is_whitelisted(&members[BENCHMARK_LIST_SIZE - 1]));
    assert!(!client.is_whitelisted(&missing));

    client.add_address(&admin, &missing);
    advance_timestamp(&env);
    client.remove_address(&admin, &missing);
    advance_timestamp(&env);
    client.clear_all(&admin);

    assert!(client.get_whitelist().is_empty());
}

#[test]
fn add_address_requires_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let address = Address::generate(&env);
    let client = deploy(&env, &admin);

    env.set_auths(&[]);
    assert!(client.try_add_address(&admin, &address).is_err());
    assert!(client.get_whitelist().is_empty());
}

#[test]
fn remove_address_requires_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let address = Address::generate(&env);
    let (contract_id, client) = deploy_with_id(&env, &admin);
    seed_member(&env, &contract_id, &address);

    env.set_auths(&[]);
    assert!(client.is_whitelisted(&address));
    assert!(client.try_remove_address(&admin, &address).is_err());
    assert!(client.is_whitelisted(&address));
}

#[test]
fn clear_all_requires_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let address = Address::generate(&env);
    let (contract_id, client) = deploy_with_id(&env, &admin);
    seed_member(&env, &contract_id, &address);

    env.set_auths(&[]);
    assert!(client.is_whitelisted(&address));
    assert!(client.try_clear_all(&admin).is_err());
    assert!(client.is_whitelisted(&address));
}
