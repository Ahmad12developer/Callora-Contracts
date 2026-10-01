//! Authorization-parity coverage for the deposit allowlist (issue #1110).
//!
//! `is_authorized_depositor` used to read `DataKey::Depositor(caller)` — a key
//! nothing ever writes — while `deposit`, `add_address`, and `clear_all` all
//! operate on the `StorageKey::AllowedDepositors` vector.  The view therefore
//! always returned `false`, even for allowlisted addresses, and frontends that
//! pre-checked eligibility through it would refuse valid deposits.
//!
//! # Invariants asserted here
//!
//! 1. **Add**: after `add_address`, `is_authorized_depositor(addr)` is `true`.
//! 2. **Clear**: after `clear_all`, it is `false` (and `deposit` from that
//!    address is rejected with `VaultError::CallerNotInAllowlist`, code 44).
//! 3. **Owner**: the owner is always authorized, regardless of allowlist state
//!    (documented owner bypass, mirrors `deposit`).
//! 4. **Parity (property test)**: for randomly generated addresses under a
//!    random sequence of `add_address` / duplicate-add / `clear_all` operations,
//!    the view agrees with `deposit` acceptance exactly — `deposit` succeeds
//!    iff the view returned `true`, and is rejected with code 44 (leaving the
//!    tracked balance untouched) iff the view returned `false`.
//! 5. **Pre-init**: the view returns `false` before `init` (no panic).
//!
//! # Determinism
//!
//! The property test is a seeded xorshift trace (seeds 0..`SEED_COUNT`), so
//! failures are reproducible; the failing seed, step, and address are
//! included in every assertion message.

extern crate std;

use std::vec::Vec as StdVec;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Error, InvokeError};

use super::*;

/// Amount used for every probe deposit.  ≥ `min_deposit` (1) configured by
/// [`setup`].
const DEPOSIT_AMOUNT: i128 = 100;

/// Faucet amount minted to the probe address before each `deposit` attempt so
/// a wrongly-accepted deposit surfaces as `Ok`, not as a token-funding error.
const MINT_AMOUNT: i128 = 100_000;

/// Number of deterministic seeds for the property test.
const SEED_COUNT: u64 = 8;

/// Addresses participating in each property-test trace (plus the owner and a
/// freshly generated stranger per verification pass).
const ADDR_POOL_SIZE: usize = 6;

/// Mutating operations per trace before re-verifying parity.
const OPS_PER_TRACE: usize = 12;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// True if the `try_*` wrapper returned a specific vault error code.
///
/// The Soroban `try_*` client returns `Result<Result<V, CE>, Result<E,
/// InvokeError>>`; a contract-level `Err(VaultError)` surfaces as the outer
/// `Err` wrapping the inner `Ok(e)` (the established convention across this
/// workspace's tests).
fn is_vault_err<V, CE: Into<Error>, E: Into<Error>>(
    result: Result<Result<V, CE>, Result<E, InvokeError>>,
    expected: u32,
) -> bool {
    match result {
        Err(Ok(e)) => e.into().get_code() == expected,
        _ => false,
    }
}

/// Initialize a vault with `min_deposit = 1` and a real USDC SAC.
///
/// Returns `(owner, usdc_admin, client)`.
fn setup(
    env: &Env,
) -> (
    Address,
    token::StellarAssetClient<'_>,
    CalloraVaultClient<'_>,
) {
    env.mock_all_auths_allowing_non_root_auth();
    let owner = Address::generate(env);
    let usdc_addr = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let usdc_admin = token::StellarAssetClient::new(env, &usdc_addr);

    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);
    let settlement = Address::generate(env);
    client.init(
        &owner,
        &usdc_addr,
        &Some(0i128),
        &None,
        &Some(1i128),
        &None,
        &None,
        &Some(settlement),
    );
    (owner, usdc_admin, client)
}

/// Assert that the view and `deposit` agree for `addr`.
///
/// When `expected` is `true` the deposit must succeed and increase the
/// tracked balance by `DEPOSIT_AMOUNT`; otherwise the deposit must fail with
/// `VaultError::CallerNotInAllowlist` (44) and leave the balance untouched.
fn assert_parity(
    client: &CalloraVaultClient<'_>,
    usdc_admin: &token::StellarAssetClient<'_>,
    addr: &Address,
    expected: bool,
    ctx: &str,
) {
    let view = client.is_authorized_depositor(addr);
    assert_eq!(
        view, expected,
        "{ctx}: view returned {view}, expected {expected} for {addr:?}"
    );

    // Top up before probing so a wrongly-accepted deposit surfaces as `Ok`
    // (caught below) rather than as an unrelated token-funding error.
    usdc_admin.mint(addr, &MINT_AMOUNT);

    let before = client.balance();
    let res = client.try_deposit(addr, &DEPOSIT_AMOUNT);
    if expected {
        assert!(
            res.is_ok(),
            "{ctx}: deposit must succeed when the view reports true for {addr:?}"
        );
        assert_eq!(
            client.balance(),
            before + DEPOSIT_AMOUNT,
            "{ctx}: successful deposit must increase the tracked balance for {addr:?}"
        );
    } else {
        assert!(
            is_vault_err(res, VaultError::CallerNotInAllowlist as u32),
            "{ctx}: deposit must be rejected with CallerNotInAllowlist (44) \
             when the view reports false for {addr:?}"
        );
        assert_eq!(
            client.balance(),
            before,
            "{ctx}: rejected deposit must not change the tracked balance for {addr:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance criteria (issue #1110)
// ---------------------------------------------------------------------------

/// AC: after `add_address`, `is_authorized_depositor` returns `true`.
#[test]
fn view_returns_true_after_add_address() {
    let env = Env::default();
    let (owner, _usdc_admin, client) = setup(&env);
    let depositor = Address::generate(&env);

    assert!(
        !client.is_authorized_depositor(&depositor),
        "fresh address must not be authorized before add_address"
    );

    client.add_address(&owner, &depositor);

    assert!(
        client.is_authorized_depositor(&depositor),
        "view must return true after add_address"
    );
}

/// AC: after `clear_all`, `is_authorized_depositor` returns `false` — and the
/// deposit path agrees by rejecting with code 44.
#[test]
fn view_returns_false_after_clear_all() {
    let env = Env::default();
    let (owner, usdc_admin, client) = setup(&env);
    let depositor = Address::generate(&env);

    client.add_address(&owner, &depositor);
    assert!(client.is_authorized_depositor(&depositor));

    client.clear_all(&owner);

    assert!(
        !client.is_authorized_depositor(&depositor),
        "view must return false after clear_all"
    );
    assert_parity(&client, &usdc_admin, &depositor, false, "after clear_all");
}

/// Documented owner bypass: the owner is authorized even when the allowlist
/// is empty or has just been cleared.
#[test]
fn owner_is_authorized_regardless_of_allowlist_state() {
    let env = Env::default();
    let (owner, usdc_admin, client) = setup(&env);
    let other = Address::generate(&env);

    assert!(
        client.is_authorized_depositor(&owner),
        "owner must be authorized on a fresh vault"
    );

    client.add_address(&owner, &other);
    assert!(client.is_authorized_depositor(&owner));

    client.clear_all(&owner);
    assert!(
        client.is_authorized_depositor(&owner),
        "owner must remain authorized after clear_all"
    );
    assert_parity(&client, &usdc_admin, &owner, true, "owner after clear_all");
}

/// Duplicate adds are idempotent: the view stays `true` and the list does
/// not grow.
#[test]
fn duplicate_add_is_idempotent_for_view_and_list() {
    let env = Env::default();
    let (owner, _usdc_admin, client) = setup(&env);
    let depositor = Address::generate(&env);

    client.add_address(&owner, &depositor);
    client.add_address(&owner, &depositor);

    assert!(client.is_authorized_depositor(&depositor));
    assert_eq!(
        client.get_allowlist().len(),
        1,
        "duplicate add must not extend the allowlist"
    );
}

/// The view is a non-panicking read before `init`: no owner, no allowlist,
/// so it reports `false`.
#[test]
fn view_returns_false_before_init() {
    let env = Env::default();
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(&env, &vault_addr);
    let stranger = Address::generate(&env);

    assert!(
        !client.is_authorized_depositor(&stranger),
        "view must return false (not panic) before init"
    );
}

// ---------------------------------------------------------------------------
// Property test: view ≡ deposit acceptance
// ---------------------------------------------------------------------------

/// Deterministic xorshift64 PRNG so failing traces replay exactly.
struct Prng {
    state: u64,
}

impl Prng {
    fn new(seed: u64) -> Self {
        // Never allow the all-zero fixed point.
        Self {
            state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Assert the view matches `deposit` acceptance for every address in the
/// pool, plus the owner and a freshly generated stranger, and that
/// `get_allowlist` matches the model list.
fn verify_parity(
    env: &Env,
    client: &CalloraVaultClient<'_>,
    usdc_admin: &token::StellarAssetClient<'_>,
    owner: &Address,
    pool: &StdVec<Address>,
    model: &StdVec<Address>,
    seed: u64,
    step: usize,
) {
    let list = client.get_allowlist();
    assert_eq!(
        list.len() as usize,
        model.len(),
        "seed={seed} step={step}: get_allowlist length diverged from the model"
    );

    let stranger = Address::generate(env);
    for addr in pool {
        let model_has = model.contains(addr);
        assert_eq!(
            list.contains(addr),
            model_has,
            "seed={seed} step={step}: get_allowlist membership diverged for {addr:?}"
        );
        let expected = model_has;
        let ctx = std::format!("seed={seed} step={step} pool member");
        assert_parity(client, usdc_admin, addr, expected, &ctx);
    }

    let ctx = std::format!("seed={seed} step={step} owner");
    assert_parity(client, usdc_admin, owner, true, &ctx);

    let ctx = std::format!("seed={seed} step={step} stranger");
    assert_parity(client, usdc_admin, &stranger, false, &ctx);
}

/// AC: a property test asserting the view matches deposit acceptance for
/// random addresses under random allowlist operation sequences.
#[test]
fn view_matches_deposit_acceptance_for_random_addresses() {
    for seed in 0..SEED_COUNT {
        let env = Env::default();
        let (owner, usdc_admin, client) = setup(&env);
        let mut prng = Prng::new(seed);

        let mut pool: StdVec<Address> = StdVec::new();
        for _ in 0..ADDR_POOL_SIZE {
            pool.push(Address::generate(&env));
        }
        for addr in &pool {
            usdc_admin.mint(addr, &MINT_AMOUNT);
        }
        usdc_admin.mint(&owner, &MINT_AMOUNT);

        let mut model: StdVec<Address> = StdVec::new();

        for step in 0..OPS_PER_TRACE {
            match prng.below(3) {
                // add_address (idempotent whether or not already present)
                0 => {
                    let addr = pool[prng.below(pool.len())].clone();
                    client.add_address(&owner, &addr);
                    if !model.contains(&addr) {
                        model.push(addr);
                    }
                }
                // duplicate add of an already-allowed address when possible
                1 => {
                    if model.is_empty() {
                        let addr = pool[prng.below(pool.len())].clone();
                        client.add_address(&owner, &addr);
                        model.push(addr);
                    } else {
                        let addr = model[prng.below(model.len())].clone();
                        client.add_address(&owner, &addr);
                    }
                }
                // clear_all
                _ => {
                    client.clear_all(&owner);
                    model.clear();
                }
            }
            verify_parity(
                &env,
                &client,
                &usdc_admin,
                &owner,
                &pool,
                &model,
                seed,
                step,
            );
        }
    }
}
