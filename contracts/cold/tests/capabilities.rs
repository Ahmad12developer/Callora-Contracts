//! Focused tests for the cold `capabilities()` view (#1117).

extern crate std;

use callora_cold::{CalloraCold, CalloraColdClient, ALL_CAPABILITIES};
use soroban_sdk::Env;

fn client(env: &Env) -> CalloraColdClient<'_> {
    let addr = env.register(CalloraCold, ());
    CalloraColdClient::new(env, &addr)
}

#[test]
fn capabilities_returns_empty_until_vault_supports_cold_storage() {
    let env = Env::default();
    assert_eq!(client(&env).capabilities(), 0);
}

#[test]
fn capabilities_equals_all_capabilities_constant() {
    let env = Env::default();
    assert_eq!(client(&env).capabilities(), ALL_CAPABILITIES);
}

#[test]
fn no_cold_capability_is_advertised() {
    let env = Env::default();
    let caps = client(&env).capabilities();
    assert_eq!(caps, ALL_CAPABILITIES);
    assert_eq!(caps, 0);
}

#[test]
fn capability_delta_has_no_false_positive_features() {
    let current = client(&Env::default()).capabilities();
    assert_eq!(current & !ALL_CAPABILITIES, 0);
}

#[test]
fn reserved_high_bits_remain_zero() {
    let env = Env::default();
    let caps = client(&env).capabilities();
    assert_eq!(caps >> 7, 0);
}

#[test]
fn capabilities_is_stable_across_calls() {
    let env = Env::default();
    let c = client(&env);
    assert_eq!(c.capabilities(), c.capabilities());
}
