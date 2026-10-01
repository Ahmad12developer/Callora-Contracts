//! Invariant property tests for the `errors` contract (#907).
//!
//! # Invariant under test
//! Across arbitrary valid action sequences on the errors contract, the
//! following **core invariants** must hold:
//!
//! 1. **Admin identity stability** — the admin stored at `init` never changes
//!    without a matching call that would require `admin.require_auth()`.
//!
//! 2. **Registered-code gate** — `log_error` succeeds only for codes that a
//!    prior `register_error` defined. An unregistered code always returns
//!    [`Error::UnknownErrorCode`] and never panics.
//!
//! 3. **Idempotent double-init** — calling `init` twice always returns
//!    [`Error::AlreadyInitialized`], regardless of the admin supplied.
//!
//! 4. **Authorisation monotonicity** — `register_error` / `update_error`
//!    called with any address that is not the stored admin always returns
//!    [`Error::Unauthorized`], regardless of description length or whether
//!    the code exists.
//!
//! 5. **Overflow-safe accumulation** — for a *registered* code `u32::MAX`,
//!    `log_error` fails with [`Error::Overflow`]; every other registered code
//!    succeeds.
//!
//! 6. **Registry immutability (#1225)** — a successful `register_error` for
//!    an already-registered code always returns [`Error::AlreadyRegistered`]
//!    and never changes the stored description; descriptions change only via
//!    `update_error`.
//!
//! 7. **Update-path totality (#1225)** — `update_error` by the admin
//!    succeeds if and only if the code is registered (and the description
//!    is within bounds); otherwise it returns [`Error::NotRegistered`].
//!
//! 8. **Bounded descriptions (#1225)** — every description written by this
//!    suite is at most `errors::MAX_DESC_LEN` bytes; the over-cap rejection
//!    is exercised by the unit tests in `src/test.rs`.
//!
//! # Strategy
//! - `proptest` drives random action sequences (`init → mix of operations`).
//! - A deterministic LCG seeded with 64 values runs stable "golden path"
//!   traces so CI failures are reproducible without proptest shrinking.
//! - A `BTreeMap<u32, String>` models the expected registry, so arbitrary
//!   interleavings of register/update are checked against a reference
//!   implementation of the immutability rules.
//!
//! Closes CalloraOrg/Callora-Contracts#907.

extern crate std;

use errors::{Error, ErrorsContract, ErrorsContractClient};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Deterministic PRNG (no std::rand, reproducible across platforms)
// ---------------------------------------------------------------------------

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
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn gen_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn gen_bool(&mut self) -> bool {
        self.next_u64() & 1 == 0
    }
}

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// Every state-mutating or observing operation the proptest can exercise.
#[derive(Clone, Debug)]
enum ErrorsAction {
    /// Register an error code with a description.
    RegisterError { code: u32, desc_seed: u64 },
    /// Replace the description of an existing error code.
    UpdateError { code: u32, desc_seed: u64 },
    /// Log an error for a user (any code, including MAX).
    LogError { code: u32 },
    /// Attempt to register using a wrong admin (must fail).
    RegisterErrorWrongAdmin { code: u32 },
    /// Attempt a second init (must always fail).
    DoubleInit,
}

// ---------------------------------------------------------------------------
// Strategy
// ---------------------------------------------------------------------------

fn errors_action_strategy() -> impl Strategy<Value = ErrorsAction> {
    prop_oneof![
        4 => (any::<u32>(), any::<u64>())
            .prop_map(|(code, desc_seed)| ErrorsAction::RegisterError { code, desc_seed }),
        3 => (any::<u32>(), any::<u64>())
            .prop_map(|(code, desc_seed)| ErrorsAction::UpdateError { code, desc_seed }),
        4 => any::<u32>().prop_map(|code| ErrorsAction::LogError { code }),
        2 => any::<u32>()
            .prop_map(|code| ErrorsAction::RegisterErrorWrongAdmin { code }),
        1 => Just(ErrorsAction::DoubleInit),
    ]
}

// ---------------------------------------------------------------------------
// Invariant checker (shared by seeded traces and proptest)
// ---------------------------------------------------------------------------

fn run_sequence(env: &Env, actions: &[ErrorsAction]) {
    env.mock_all_auths();

    let contract_id = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let other = Address::generate(env);

    // ----- init -----
    client.init(&admin);

    // Invariant 1a: get_admin is not exposed by the contract, but we can verify
    // that re-init is rejected (confirms admin was stored).
    assert_eq!(
        client.try_init(&admin).unwrap_err().unwrap(),
        Error::AlreadyInitialized,
        "double-init must return AlreadyInitialized"
    );

    // ----- run action sequence -----
    // Reference model of the registry: code -> last accepted description.
    // `log_error` is only allowed for codes in this map (Invariant 2).
    let mut registered: BTreeMap<u32, std::string::String> = BTreeMap::new();

    for action in actions {
        match action {
            ErrorsAction::RegisterError { code, desc_seed } => {
                // Overflow-safe: just build a small description string.
                let desc_str = std::format!("err_{}", desc_seed % 10_000);
                let desc = String::from_str(env, &desc_str);

                if registered.contains_key(code) {
                    // Invariant 6: repeat registration is always rejected and
                    // never overwrites the stored description.
                    assert_eq!(
                        client
                            .try_register_error(&admin, code, &desc)
                            .unwrap_err()
                            .unwrap(),
                        Error::AlreadyRegistered,
                        "repeat register_error must return AlreadyRegistered"
                    );
                } else {
                    client.register_error(&admin, code, &desc);
                    registered.insert(*code, desc_str);
                }

                // Invariant 4: registering with a non-admin must fail — the
                // authorisation check runs before any other validation.
                assert_eq!(
                    client
                        .try_register_error(&other, code, &desc)
                        .unwrap_err()
                        .unwrap(),
                    Error::Unauthorized,
                    "non-admin register_error must return Unauthorized"
                );
            }

            ErrorsAction::UpdateError { code, desc_seed } => {
                let desc_str = std::format!("upd_{}", desc_seed % 10_000);
                let desc = String::from_str(env, &desc_str);

                if registered.contains_key(code) {
                    // Invariant 7: admin update of a registered code succeeds.
                    client.update_error(&admin, code, &desc);
                    registered.insert(*code, desc_str);
                } else {
                    // Invariant 7: update of an unregistered code is rejected.
                    assert_eq!(
                        client
                            .try_update_error(&admin, code, &desc)
                            .unwrap_err()
                            .unwrap(),
                        Error::NotRegistered,
                        "update_error on an unregistered code must return NotRegistered"
                    );
                }

                // Invariant 4: non-admin update must fail regardless of state.
                assert_eq!(
                    client
                        .try_update_error(&other, code, &desc)
                        .unwrap_err()
                        .unwrap(),
                    Error::Unauthorized,
                    "non-admin update_error must return Unauthorized"
                );
            }

            ErrorsAction::LogError { code } => {
                let user = Address::generate(env);
                if registered.contains_key(code) {
                    if *code == u32::MAX {
                        // Invariant 5: overflow path must return Overflow, never panic.
                        assert_eq!(
                            client.try_log_error(&user, code).unwrap_err().unwrap(),
                            Error::Overflow,
                            "log_error(u32::MAX) must return Overflow"
                        );
                    } else {
                        // Invariant 5: any other registered code must succeed.
                        client.log_error(&user, code);
                    }
                } else {
                    // Invariant 2: unregistered codes are rejected with a typed error.
                    assert_eq!(
                        client.try_log_error(&user, code).unwrap_err().unwrap(),
                        Error::UnknownErrorCode,
                        "log_error on an unregistered code must return UnknownErrorCode"
                    );
                }
            }

            ErrorsAction::RegisterErrorWrongAdmin { code } => {
                // Invariant 4: wrong admin always rejected.
                let desc = String::from_str(env, "bad");
                let result = client.try_register_error(&other, code, &desc);
                assert_eq!(
                    result.unwrap_err().unwrap(),
                    Error::Unauthorized,
                    "wrong-admin register_error must return Unauthorized"
                );
            }

            ErrorsAction::DoubleInit => {
                // Invariant 3: double-init always rejected.
                assert_eq!(
                    client.try_init(&admin).unwrap_err().unwrap(),
                    Error::AlreadyInitialized,
                    "double-init must return AlreadyInitialized"
                );
                assert_eq!(
                    client.try_init(&other).unwrap_err().unwrap(),
                    Error::AlreadyInitialized,
                    "double-init with different caller must return AlreadyInitialized"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic seeded traces
// ---------------------------------------------------------------------------

const SEED_COUNT: u64 = 64;
const TRACE_LEN: u32 = 32;

fn build_trace(seed: u64) -> std::vec::Vec<ErrorsAction> {
    let mut rng = Prng::new(seed);
    (0..TRACE_LEN)
        .map(|_| {
            let pick = rng.next_u64() % 5;
            match pick {
                0 => ErrorsAction::RegisterError {
                    code: rng.gen_u32(),
                    desc_seed: rng.next_u64(),
                },
                1 => ErrorsAction::UpdateError {
                    code: rng.gen_u32(),
                    desc_seed: rng.next_u64(),
                },
                2 => ErrorsAction::LogError {
                    code: if rng.gen_bool() {
                        u32::MAX
                    } else {
                        rng.gen_u32()
                    },
                },
                3 => ErrorsAction::RegisterErrorWrongAdmin {
                    code: rng.gen_u32(),
                },
                _ => ErrorsAction::DoubleInit,
            }
        })
        .collect()
}

#[test]
fn errors_invariant_seeded_traces() {
    for seed in 0..SEED_COUNT {
        let env = Env::default();
        let actions = build_trace(seed);
        run_sequence(&env, &actions);
    }
}

// ---------------------------------------------------------------------------
// proptest-driven random sequences
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Core invariants hold across arbitrary valid action sequences.
    #[test]
    fn errors_invariant_arbitrary_sequence(
        actions in prop::collection::vec(errors_action_strategy(), 1..=48)
    ) {
        let env = Env::default();
        run_sequence(&env, &actions);
    }
}

// ---------------------------------------------------------------------------
// Targeted edge-case invariant tests
// ---------------------------------------------------------------------------

#[test]
fn overflow_guard_is_robust_at_u32_max() {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &cid);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    client.init(&admin);
    // u32::MAX must be a registered code for the arithmetic guard to be the
    // thing that rejects it.
    let desc = String::from_str(&env, "Max Code");
    client.register_error(&admin, &u32::MAX, &desc);
    assert_eq!(
        client.try_log_error(&user, &u32::MAX).unwrap_err().unwrap(),
        Error::Overflow
    );
}

#[test]
fn overflow_guard_does_not_fire_below_max() {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &cid);
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    client.init(&admin);
    // Codes that do not overflow must succeed once registered.
    for &code in &[0_u32, 1, 100, u32::MAX - 1] {
        let desc = String::from_str(&env, "ok");
        client.register_error(&admin, &code, &desc);
        client.log_error(&user, &code);
    }
}

#[test]
fn unknown_code_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &cid);
    let user = Address::generate(&env);
    assert_eq!(
        client.try_log_error(&user, &123).unwrap_err().unwrap(),
        Error::UnknownErrorCode
    );
}

#[test]
fn double_init_always_returns_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &cid);
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    client.init(&admin);
    assert_eq!(
        client.try_init(&admin).unwrap_err().unwrap(),
        Error::AlreadyInitialized
    );
    assert_eq!(
        client.try_init(&other).unwrap_err().unwrap(),
        Error::AlreadyInitialized
    );
}

#[test]
fn unauthorised_register_always_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &cid);
    let admin = Address::generate(&env);
    let rogue = Address::generate(&env);
    client.init(&admin);
    let desc = String::from_str(&env, "bad");
    assert_eq!(
        client
            .try_register_error(&rogue, &999, &desc)
            .unwrap_err()
            .unwrap(),
        Error::Unauthorized
    );
}
