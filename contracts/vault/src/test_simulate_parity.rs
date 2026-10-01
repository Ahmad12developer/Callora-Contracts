//! Parity tests for `simulate_deduct` vs `deduct` (Issue #1115).
//!
//! Verifies that `simulate_deduct` and `deduct` share `validate_deduct` and
//! therefore return matching error codes for every validation-stage input.
//! All test function names contain "simulate" so `cargo test -p callora-vault simulate`
//! runs them.

extern crate std;

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{token, Address, Env};

use super::*;
use callora_settlement::CalloraSettlement;

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

/// Set up a vault with:
/// - `tracked` initial balance
/// - `on_ledger` USDC minted to vault (for real deducts)
/// - `authorized_caller` as the deduct caller
/// - `min_deposit = 1`, `max_deduct = max_deduct`
///
/// Returns `(client, vault_addr, auth_caller, usdc_admin)`.
fn setup_simulate_vault<'a>(
    env: &'a Env,
    tracked: i128,
    on_ledger: i128,
    max_deduct: i128,
) -> (
    CalloraVaultClient<'a>,
    Address,
    Address,
    token::StellarAssetClient<'a>,
) {
    let owner = Address::generate(env);
    let auth_caller = Address::generate(env);
    let (vault_addr, client) = create_vault(env);
    let (usdc, _usdc_client, usdc_admin) = create_usdc(env, &owner);
    let settlement = create_settlement(env, &owner, &vault_addr);

    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(tracked),
        &Some(auth_caller.clone()),
        &Some(1i128),
        &None::<Address>,
        &Some(max_deduct),
        &Some(settlement),
    );
    usdc_admin.mint(&vault_addr, &on_ledger);

    (client, vault_addr, auth_caller, usdc_admin)
}

// ---------------------------------------------------------------------------
// Proptest: simulate_deduct and try_deduct return the same error code
// ---------------------------------------------------------------------------

/// Helper to extract the `VaultError` discriminant from a `try_*` result.
///
/// Returns `Some(code)` when the call returned a `VaultError`, or `None` when
/// the call succeeded.
fn err_code_from<V>(
    result: Result<Result<V, soroban_sdk::Error>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Option<u32> {
    match result {
        Err(Ok(e)) => Some(e.get_code()),
        Ok(Err(e)) => Some(e.get_code()),
        Ok(Ok(_)) => None,
        Err(Err(_)) => None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For any combination of vault state and deduct inputs, `simulate_deduct`
    /// and `try_deduct` must return the same error code (or both succeed).
    ///
    /// The proptest explores:
    /// - `amount` in `[-10, 200]` to cover negative, zero, below-min, in-range, above-max, and above-balance
    /// - `use_auth_caller: bool` to cover authorized vs unauthorized
    /// - `paused: bool` to cover the circuit-breaker
    /// - `balance` in `[0, 150]`
    #[test]
    fn simulate_deduct_matches_deduct_error_code(
        amount in -10i128..=200i128,
        use_auth_caller in proptest::bool::ANY,
        paused in proptest::bool::ANY,
        balance in 0i128..=150i128,
    ) {
        let env = Env::default();
        // max_deduct = 100; min_deposit = 1; on_ledger ≥ balance so real deducts don't
        // fail due to token balance (validation errors fire first).
        let (client, _vault, auth_caller, usdc_admin) =
            setup_simulate_vault(&env, balance, balance.max(200), 100);

        env.mock_all_auths();

        if paused {
            // Owner == auth_caller is not true here; pause via direct storage write
            // to avoid the owner/auth_caller dependency.
            env.as_contract(&_vault, || {
                env.storage().instance().set(&DataKey::Paused, &true);
            });
        }

        let caller = if use_auth_caller {
            auth_caller.clone()
        } else {
            Address::generate(&env)
        };

        let request_id = 42u64;

        let sim_result = client.try_simulate_deduct(&caller, &amount, &request_id);
        let deduct_result = client.try_deduct(&caller, &amount, &request_id);

        let sim_code = err_code_from(sim_result);
        let deduct_code = err_code_from(deduct_result);

        prop_assert_eq!(
            sim_code, deduct_code,
            "simulate_deduct and deduct returned different outcomes for \
             amount={amount} use_auth={use_auth_caller} paused={paused} balance={balance}"
        );
    }
}

// ---------------------------------------------------------------------------
// Explicit parity tests
// ---------------------------------------------------------------------------

/// simulate_deduct and deduct both return BelowMinDeposit for an amount below min.
#[test]
fn simulate_deduct_below_min_matches_deduct() {
    let env = Env::default();
    let (client, _vault, auth_caller, _usdc_admin) =
        setup_simulate_vault(&env, 1_000, 1_000, 500);
    env.mock_all_auths();

    // min_deposit = 1; pass amount = 0 to trigger AmountNotPositive,
    // and amount = -5 to be safe. Use amount that is valid positive but < min
    // by injecting a min via a fresh vault with min_deposit=10.
    let owner2 = Address::generate(&env);
    let auth2 = Address::generate(&env);
    let (_vault2_addr, client2) = {
        let address = env.register(CalloraVault, ());
        let c = CalloraVaultClient::new(&env, &address);
        let (usdc2, _, usdc2_admin) = create_usdc(&env, &owner2);
        let settlement2 = create_settlement(&env, &owner2, &address);
        c.init(
            &owner2,
            &usdc2,
            &Some(1_000i128),
            &Some(auth2.clone()),
            &Some(10i128),       // min_deposit = 10
            &None::<Address>,
            &Some(1_000i128),
            &Some(settlement2),
        );
        usdc2_admin.mint(&address, &1_000i128);
        (address, c)
    };

    // amount=5 is positive but below min_deposit=10 → BelowMinDeposit
    let sim = client2.try_simulate_deduct(&auth2, &5i128, &1u64);
    let ded = client2.try_deduct(&auth2, &5i128, &1u64);
    assert_eq!(err_code_from(sim), err_code_from(ded));
    assert_eq!(
        err_code_from(client2.try_simulate_deduct(&auth2, &5i128, &1u64)),
        Some(VaultError::BelowMinDeposit as u32)
    );
}

/// simulate_deduct and deduct both return Unauthorized for a non-auth caller.
#[test]
fn simulate_deduct_unauthorized_matches_deduct() {
    let env = Env::default();
    let (client, _vault, _auth_caller, _) = setup_simulate_vault(&env, 1_000, 1_000, 500);
    env.mock_all_auths();

    let stranger = Address::generate(&env);

    let sim = client.try_simulate_deduct(&stranger, &100i128, &1u64);
    let ded = client.try_deduct(&stranger, &100i128, &1u64);
    assert_eq!(err_code_from(sim), err_code_from(ded));
    assert_eq!(
        err_code_from(client.try_simulate_deduct(&stranger, &100i128, &1u64)),
        Some(VaultError::Unauthorized as u32)
    );
}

/// simulate_deduct and deduct both return Paused when the vault is paused.
#[test]
fn simulate_deduct_paused_matches_deduct() {
    let env = Env::default();
    let (client, vault, auth_caller, _) = setup_simulate_vault(&env, 1_000, 1_000, 500);
    env.mock_all_auths();

    // Write Paused=true directly so we don't need the owner address.
    env.as_contract(&vault, || {
        env.storage().instance().set(&DataKey::Paused, &true);
    });

    let sim = client.try_simulate_deduct(&auth_caller, &100i128, &1u64);
    let ded = client.try_deduct(&auth_caller, &100i128, &1u64);
    assert_eq!(err_code_from(sim), err_code_from(ded));
    assert_eq!(
        err_code_from(client.try_simulate_deduct(&auth_caller, &100i128, &1u64)),
        Some(VaultError::Paused as u32)
    );
}

/// simulate_deduct and deduct both return InsufficientBalance when balance < amount.
#[test]
fn simulate_deduct_insufficient_balance_matches_deduct() {
    let env = Env::default();
    // tracked = 50, on_ledger = 50, max_deduct = 500
    let (client, _vault, auth_caller, _) = setup_simulate_vault(&env, 50, 50, 500);
    env.mock_all_auths();

    // amount=100 > balance=50
    let sim = client.try_simulate_deduct(&auth_caller, &100i128, &1u64);
    let ded = client.try_deduct(&auth_caller, &100i128, &1u64);
    assert_eq!(err_code_from(sim), err_code_from(ded));
    assert_eq!(
        err_code_from(client.try_simulate_deduct(&auth_caller, &100i128, &1u64)),
        Some(VaultError::InsufficientBalance as u32)
    );
}

/// simulate_deduct returns Ok (projected balance) when deduct would also succeed.
#[test]
fn simulate_deduct_success_matches_deduct_projected_balance() {
    let env = Env::default();
    let (client, _vault, auth_caller, _) = setup_simulate_vault(&env, 1_000, 1_000, 500);
    env.mock_all_auths();

    // simulate returns the projected new balance
    let sim = client.try_simulate_deduct(&auth_caller, &200i128, &1u64);
    assert_eq!(sim, Ok(Ok(800i128)));

    // real deduct succeeds and the actual balance matches
    assert!(client.try_deduct(&auth_caller, &200i128, &1u64).is_ok());
    assert_eq!(client.balance(), 800i128);
}
