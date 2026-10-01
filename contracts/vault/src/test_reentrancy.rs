extern crate std;

use crate::{CalloraVault, CalloraVaultClient};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{contract, contractimpl, Address, Env, IntoVal, Symbol, Vec};

// ---------------------------------------------------------------------------
// Malicious token mock
//
// `transfer` tries to call back into the vault's `deduct` while the vault is
// already executing. It records whether it tried and whether the vault
// blocked the re-entry.
// ---------------------------------------------------------------------------

#[contract]
pub struct MaliciousToken;

#[contractimpl]
impl MaliciousToken {
    pub fn transfer(env: Env, from: Address, _to: Address, _amount: i128) {
        from.require_auth();

        let vault_addr: Option<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "vault_addr"));
        let attack_active: bool = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "attack_active"))
            .unwrap_or(false);

        if attack_active {
            if let Some(vault) = vault_addr {
                // Prevent infinite recursion in the mock
                env.storage()
                    .instance()
                    .set(&Symbol::new(&env, "attack_active"), &false);

                let caller: Address = env
                    .storage()
                    .instance()
                    .get(&Symbol::new(&env, "attack_caller"))
                    .unwrap();
                let client = CalloraVaultClient::new(&env, &vault);

                // Attempt re-entry into deduct while the vault is mid-call.
                let res = client.try_deduct(&caller, &1, &999u64);

                env.storage()
                    .instance()
                    .set(&Symbol::new(&env, "attempted"), &true);
                env.storage()
                    .instance()
                    .set(&Symbol::new(&env, "blocked"), &res.is_err());
            }
        }
    }

    pub fn balance(_env: Env, _id: Address) -> i128 {
        1_000_000_000
    }

    pub fn set_token_attack_config(env: Env, vault: Address, caller: Address, active: bool) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "vault_addr"), &vault);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_caller"), &caller);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "attack_active"), &active);
    }

    /// True once the mock has tried to re-enter the vault.
    pub fn attack_attempted(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "attempted"))
            .unwrap_or(false)
    }

    /// True if the vault rejected the re-entrant call.
    pub fn reentry_blocked(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "blocked"))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `(vault_addr, vault_client, token_addr, owner)`.
/// `authorized` is the authorized deduct caller; defaults to the owner.
fn setup_reentrancy_test(
    env: &Env,
    authorized: Option<Address>,
) -> (Address, CalloraVaultClient<'_>, Address, Address) {
    let owner = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let vault_client = CalloraVaultClient::new(env, &vault_addr);

    let token_addr = env.register(MaliciousToken, ());
    // The settlement client is a no-op stub in native builds, so any address works.
    let settlement_addr = Address::generate(env);
    let authorized_caller = authorized.unwrap_or_else(|| owner.clone());

    env.mock_all_auths();

    // Init the vault with the malicious token as its USDC token.
    vault_client.init(
        &owner,
        &token_addr,
        &Some(1000),
        &Some(authorized_caller),
        &Some(1),
        &None,
        &None,
        &Some(settlement_addr),
    );

    (vault_addr, vault_client, token_addr, owner)
}

/// Number of `deduct` events the vault has emitted.
fn deduct_event_count(env: &Env, vault: &Address) -> u32 {
    let deduct = crate::events::event_deduct(env);
    let mut count = 0;
    for e in env.events().all().iter() {
        if e.0 != *vault {
            continue;
        }
        let topic0: Symbol = e.1.get(0).unwrap().into_val(env);
        if topic0 == deduct {
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Reentrancy tests
// ---------------------------------------------------------------------------

#[test]
fn test_reentrancy_via_token_transfer_is_blocked_by_auth() {
    let env = Env::default();
    let (vault_addr, vault_client, token_addr, owner) = setup_reentrancy_test(&env, None);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_token_attack_config(&vault_addr, &owner, &true);

    assert_eq!(vault_client.balance(), 1000);

    // deduct -> token.transfer -> vault.deduct (re-entry)
    let result = vault_client.try_deduct(&owner, &100, &1u64);
    let deduct_events = deduct_event_count(&env, &vault_addr);

    assert!(result.is_ok(), "First deduct should succeed");
    assert!(token_mock.attack_attempted(), "The attack must have run");
    assert!(token_mock.reentry_blocked(), "The vault must block re-entry");
    assert_eq!(
        vault_client.balance(),
        900,
        "Balance should only be deducted once"
    );
    assert_eq!(deduct_events, 1, "Only the outer deduct may emit an event");
}

#[test]
fn test_batch_deduct_reentrancy_via_token() {
    let env = Env::default();
    let (vault_addr, vault_client, token_addr, owner) = setup_reentrancy_test(&env, None);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_token_attack_config(&vault_addr, &owner, &true);

    let items = Vec::from_array(&env, [(50i128, 1u64), (50i128, 2u64)]);

    let result = vault_client.try_batch_deduct(&owner, &items);
    let deduct_events = deduct_event_count(&env, &vault_addr);

    assert!(result.is_ok(), "Batch deduct should succeed");
    assert!(token_mock.attack_attempted(), "The attack must have run");
    assert!(token_mock.reentry_blocked(), "The vault must block re-entry");
    assert_eq!(
        vault_client.balance(),
        900,
        "Balance should only be deducted by the batch amount"
    );
    // One event per batch item, none from the re-entrant call.
    assert_eq!(deduct_events, 2);
}

#[test]
fn test_reentrancy_by_authorized_attacker() {
    let env = Env::default();
    let attacker = Address::generate(&env);
    let (vault_addr, vault_client, token_addr, _owner) =
        setup_reentrancy_test(&env, Some(attacker.clone()));

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    token_mock.set_token_attack_config(&vault_addr, &attacker, &true);

    assert_eq!(vault_client.balance(), 1000);

    // The attacker is the authorized caller: deduct -> token.transfer ->
    // attacker calls vault.deduct again.
    let result = vault_client.try_deduct(&attacker, &100, &1u64);
    let deduct_events = deduct_event_count(&env, &vault_addr);

    assert!(result.is_ok(), "First deduct should succeed");
    assert!(token_mock.attack_attempted(), "The attack must have run");
    assert!(
        token_mock.reentry_blocked(),
        "Re-entry by an authorized caller must still be blocked"
    );
    assert_eq!(
        vault_client.balance(),
        900,
        "Balance should only be deducted once"
    );
    assert_eq!(deduct_events, 1);
}

#[test]
fn test_withdraw_reentrancy_via_token() {
    let env = Env::default();
    let (vault_addr, vault_client, token_addr, owner) = setup_reentrancy_test(&env, None);

    let token_mock = MaliciousTokenClient::new(&env, &token_addr);
    // withdraw calls token.transfer; the mock tries to call deduct() during it.
    token_mock.set_token_attack_config(&vault_addr, &owner, &true);

    let result = vault_client.try_withdraw(&100);
    let deduct_events = deduct_event_count(&env, &vault_addr);

    assert!(result.is_ok(), "Withdraw should succeed");
    assert!(token_mock.attack_attempted(), "The attack must have run");
    assert!(token_mock.reentry_blocked(), "The vault must block re-entry");
    assert_eq!(
        vault_client.balance(),
        900,
        "Balance should only be deducted by the withdraw amount"
    );
    assert_eq!(deduct_events, 0, "No deduct may occur during a withdraw");
}