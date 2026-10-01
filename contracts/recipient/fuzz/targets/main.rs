//! Fuzz target: comprehensive recipient-registry state-machine fuzzer.
//!
//! Exercises the full public surface of [`CalloraRecipient`] by parsing a raw
//! byte stream into a sequence of typed operations and verifying that key
//! invariants hold after every call.
//!
//! # Invariants checked
//!
//! 1. **Admin invariance** — `get_admin()` never changes after initialization.
//! 2. **Count conservation** — `get_recipient_count()` equals the number of
//!    currently registered recipients; it increases by exactly 1 on every
//!    successful `register_recipient` and decreases by exactly 1 on every
//!    successful `remove_recipient`.
//! 3. **Existence consistency** — after a successful `register_recipient`,
//!    `has_recipient(name)` returns `true` and `get_recipient(name)` returns
//!    the registered address. After a successful `remove_recipient`,
//!    `has_recipient(name)` returns `false`.
//! 4. **Auth gate** — every state-changing entrypoint must return an error
//!    when called without authentication (i.e. after `env.set_auths(&[])`).
//! 5. **No uncontrolled panics** — the contract must never abort the process
//!    with a panic that is not caught through the normal Soroban error path or
//!    `std::panic::catch_unwind`.
//!
//! # Wire format
//!
//! The fuzzer byte stream is sliced into **3-byte operation tokens**:
//!
//! ```text
//! byte 0: operation discriminant (mod NUM_OPS)
//! bytes 1-2: big-endian u16 operand (used as name seed / address seed)
//! ```
//!
//! # Running
//!
//! ```bash
//! cargo fuzz run main
//! ```

#![no_main]

extern crate std;

use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, InvokeError};

use callora_recipient::{CalloraRecipient, CalloraRecipientClient};

// ---------------------------------------------------------------------------
// Configuration constants
// ---------------------------------------------------------------------------

/// Bytes consumed per operation token.
const BYTES_PER_OP: usize = 3;

/// Total number of distinct operation discriminants.
const NUM_OPS: u8 = 7;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register the recipient contract and return its address alongside a typed
/// client.
fn create_recipient(env: &Env) -> (Address, CalloraRecipientClient<'_>) {
    let addr = env.register(CalloraRecipient, ());
    (addr.clone(), CalloraRecipientClient::new(env, &addr))
}

/// Returns `true` when a `catch_unwind`-wrapped Soroban `try_*` call
/// completed without a panic **and** the contract returned `Ok(())`.
///
/// Soroban SDK v22 `try_*` methods return
/// `Result<Result<(), ConversionError>, Result<ContractError, InvokeError>>`.
/// Wrapped in `catch_unwind` the outermost layer adds
/// `Result<..., Box<dyn Any + Send>>`.
///
/// A logical success is `Ok(Ok(Ok(())))`.
fn is_try_success(
    result: &std::result::Result<
        std::result::Result<
            Result<(), soroban_sdk::ConversionError>,
            Result<callora_recipient::RecipientError, InvokeError>,
        >,
        Box<dyn std::any::Any + Send>,
    >,
) -> bool {
    matches!(result, Ok(Ok(Ok(()))))
}

/// Derive a deterministic recipient name from a u16 seed.
///
/// Names are 1–8 bytes, well within the max name length.
fn seed_name(env: &Env, seed: u16) -> soroban_sdk::String {
    let bytes = seed.to_be_bytes();
    let len = ((seed % 7) + 1) as usize; // 1–8 chars
    let slice = &bytes[..len.min(bytes.len())];
    soroban_sdk::String::from_bytes(env, slice)
}

/// Derive a unique Address from a seed.
fn seed_address(env: &Env, _seed: u16) -> Address {
    Address::generate(env)
}

// ---------------------------------------------------------------------------
// Fuzz entry-point
// ---------------------------------------------------------------------------

fuzz_target!(|data: &[u8]| {
    // Need at least one operation token.
    if data.len() < BYTES_PER_OP {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();

    // --- Static participants ------------------------------------------------
    let admin = Address::generate(&env);
    let outsider = Address::generate(&env);

    // --- Contract -----------------------------------------------------------
    let (_contract_addr, client) = create_recipient(&env);

    // Initialize.
    client.init(&admin);

    // --- Tracked state ------------------------------------------------------
    // Maps operand (name seed) → registered Address. We use the raw u16 operand
    // as the key since SorobanString does not implement Hash.
    let mut registered: std::collections::HashMap<u16, Address> = std::collections::HashMap::new();

    // -----------------------------------------------------------------------
    // Main operation loop
    // -----------------------------------------------------------------------
    for chunk in data.chunks(BYTES_PER_OP) {
        if chunk.len() < BYTES_PER_OP {
            break;
        }

        let op = chunk[0] % NUM_OPS;
        let operand = u16::from_be_bytes([chunk[1], chunk[2]]);

        match op {
            // ----------------------------------------------------------------
            // 0 — register_recipient(admin, name, address)
            // ----------------------------------------------------------------
            0 => {
                let rname = seed_name(&env, operand);
                let raddr = seed_address(&env, operand);
                let count_before = client.get_recipient_count();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_register_recipient(&admin, &rname, &raddr)
                }));

                if is_try_success(&result) {
                    // Success: recipient must now exist.
                    assert!(
                        client.has_recipient(&rname),
                        "register succeeded but has_recipient is false"
                    );
                    let record = client.get_recipient(&rname);
                    assert_eq!(record.address, raddr, "registered address mismatch");
                    assert_eq!(
                        client.get_recipient_count(),
                        count_before + 1,
                        "count did not increase by 1 after register"
                    );
                    registered.insert(operand, raddr);
                } else {
                    // Rejected (likely AlreadyRegistered): count unchanged.
                    assert_eq!(
                        client.get_recipient_count(),
                        count_before,
                        "rejected register mutated count"
                    );
                }
            }

            // ----------------------------------------------------------------
            // 1 — update_recipient(admin, name, new_address)
            // ----------------------------------------------------------------
            1 => {
                let rname = seed_name(&env, operand);
                let new_addr = seed_address(&env, operand.wrapping_add(0xA0A0));

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_update_recipient(&admin, &rname, &new_addr)
                }));

                if is_try_success(&result) {
                    assert!(
                        client.has_recipient(&rname),
                        "update succeeded but has_recipient is false"
                    );
                    let record = client.get_recipient(&rname);
                    assert_eq!(record.address, new_addr, "updated address mismatch");
                    registered.insert(operand, new_addr);
                }
                // If rejected (NotFound), tracked state is unchanged.
            }

            // ----------------------------------------------------------------
            // 2 — remove_recipient(admin, name)
            // ----------------------------------------------------------------
            2 => {
                let rname = seed_name(&env, operand);
                let count_before = client.get_recipient_count();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_remove_recipient(&admin, &rname)
                }));

                if is_try_success(&result) {
                    assert!(
                        !client.has_recipient(&rname),
                        "remove succeeded but has_recipient is still true"
                    );
                    assert_eq!(
                        client.get_recipient_count(),
                        count_before - 1,
                        "count did not decrease by 1 after remove"
                    );
                    registered.remove(&operand);
                } else {
                    // Rejected (NotFound): count unchanged.
                    assert_eq!(
                        client.get_recipient_count(),
                        count_before,
                        "rejected remove mutated count"
                    );
                }
            }

            // ----------------------------------------------------------------
            // 3 — view-only calls: get_recipient / has_recipient / get_admin /
            //     get_recipient_count — must not mutate state
            // ----------------------------------------------------------------
            3 => {
                let count_before = client.get_recipient_count();
                let rname = seed_name(&env, operand);

                let _ = client.get_admin();
                let _ = client.get_recipient_count();
                let _ = client.has_recipient(&rname);
                let _ = client.get_recipient(&rname);

                assert_eq!(
                    client.get_recipient_count(),
                    count_before,
                    "view calls mutated recipient count"
                );
            }

            // ----------------------------------------------------------------
            // 4 — unauthorised register_recipient attempt (auth gate)
            // ----------------------------------------------------------------
            4 => {
                let rname = seed_name(&env, operand);
                let raddr = seed_address(&env, operand);
                let count_before = client.get_recipient_count();

                env.set_auths(&[]);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_register_recipient(&outsider, &rname, &raddr)
                }));
                env.mock_all_auths();

                assert!(
                    !is_try_success(&result),
                    "unauthenticated register unexpectedly succeeded"
                );
                assert_eq!(
                    client.get_recipient_count(),
                    count_before,
                    "unauthenticated register mutated count"
                );
            }

            // ----------------------------------------------------------------
            // 5 — unauthorised update_recipient attempt (auth gate)
            // ----------------------------------------------------------------
            5 => {
                let rname = seed_name(&env, operand);
                let new_addr = seed_address(&env, operand);
                let count_before = client.get_recipient_count();
                let existed_before = client.has_recipient(&rname);

                env.set_auths(&[]);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_update_recipient(&outsider, &rname, &new_addr)
                }));
                env.mock_all_auths();

                assert!(
                    !is_try_success(&result),
                    "unauthenticated update unexpectedly succeeded"
                );
                assert_eq!(
                    client.get_recipient_count(),
                    count_before,
                    "unauthenticated update mutated count"
                );
                assert_eq!(
                    client.has_recipient(&rname),
                    existed_before,
                    "unauthenticated update changed recipient existence"
                );
            }

            // ----------------------------------------------------------------
            // 6 — unauthorised remove_recipient attempt (auth gate)
            // ----------------------------------------------------------------
            6 => {
                let rname = seed_name(&env, operand);
                let count_before = client.get_recipient_count();
                let existed_before = client.has_recipient(&rname);

                env.set_auths(&[]);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_remove_recipient(&outsider, &rname)
                }));
                env.mock_all_auths();

                assert!(
                    !is_try_success(&result),
                    "unauthenticated remove unexpectedly succeeded"
                );
                assert_eq!(
                    client.get_recipient_count(),
                    count_before,
                    "unauthenticated remove mutated count"
                );
                assert_eq!(
                    client.has_recipient(&rname),
                    existed_before,
                    "unauthenticated remove changed recipient existence"
                );
            }

            _ => unreachable!("op discriminant outside NUM_OPS range"),
        }

        // -------------------------------------------------------------------
        // Post-step global invariants
        // -------------------------------------------------------------------

        // I1: admin never changes.
        assert_eq!(
            client.get_admin(),
            admin,
            "invariant I1: admin changed after op {op}"
        );

        // I2: count consistency with tracked state.
        let actual_count = client.get_recipient_count();
        assert_eq!(
            actual_count as usize,
            registered.len(),
            "invariant I2: count ({actual_count}) != tracked len ({}) after op {op}",
            registered.len()
        );

        // I3: every tracked name must exist in the contract.
        for &operand_key in registered.keys() {
            let rname = seed_name(&env, operand_key);
            assert!(
                client.has_recipient(&rname),
                "invariant I3: tracked name not found in contract after op {op}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Terminal invariant: tracked count matches contract count.
    // -----------------------------------------------------------------------
    let final_count = client.get_recipient_count();
    assert_eq!(
        final_count as usize,
        registered.len(),
        "terminal: count={} tracked={}",
        final_count,
        registered.len()
    );
});
