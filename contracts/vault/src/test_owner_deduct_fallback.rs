//! Regression tests for the owner-deduction fallback required by
//! [Issue #1107 — "Honor owner deduction fallback when caller unset"]:
//!
//! `set_authorized_caller` documents that `None` means *"only the owner can
//! deduct"*, and [`CalloraVault::require_authorized_deduct_caller`] implements
//! exactly that owner-or-caller rule. The inline checks in `deduct` and
//! `batch_deduct` nevertheless read `DataKey::AuthorizedCaller` as a bare
//! `Address` and did:
//!
//! ```ignore
//! .get::<_, Address>(&DataKey::AuthorizedCaller)
//! .unwrap_or_else(|| panic!("Authorized caller not set"))
//! ```
//!
//! Two things were wrong with that. The stored value is an `Option<Address>`,
//! so the type never matched, and clearing the caller during an incident
//! aborted all metering with an untyped host panic instead of falling back to
//! the owner. These tests pin the corrected behaviour:
//!
//! 1. The owner can `deduct` when the authorized caller is `None`.
//! 2. The owner can `deduct` when a *different* authorized caller is set.
//! 3. A stranger gets a typed `Unauthorized`, never a panic.
//! 4. Caller rotation via `set_authorized_caller` (with nonce) then `deduct`
//!    works, and rotating back to `None` restores owner-only deduction.
//! 5. The same rules hold for `batch_deduct`.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{token, Address, Env, Error, InvokeError, Vec};

use super::*;
use callora_settlement::CalloraSettlement;

/// True if a `try_*` wrapper returned a specific vault error code.
///
/// A host-level panic would surface as `Err(Err(InvokeError))` instead, so this
/// doubles as the "no code path panics" assertion the issue asks for.
fn is_vault_err<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
    expected: u32,
) -> bool {
    match result {
        Err(Ok(e)) => e.into().get_code() == expected,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (
        addr.clone(),
        token::Client::new(env, &addr),
        token::StellarAssetClient::new(env, &addr),
    )
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let address = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &address);
    (address, client)
}

fn create_settlement(env: &Env, admin: &Address, vault_address: &Address) -> Address {
    let settlement_address = env.register(CalloraSettlement, ());
    let settlement_client =
        callora_settlement::CalloraSettlementClient::new(env, &settlement_address);
    env.mock_all_auths();
    settlement_client.init(admin, vault_address);
    settlement_address
}

/// Fund a vault with `tracked` balance and matching on-ledger USDC, initialized
/// with whatever `authorized_caller` the test wants to exercise.
fn setup(env: &Env, authorized_caller: Option<Address>) -> (CalloraVaultClient<'_>, Address) {
    const TRACKED: i128 = 1_000;
    const MAX_DEDUCT: i128 = 500;

    let owner = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, _usdc_client, usdc_admin) = create_usdc(env, &owner);
    let settlement = create_settlement(env, &owner, &vault_addr);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(TRACKED),
        &authorized_caller,
        &Some(1i128),
        &None::<Address>,
        &Some(MAX_DEDUCT),
        &Some(settlement),
    );
    usdc_admin.mint(&vault_addr, &TRACKED);

    (client, owner)
}

fn items_from(env: &Env, amounts: &[i128]) -> Vec<(i128, u64)> {
    let mut v: Vec<(i128, u64)> = Vec::new(env);
    for (i, a) in amounts.iter().enumerate() {
        v.push_back((*a, i as u64));
    }
    v
}

// ---------------------------------------------------------------------------
// deduct
// ---------------------------------------------------------------------------

/// The headline regression: `None` means owner-only, and must not panic.
#[test]
fn owner_can_deduct_when_authorized_caller_is_none() {
    let env = Env::default();
    let (client, owner) = setup(&env, None);

    let result = client.try_deduct(&owner, &100i128, &1u64);

    assert!(result.is_ok(), "owner deduct with unset caller must succeed");
    assert_eq!(client.balance(), 900);
}

/// The documented policy is owner-*or*-caller, so a set caller does not lock the
/// owner out.
#[test]
fn owner_can_deduct_when_a_different_authorized_caller_is_set() {
    let env = Env::default();
    let other = Address::generate(&env);
    let (client, owner) = setup(&env, Some(other));

    let result = client.try_deduct(&owner, &100i128, &1u64);

    assert!(result.is_ok(), "owner must always be able to deduct");
    assert_eq!(client.balance(), 900);
}

/// A stranger must get a typed error, not the old untyped host panic.
#[test]
fn stranger_is_unauthorized_and_does_not_panic_when_caller_is_none() {
    let env = Env::default();
    let (client, _owner) = setup(&env, None);
    let stranger = Address::generate(&env);

    let result = client.try_deduct(&stranger, &100i128, &1u64);

    assert!(
        is_vault_err(result, VaultError::Unauthorized as u32),
        "expected a typed Unauthorized contract error (a panic would surface as Err(Err(..)))",
    );
    assert_eq!(client.balance(), 1_000, "rejected deduct must not mutate");
}

#[test]
fn stranger_is_unauthorized_when_a_different_caller_is_set() {
    let env = Env::default();
    let other = Address::generate(&env);
    let (client, _owner) = setup(&env, Some(other));
    let stranger = Address::generate(&env);

    let result = client.try_deduct(&stranger, &100i128, &1u64);

    assert!(is_vault_err(result, VaultError::Unauthorized as u32));
    assert_eq!(client.balance(), 1_000);
}

/// Acceptance criterion: caller rotation via `set_authorized_caller` with a
/// nonce, followed by `deduct` by the newly-authorized caller.
#[test]
fn rotated_caller_can_deduct_and_stranger_cannot() {
    let env = Env::default();
    let (client, _owner) = setup(&env, None);
    let rotated = Address::generate(&env);

    env.mock_all_auths();
    client.set_authorized_caller(&Some(rotated.clone()), &0u64);

    assert!(
        client.try_deduct(&rotated, &100i128, &1u64).is_ok(),
        "rotated caller must be able to deduct",
    );

    let stranger = Address::generate(&env);
    assert!(
        is_vault_err(
            client.try_deduct(&stranger, &50i128, &2u64),
            VaultError::Unauthorized as u32
        ),
        "stranger must stay unauthorized after rotation",
    );
}

/// Clearing the caller restores owner-only deduction, and the previous caller
/// loses access — the incident-response path the issue describes.
#[test]
fn clearing_the_caller_restores_owner_only_deduction() {
    let env = Env::default();
    let previous = Address::generate(&env);
    let (client, owner) = setup(&env, Some(previous.clone()));

    env.mock_all_auths();
    client.set_authorized_caller(&None::<Address>, &0u64);

    assert!(
        client.try_deduct(&owner, &100i128, &1u64).is_ok(),
        "owner must be able to deduct once the caller is cleared",
    );
    assert!(
        is_vault_err(
            client.try_deduct(&previous, &50i128, &2u64),
            VaultError::Unauthorized as u32
        ),
        "the cleared caller must no longer be able to deduct",
    );
    assert_eq!(client.balance(), 900);
}

// ---------------------------------------------------------------------------
// batch_deduct
// ---------------------------------------------------------------------------

#[test]
fn owner_can_batch_deduct_when_authorized_caller_is_none() {
    let env = Env::default();
    let (client, owner) = setup(&env, None);
    let items = items_from(&env, &[10, 20, 30]);

    let result = client.try_batch_deduct(&owner, &items);

    assert!(result.is_ok(), "owner batch deduct must succeed");
    assert_eq!(client.balance(), 940);
}

#[test]
fn stranger_batch_deduct_is_unauthorized_and_does_not_panic() {
    let env = Env::default();
    let (client, _owner) = setup(&env, None);
    let stranger = Address::generate(&env);
    let items = items_from(&env, &[10, 20, 30]);

    let result = client.try_batch_deduct(&stranger, &items);

    assert!(
        is_vault_err(result, VaultError::Unauthorized as u32),
        "expected a typed Unauthorized contract error, not a host panic",
    );
    assert_eq!(client.balance(), 1_000, "rejected batch must not mutate");
}

#[test]
fn rotated_caller_can_batch_deduct() {
    let env = Env::default();
    let (client, _owner) = setup(&env, None);
    let rotated = Address::generate(&env);
    let items = items_from(&env, &[10, 20]);

    env.mock_all_auths();
    client.set_authorized_caller(&Some(rotated.clone()), &0u64);

    assert!(client.try_batch_deduct(&rotated, &items).is_ok());
    assert_eq!(client.balance(), 970);
}
