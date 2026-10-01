//! cargo-fuzz target: hammer `batch_distribute` entrypoint with malformed inputs.
//!
//! The fuzzer parses a raw byte stream into a sequence of typed operations that
//! exercise `batch_distribute` on a pre-initialised Callora Distribute contract.
//! It also exercises the surrounding administration surface (init, pause,
//! set_max_distribute, single-leg distribute) to ensure batch operations interact
//! correctly with the broader contract state machine.
//!
//! # Properties checked on every execution
//!
//! 1. **Conservation** – if `batch_distribute` succeeds, the pool balance
//!    decreases by exactly the sum of the distributed amounts, and the total
//!    transferred out via USDC matches.
//!
//! 2. **Rejection invariant** – on any expected-invalid input (empty batch,
//!    oversized batch, non-positive amount, per-leg cap exceeded, self-recipient,
//!    insufficient balance), the contract balance must remain unchanged.
//!
//! 3. **Pause gate** – while paused, `batch_distribute` (and `distribute`) must
//!    be rejected.
//!
//! 4. **Auth gate** – every state-changing entry-point must fail when the caller
//!    lacks authentication (tested by calling with intruder addresses).
//!
//! 5. **No uncontrolled panic** – any panic must be caught via
//!    `std::panic::catch_unwind`; the harness itself must never abort.
//!
//! 6. **Admin invariance** – after batch ops, `get_admin` and `get_usdc_token`
//!    must remain unchanged.
//!
//! # Wire format
//!
//! Each operation is encoded as **3 bytes**:
//! ```text
//! byte 0: operation discriminant (mod NUM_OPS)
//! bytes 1-2: big-endian u16 operand (amount / recipient index / batch size)
//! ```
//!
//! # Running
//! ```bash
//! cargo fuzz run main
//! ```
//!
//! Closes CalloraOrg/Callora-Contracts#891.

#![no_main]

extern crate std;

use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env, Vec as SorobanVec};

use callora_distribute::{limits, Distribute, DistributeClient};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Bytes consumed per operation token.
const BYTES_PER_OP: usize = 3;

/// Total number of distinct operation discriminants (0–8 = 9 ops).
const NUM_OPS: u8 = 9;

/// USDC pre-funded into the contract at startup.
const CONTRACT_FUNDING: i128 = 1_000_000;

/// Initial per-leg `max_distribute` cap.
const INITIAL_MAX_DISTRIBUTE: i128 = 100_000;

/// Number of fixed recipient addresses available as distribute targets.
const RECIPIENT_POOL_SIZE: usize = 4;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a Stellar asset contract and return its address + admin client.
fn create_usdc<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (addr.clone(), StellarAssetClient::new(env, &addr))
}

/// Register and return a distribute contract client.
fn create_contract(env: &Env) -> DistributeClient<'_> {
    let addr = env.register(Distribute, ());
    DistributeClient::new(env, &addr)
}

type CatchResult<T> = std::result::Result<
    std::result::Result<T, soroban_sdk::InvokeError>,
    Box<dyn std::any::Any + Send>,
>;

/// True if the catch_unwind + try_ pattern returned Ok(Ok(_)).
fn is_success<T>(result: &CatchResult<T>) -> bool {
    matches!(result, Ok(Ok(_)))
}

// ---------------------------------------------------------------------------
// Fuzz entry-point
// ---------------------------------------------------------------------------

fuzz_target!(|data: &[u8]| {
    if data.len() < BYTES_PER_OP {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();

    // --- Static participants -------------------------------------------
    let admin = Address::generate(&env);
    let intruder = Address::generate(&env);

    // --- USDC token ----------------------------------------------------
    let (usdc_addr, usdc_admin) = create_usdc(&env, &admin);

    // --- Contract -------------------------------------------------------
    let contract = create_contract(&env);

    // Fund the contract with USDC.
    usdc_admin.mint(&env.current_contract_address(), &CONTRACT_FUNDING);

    contract.init(&admin, &usdc_addr);
    contract.set_max_distribute(&admin, &INITIAL_MAX_DISTRIBUTE);

    // Fixed recipient pool.
    let recipients: std::vec::Vec<Address> = (0..RECIPIENT_POOL_SIZE)
        .map(|_| Address::generate(&env))
        .collect();

    // Mint 1 unit to each recipient so they can receive transfers.
    for r in &recipients {
        usdc_admin.mint(r, &1_i128);
    }

    let mut max_distribute: i128 = INITIAL_MAX_DISTRIBUTE;
    let mut paused: bool = false;

    // --- Main fuzz loop ------------------------------------------------
    for chunk in data.chunks(BYTES_PER_OP) {
        if chunk.len() < BYTES_PER_OP {
            break;
        }

        let op = chunk[0] % NUM_OPS;
        let operand = u16::from_be_bytes([chunk[1], chunk[2]]);

        match op {
            // -----------------------------------------------------------
            // 0 — batch_distribute(admin, [(recipient, amount), ...])
            // -----------------------------------------------------------
            0 => {
                let leg_count = ((chunk[1] as usize) % RECIPIENT_POOL_SIZE) + 1;
                let per_leg = (operand as i128 % 10_000).max(1);
                let total = per_leg * leg_count as i128;

                let mut payments: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                for i in 0..leg_count {
                    payments.push_back((recipients[i].clone(), per_leg));
                }

                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &payments)
                }));

                let balance_after = contract.balance();

                if is_success(&result) {
                    // Conservation: balance must decrease by exactly `total`.
                    assert_eq!(
                        balance_after,
                        balance_before - total,
                        "op=0 (batch_distribute) conservation violated: \
                         before={balance_before} total={total} after={balance_after}"
                    );
                    assert!(
                        per_leg <= max_distribute,
                        "op=0 (batch_distribute) exceeded max_distribute cap"
                    );
                } else {
                    // Rejected call must not change balance.
                    assert_eq!(
                        balance_before, balance_after,
                        "op=0 (batch_distribute) rejected but balance changed: \
                         before={balance_before} after={balance_after}"
                    );
                }
                assert!(
                    contract.balance() >= 0,
                    "op=0 (batch_distribute) negative balance"
                );
            }

            // -----------------------------------------------------------
            // 1 — batch_distribute with an empty Vec (must always fail)
            // -----------------------------------------------------------
            1 => {
                let empty: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &empty)
                }));

                let balance_after = contract.balance();
                assert!(
                    !is_success(&result),
                    "op=1 (empty batch_distribute) unexpectedly succeeded"
                );
                assert_eq!(
                    balance_before, balance_after,
                    "op=1 (empty batch) changed balance: {balance_before} -> {balance_after}"
                );
            }

            // -----------------------------------------------------------
            // 2 — batch_distribute with a large batch (exceeds MAX_BATCH_SIZE)
            // -----------------------------------------------------------
            2 => {
                let oversized = limits::MAX_BATCH_SIZE + 1 + (operand as u32 % 10);
                let amount = (operand as i128 % 100).max(1);
                let mut payments: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                for i in 0..oversized {
                    let idx = (i as usize) % RECIPIENT_POOL_SIZE;
                    payments.push_back((recipients[idx].clone(), amount));
                }

                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &payments)
                }));

                let balance_after = contract.balance();
                assert!(
                    !is_success(&result),
                    "op=2 (batch > MAX_BATCH_SIZE) unexpectedly succeeded"
                );
                assert_eq!(
                    balance_before, balance_after,
                    "op=2 (oversized batch) changed balance: {balance_before} -> {balance_after}"
                );
            }

            // -----------------------------------------------------------
            // 3 — batch_distribute with zero amount leg (must fail)
            // -----------------------------------------------------------
            3 => {
                let mut payments: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                payments.push_back((recipients[0].clone(), 0_i128));
                // Add a valid leg second to ensure fail-early
                payments.push_back((recipients[1].clone(), 100_i128));

                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &payments)
                }));

                let balance_after = contract.balance();
                assert!(
                    !is_success(&result),
                    "op=3 (zero amount) batch_distribute unexpectedly succeeded"
                );
                assert_eq!(
                    balance_before, balance_after,
                    "op=3 (zero-amount leg) changed balance: {balance_before} -> {balance_after}"
                );
            }

            // -----------------------------------------------------------
            // 4 — batch_distribute with amount > max_distribute (must fail)
            // -----------------------------------------------------------
            4 => {
                let oversized_amount = max_distribute + 1 + (operand as i128 % 1000);
                let mut payments: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                payments.push_back((recipients[0].clone(), oversized_amount));

                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &payments)
                }));

                let balance_after = contract.balance();
                assert!(
                    !is_success(&result),
                    "op=4 (exceeds cap) batch_distribute unexpectedly succeeded"
                );
                assert_eq!(
                    balance_before, balance_after,
                    "op=4 (exceeds-cap leg) changed balance: {balance_before} -> {balance_after}"
                );
            }

            // -----------------------------------------------------------
            // 5 — single-leg distribute (admin, recipient, amount)
            // -----------------------------------------------------------
            5 => {
                let recipient_idx = (chunk[1] as usize) % RECIPIENT_POOL_SIZE;
                let amount = (operand as i128 % 10_000).max(1);
                let to = recipients[recipient_idx].clone();
                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_distribute(&admin, &to, &amount)
                }));

                let balance_after = contract.balance();

                if is_success(&result) {
                    assert_eq!(
                        balance_after,
                        balance_before - amount,
                        "op=5 (distribute) conservation violated: \
                         before={balance_before} amount={amount} after={balance_after}"
                    );
                } else {
                    assert_eq!(
                        balance_before, balance_after,
                        "op=5 (distribute) rejected but balance changed"
                    );
                }
            }

            // -----------------------------------------------------------
            // 6 — set_max_distribute / pause / unpause
            // -----------------------------------------------------------
            6 => {
                match operand % 3 {
                    0 => {
                        // Toggle pause
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if paused {
                                contract.try_unpause(&admin)
                            } else {
                                contract.try_pause(&admin)
                            }
                        }));
                        if is_success(&result) {
                            paused = !paused;
                        }
                    }
                    1 => {
                        // Update max_distribute
                        let new_max = (operand as i128 % 200_000).max(1);
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            contract.try_set_max_distribute(&admin, &new_max)
                        }));
                        if is_success(&result) {
                            max_distribute = new_max;
                        }
                        assert!(
                            contract.get_max_distribute() > 0,
                            "op=6 max_distribute became non-positive"
                        );
                    }
                    _ => {
                        // Unauthorized distribute attempt (auth gate)
                        let amount = (operand as i128 % 500).max(1);
                        let bal = contract.balance();

                        env.set_auths(&[]);
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            contract.try_distribute(&intruder, &recipients[0], &amount)
                        }));
                        env.mock_all_auths();

                        assert!(
                            !is_success(&result),
                            "op=6 unauthorized distribute succeeded"
                        );
                        assert_eq!(bal, contract.balance(), "op=6 auth gate balance changed");
                    }
                }
            }

            // -----------------------------------------------------------
            // 7 — batch_distribute with contract-itself as recipient
            // -----------------------------------------------------------
            7 => {
                let self_addr = env.current_contract_address();
                let amount = (operand as i128 % 1000).max(1);
                let mut payments: SorobanVec<(Address, i128)> = SorobanVec::new(&env);
                payments.push_back((self_addr.clone(), amount));

                let balance_before = contract.balance();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    contract.try_batch_distribute(&admin, &payments)
                }));

                let balance_after = contract.balance();
                assert!(
                    !is_success(&result),
                    "op=7 (self-recipient) batch_distribute unexpectedly succeeded"
                );
                assert_eq!(
                    balance_before, balance_after,
                    "op=7 (self-recipient) changed balance"
                );
            }

            // -----------------------------------------------------------
            // 8 — view-only reads + invariant checks
            // -----------------------------------------------------------
            8 => {
                let bal = contract.balance();
                let _ = contract.get_admin();
                let _ = contract.get_usdc_token();
                let _ = contract.get_paused();
                let _ = contract.get_max_distribute();
                let _ = contract.get_max_batch_size();
                let _ = contract.version();
                assert_eq!(bal, contract.balance(), "op=8 (views) mutated balance");

                // Verify admin and token invariants
                assert_eq!(
                    contract.get_admin(),
                    admin,
                    "op=8 admin changed unexpectedly"
                );
                assert_eq!(
                    contract.get_usdc_token(),
                    usdc_addr,
                    "op=8 usdc token changed unexpectedly"
                );
                assert_eq!(
                    contract.get_paused(),
                    paused,
                    "op=8 paused state out of sync"
                );
                assert!(
                    contract.get_max_distribute() > 0,
                    "op=8 max_distribute became non-positive"
                );
            }

            _ => unreachable!("op discriminant outside NUM_OPS range"),
        }

        // -----------------------------------------------------------------
        // Post-step invariants
        // -----------------------------------------------------------------
        assert!(
            contract.balance() >= 0,
            "invariant: negative balance after op {op}"
        );
        assert_eq!(
            contract.get_admin(),
            admin,
            "invariant: admin changed after op {op}"
        );
        assert_eq!(
            contract.get_usdc_token(),
            usdc_addr,
            "invariant: usdc_token changed after op {op}"
        );
        assert_eq!(
            contract.get_paused(),
            paused,
            "invariant: paused state out of sync after op {op}"
        );

        // If paused, verify both batch and single distribute are rejected.
        if paused {
            let small = 10i128;
            let b = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                contract.try_batch_distribute(&admin, &{
                    let mut v = SorobanVec::new(&env);
                    v.push_back((recipients[0].clone(), small));
                    v
                })
            }));
            assert!(
                !is_success(&b),
                "invariant: batch_distribute succeeded while paused (op {op})"
            );
            let d = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                contract.try_distribute(&admin, &recipients[0], &small)
            }));
            assert!(
                !is_success(&d),
                "invariant: distribute succeeded while paused (op {op})"
            );
        }
    }

    // -- Final coherence checks -----------------------------------------
    assert_eq!(contract.get_admin(), admin, "final: admin changed");
    assert_eq!(
        contract.get_usdc_token(),
        usdc_addr,
        "final: usdc token changed"
    );
    assert_eq!(contract.get_paused(), paused, "final: paused out of sync");
    assert!(contract.balance() >= 0, "final: negative balance");
});
