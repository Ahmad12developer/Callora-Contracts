extern crate std;

use super::*;
use soroban_sdk::{Address, Env};

fn create_usdc<'a>(env: &'a Env, admin: &'a Address) -> Address {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    ca.address()
}

#[test]
fn fuzz_like_set_authorized_caller_auth_and_nonce_invariants() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(&env, &vault_addr);

    env.mock_all_auths();
    let usdc = create_usdc(&env, &owner);
    client.init(&owner, &usdc, &Some(0), &Some(owner.clone()), &Some(1), &None, &None);

    // Deterministic pseudo-fuzz matrix over auth mode / nonce mode / caller kind.
    for auth_enabled in [false, true] {
        for use_current_nonce in [false, true] {
            for set_to_vault_address in [false, true] {
                let expected_nonce = if use_current_nonce {
                    client.get_authorized_caller_nonce()
                } else {
                    client.get_authorized_caller_nonce().wrapping_add(7)
                };

                let candidate = if set_to_vault_address {
                    Some(vault_addr.clone())
                } else {
                    Some(Address::generate(&env))
                };

                let before_meta = client.get_meta();
                let before_nonce = client.get_authorized_caller_nonce();

                if auth_enabled {
                    env.mock_all_auths();
                } else {
                    env.set_auths(&[]);
                }

                let result = client.try_set_authorized_caller(&candidate, &expected_nonce);
                let after_meta = client.get_meta();
                let after_nonce = client.get_authorized_caller_nonce();

                if !auth_enabled {
                    assert!(result.is_err());
                    assert_eq!(after_meta.authorized_caller, before_meta.authorized_caller);
                    assert_eq!(after_nonce, before_nonce);
                    continue;
                }

                if set_to_vault_address {
                    assert_eq!(result, Err(Ok(VaultError::AuthorizedCallerCannotBeVault)));
                    assert_eq!(after_meta.authorized_caller, before_meta.authorized_caller);
                    assert_eq!(after_nonce, before_nonce);
                    continue;
                }

                if use_current_nonce {
                    assert!(result.is_ok());
                    assert_eq!(after_nonce, before_nonce.wrapping_add(1));
                } else {
                    assert_eq!(result, Err(Ok(VaultError::StaleNonce)));
                    assert_eq!(after_nonce, before_nonce);
                }
            }
        }
    }
}
