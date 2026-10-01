extern crate std;

use crate::{RevenuePool, RevenuePoolClient};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal, Symbol, Vec};

// ---------------------------------------------------------------------------
// Malicious USDC Mock
// ---------------------------------------------------------------------------

/// A mock USDC token that re-enters the revenue pool during `transfer`.
///
/// Attack kind is configured via `set_attack_config`:
/// - `"distribute"` — calls `pool.distribute(attacker, new_dev, 50)`
/// - `"set_admin"`  — calls `pool.set_admin(attacker, new_admin)`
/// - `"pause"`      — calls `pool.pause(attacker)`
/// - `"batch_distribute"` — calls `pool.batch_distribute(attacker, ...)`
#[contract]
pub struct MaliciousToken;

#[contractimpl]
impl MaliciousToken {
    /// Intercept a USDC transfer and optionally re-enter the revenue pool.
    ///
    /// When `attack_active` is `true`, disables recursion and calls the pool
    /// with the configured `attack_kind` via the `attacker` address.
    pub fn transfer(env: Env, from: Address, _to: Address, _amount: i128) {
        from.require_auth();

        let pool_addr: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "pool_addr"));
        let attack_active: bool = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "attack_active"))
            .unwrap_or(false);

        if attack_active {
            if let Some(pool) = pool_addr {
                // Prevent infinite recursion
                env.storage()
                    .instance()
                    .set(&Symbol::new(&env, "attack_active"), &false);

                let attacker: Address = env
                    .storage()
                    .instance()
                    .get(&Symbol::new(&env, "attack_caller"))
                    .unwrap();
                let attack_kind: Symbol = env
                    .storage()
                    .instance()
                    .get(&Symbol::new(&env, "attack_kind"))
                    .unwrap_or(Symbol::new(&env, "none"));

                let pool_client = RevenuePoolClient::new(&env, &pool);

                // Re-enter the revenue pool with the configured attack
                let kind_distribute = Symbol::new(&env, "distribute");
                let kind_set_admin = Symbol::new(&env, "set_admin");
                let kind_pause = Symbol::new(&env, "pause");
                let kind_batch = Symbol::new(&env, "batch_distribute");

                if attack_kind == kind_distribute {
                    let recipient = Address::generate(&env);
                    let _ = pool_client.try_distribute(&attacker, &recipient, &50);
                } else if attack_kind == kind_set_admin {
                    let new_admin = Address::generate(&env);
                    let _ = pool_client.try_set_admin(&attacker, &new_admin);
                } else if attack_kind == kind_pause {
                    let _ = pool_client.try_pause(&attacker);
                } else if attack_kind == kind_batch {
                    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
                    payments.push_back((Address::generate(&env), 30));
                    payments.push_back((Address::generate(&env), 20));
                    let _ = pool_client.try_batch_distribute(&attacker, &payments);
                }
            }
        }
    }

    /// Return a fixed large balance so the pool never hits InsufficientBalance.
    pub fn balance(_env: Env, _id: Address) -> i128 {
        1_000_000_000
    }

    /// Configure the reentrancy attack parameters.
    ///
    /// # Arguments
    /// * `pool`   — the revenue pool contract address to re-enter
    /// * `caller` — the address to pass as `caller` when re-entering
    /// * `active` — whether the next `transfer` should trigger reentrancy
    /// * `kind`   — one of `"distribute"`, `"set_admin"`, `"pause"`, `"batch_distribute"`
    pub fn set_attack_config(env: Env, pool: Address, caller: Address, active: bool, kind: Symbol) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "pool_addr"), &pool);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_caller"), &caller);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_active"), &active);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_kind"), &kind);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register a real Stellar asset contract for USDC so we can mint funds.
fn create_real_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (
    Address,
    soroban_sdk::token::Client<'a>,
    soroban_sdk::token::StellarAssetClient<'a>,
) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = contract_address.address();
    (
        addr.clone(),
        soroban_sdk::token::Client::new(env, &addr),
        soroban_sdk::token::StellarAssetClient::new(env, &addr),
    )
}

/// Deploy the revenue pool with the malicious token as its USDC.
///
/// Returns `(pool_addr, pool_client, token_addr, real_usdc_addr, admin)`.
fn setup_reentrancy_test(env: &Env) -> (Address, RevenuePoolClient<'_>, Address, Address, Address) {
    let admin = Address::generate(env);
    let pool_addr = env.register(RevenuePool, ());
    let pool_client = RevenuePoolClient::new(env, &pool_addr);
    let token_addr = env.register(MaliciousToken, ());
    let (real_usdc, _, _) = create_real_usdc(env, &admin);

    env.mock_all_auths();
    pool_client.init(&admin, &token_addr);

    (pool_addr, pool_client, token_addr, real_usdc, admin)
}

/// Count events published by `contract` whose topic[0] matches `event_name`.
fn count_events(env: &Env, contract: &Address, event_name: &str) -> u32 {
    let target = Symbol::new(env, event_name);
    let mut count = 0;
    for e in env.events().all().iter() {
        if e.0 != *contract {
            continue;
        }
        let t0: Symbol = e.1.get(0).unwrap().into_val(env);
        if t0 == target {
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Reentrancy Tests — distribute
// ---------------------------------------------------------------------------

/// Distribute → token.transfer → re-enter distribute.
///
/// The re-entrant distribute must be blocked (caught by `try_`).
/// Only the original distribute event is emitted.
#[test]
fn test_distribute_reentry_distribute() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "distribute"));

    let recipient = Address::generate(&env);
    let result = pool_client.try_distribute(&admin, &recipient, &200);

    assert!(result.is_ok(), "first distribute should succeed");
    assert_eq!(
        count_events(&env, &pool_addr, "distribute"),
        1,
        "re-entrant distribute must be blocked"
    );
}

/// Distribute → token.transfer → re-enter set_admin.
///
/// The re-entrant set_admin must be blocked.
/// Admin remains unchanged.
#[test]
fn test_distribute_reentry_set_admin() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "set_admin"));

    let recipient = Address::generate(&env);
    let result = pool_client.try_distribute(&admin, &recipient, &100);

    assert!(
        result.is_ok(),
        "distribute must complete despite set_admin reentry"
    );
    assert_eq!(
        pool_client.get_admin(),
        admin,
        "admin must NOT be changed by re-entrant set_admin"
    );
}

/// Distribute → token.transfer → re-enter pause.
///
/// The re-entrant pause must be blocked.
/// Pool remains unpaused.
#[test]
fn test_distribute_reentry_pause() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "pause"));

    let recipient = Address::generate(&env);
    let result = pool_client.try_distribute(&admin, &recipient, &100);

    assert!(
        result.is_ok(),
        "distribute must complete despite pause reentry"
    );
    assert!(
        !pool_client.is_paused(),
        "pool must NOT be paused by re-entrant pause"
    );
}

// ---------------------------------------------------------------------------
// Reentrancy Tests — batch_distribute
// ---------------------------------------------------------------------------

/// Batch-distribute (2 legs) → first leg transfer → re-enter distribute.
///
/// The re-entrant distribute must be blocked.
/// The batch completes, emitting exactly 2 `batch_distribute` events.
#[test]
fn test_batch_distribute_reentry_distribute() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "distribute"));

    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);
    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
    payments.push_back((dev1, 100));
    payments.push_back((dev2, 100));

    let result = pool_client.try_batch_distribute(&admin, &payments);

    assert!(result.is_ok(), "batch_distribute must succeed");
    assert_eq!(
        count_events(&env, &pool_addr, "distribute"),
        0,
        "re-entrant distribute must be blocked during batch"
    );
    assert_eq!(
        count_events(&env, &pool_addr, "batch_distribute"),
        2,
        "both batch legs must emit events"
    );
}

/// Batch-distribute → first leg transfer → re-enter set_admin.
///
/// Admin rotation must be blocked. Admin unchanged after batch.
#[test]
fn test_batch_distribute_reentry_set_admin() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "set_admin"));

    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
    payments.push_back((Address::generate(&env), 100));

    let result = pool_client.try_batch_distribute(&admin, &payments);

    assert!(
        result.is_ok(),
        "batch_distribute must complete despite set_admin reentry"
    );
    assert_eq!(
        pool_client.get_admin(),
        admin,
        "admin must NOT be changed by re-entrant set_admin during batch"
    );
}

/// Batch-distribute → first leg transfer → re-enter pause.
///
/// Pause must be blocked. Pool remains unpaused after batch.
#[test]
fn test_batch_distribute_reentry_pause() {
    let env = Env::default();
    let (pool_addr, pool_client, token_addr, _real_usdc, admin) = setup_reentrancy_test(&env);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_attack_config(&pool_addr, &admin, &true, &Symbol::new(&env, "pause"));

    let dev1 = Address::generate(&env);
    let dev2 = Address::generate(&env);
    let mut payments: Vec<(Address, i128)> = Vec::new(&env);
    payments.push_back((dev1, 100));
    payments.push_back((dev2, 100));

    let result = pool_client.try_batch_distribute(&admin, &payments);

    assert!(
        result.is_ok(),
        "batch_distribute must complete despite pause reentry"
    );
    assert!(
        !pool_client.is_paused(),
        "pool must NOT be paused by re-entrant pause during batch"
    );
}
