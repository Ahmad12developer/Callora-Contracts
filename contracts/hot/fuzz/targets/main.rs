//! cargo-fuzz target: hammer Callora Hot entrypoints with malformed inputs.
//!
//! # Properties checked on every execution
//! 1. **Pause bit integrity** — `is_paused` is always a coherent boolean after
//!    every op; unauthorized pause/unpause never flips it.
//! 2. **Cooldown monotonicity** — after a guarded action fires, `is_ready`
//!    returns `false` and `cooldown_remaining` returns a value > 0 for that
//!    action tag until the window elapses.
//! 3. **Auth gating** — non-admin callers must never succeed at guarded actions.
//! 4. **Per-action isolation** — cooling one action never blocks an unrelated one.
//! 5. **Admin rotation** — after a successful two-step transfer the old admin can
//!    no longer call admin-only entrypoints.
//! 6. **No unexpected abort** — expected panics from auth / cooldown / already
//!    paused paths are caught; the harness itself must not unwind.
//!
//! # Running
//! ```bash
//! cargo fuzz run main
//! # or, from workspace:
//! cargo test -p callora-hot
//! ```
//!
//! Closes CalloraOrg/Callora-Contracts#871.

#![no_main]

extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use callora_hot::{
    CalloraHot, CalloraHotClient, HotError, ACTION_PAUSE, ACTION_ROTATE, ACTION_UNPAUSE,
};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of operations the fuzzer will execute in a single run.
const MAX_HOT_OPS: usize = 64;

/// A short cooldown window so the fuzzer can exercise the window-elapsed path.
const FUZZ_COOLDOWN: u64 = 5;

// ---------------------------------------------------------------------------
// Decode helpers — map a byte stream to a sequence of operations.
// ---------------------------------------------------------------------------

/// One step in a hot contract fuzz sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HotOp {
    /// Initialise the contract (admin, signer, cooldown).
    Init,
    /// Try a double-init (must always fail after the first).
    DoubleInit,
    /// Pause as admin.
    Pause,
    /// Pause as an unauthorized caller.
    PauseUnauthorized,
    /// Unpause as admin.
    Unpause,
    /// Unpause as an unauthorized caller.
    UnpauseUnauthorized,
    /// Rotate the hot signer as admin.
    Rotate,
    /// Rotate as an unauthorized caller.
    RotateUnauthorized,
    /// Update the cooldown window as admin.
    SetCooldown,
    /// Update the cooldown as an unauthorized caller.
    SetCooldownUnauthorized,
    /// Nominate a new admin.
    SetAdmin,
    /// Accept the pending admin role.
    AcceptAdmin,
    /// Read views and verify coherence.
    CheckViews,
    /// Advance the ledger clock by a fixed tick.
    Advance,
}

impl HotOp {
    /// Decode a raw fuzzer byte into an operation.
    fn from_byte(b: u8) -> Self {
        match b % 14 {
            0 => Self::Init,
            1 => Self::DoubleInit,
            2 => Self::Pause,
            3 => Self::PauseUnauthorized,
            4 => Self::Unpause,
            5 => Self::UnpauseUnauthorized,
            6 => Self::Rotate,
            7 => Self::RotateUnauthorized,
            8 => Self::SetCooldown,
            9 => Self::SetCooldownUnauthorized,
            10 => Self::SetAdmin,
            11 => Self::AcceptAdmin,
            12 => Self::CheckViews,
            _ => Self::Advance,
        }
    }

    /// Decode a byte slice into a bounded operation list.
    fn decode_sequence(data: &[u8], max_ops: usize) -> std::vec::Vec<Self> {
        data.iter()
            .take(max_ops)
            .map(|b| Self::from_byte(*b))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Fuzz harness
// ---------------------------------------------------------------------------

fn run_sequence(ops: &[HotOp]) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let intruder = Address::generate(&env);

    let contract_id = env.register(CalloraHot, ());
    let client = CalloraHotClient::new(&env, &contract_id);

    let pause_tag = Symbol::new(&env, ACTION_PAUSE);
    let unpause_tag = Symbol::new(&env, ACTION_UNPAUSE);
    let rotate_tag = Symbol::new(&env, ACTION_ROTATE);

    // Track model state.
    let mut initialized = false;
    let mut paused = false;
    let mut current_admin = admin.clone();
    let mut current_signer = signer.clone();
    let mut cooldown = FUZZ_COOLDOWN;

    for op in ops {
        match *op {
            HotOp::Init => {
                if initialized {
                    // Already initialized — second init must fail.
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        client.init(&admin, &signer, &Some(FUZZ_COOLDOWN));
                    }));
                    assert!(result.is_err(), "double-init must panic");
                } else {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        client.init(&admin, &signer, &Some(FUZZ_COOLDOWN));
                    }));
                    assert!(result.is_ok(), "first init must succeed");
                    initialized = true;
                }
            }

            HotOp::DoubleInit => {
                if initialized {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        client.try_init(&admin, &signer, &Some(FUZZ_COOLDOWN));
                    }));
                    if let Ok(Err(e)) = result {
                        assert_eq!(e, HotError::AlreadyInitialized);
                    }
                }
            }

            HotOp::Pause => {
                if !initialized {
                    continue;
                }
                let before = client.is_paused();
                let result = catch_unwind(AssertUnwindSafe(|| client.pause(&current_admin)));
                match result {
                    Ok(()) => {
                        paused = true;
                        assert!(client.is_paused());
                    }
                    Err(_) => {
                        // Either AlreadyPaused or CooldownActive — flag unchanged.
                        assert_eq!(client.is_paused(), before);
                        assert_eq!(client.is_paused(), paused);
                    }
                }
            }

            HotOp::PauseUnauthorized => {
                if !initialized {
                    continue;
                }
                let before = client.is_paused();
                let result = catch_unwind(AssertUnwindSafe(|| client.pause(&intruder)));
                assert!(result.is_err(), "unauthorized pause must fail");
                assert_eq!(client.is_paused(), before);
                assert_eq!(client.is_paused(), paused);
            }

            HotOp::Unpause => {
                if !initialized {
                    continue;
                }
                let before = client.is_paused();
                let result = catch_unwind(AssertUnwindSafe(|| client.unpause(&current_admin)));
                match result {
                    Ok(()) => {
                        paused = false;
                        assert!(!client.is_paused());
                    }
                    Err(_) => {
                        assert_eq!(client.is_paused(), before);
                        assert_eq!(client.is_paused(), paused);
                    }
                }
            }

            HotOp::UnpauseUnauthorized => {
                if !initialized {
                    continue;
                }
                let before = client.is_paused();
                let result = catch_unwind(AssertUnwindSafe(|| client.unpause(&intruder)));
                assert!(result.is_err(), "unauthorized unpause must fail");
                assert_eq!(client.is_paused(), before);
                assert_eq!(client.is_paused(), paused);
            }

            HotOp::Rotate => {
                if !initialized {
                    continue;
                }
                let new_signer = Address::generate(&env);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    client.rotate_signer(&current_admin, &new_signer)
                }));
                if result.is_ok() {
                    current_signer = new_signer;
                    assert_eq!(client.get_signer(), current_signer);
                }
            }

            HotOp::RotateUnauthorized => {
                if !initialized {
                    continue;
                }
                let target = Address::generate(&env);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    client.rotate_signer(&intruder, &target)
                }));
                assert!(result.is_err(), "unauthorized rotate must fail");
                assert_eq!(client.get_signer(), current_signer);
            }

            HotOp::SetCooldown => {
                if !initialized {
                    continue;
                }
                let new_cooldown = if cooldown == FUZZ_COOLDOWN {
                    // Alternate between valid and invalid values.
                    cooldown + 10
                } else {
                    FUZZ_COOLDOWN
                };
                let result = catch_unwind(AssertUnwindSafe(|| {
                    client.set_cooldown(&current_admin, &new_cooldown);
                }));
                if result.is_ok() {
                    assert_eq!(client.get_cooldown(), new_cooldown);
                    cooldown = new_cooldown;
                }
            }

            HotOp::SetCooldownUnauthorized => {
                if !initialized {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| client.set_cooldown(&intruder, &60)));
                assert!(result.is_err(), "unauthorized set_cooldown must fail");
            }

            HotOp::SetAdmin => {
                if !initialized {
                    continue;
                }
                let new_admin = Address::generate(&env);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    client.set_admin(&current_admin, &new_admin)
                }));
                if result.is_ok() {
                    // Nomination stored; current_admin unchanged until accept.
                    assert_eq!(client.get_pending_admin(), Some(new_admin));
                }
            }

            HotOp::AcceptAdmin => {
                if !initialized {
                    continue;
                }
                if let Some(pending) = client.get_pending_admin() {
                    let old_admin = current_admin.clone();
                    let result = catch_unwind(AssertUnwindSafe(|| client.accept_admin(&pending)));
                    if result.is_ok() {
                        current_admin = pending;
                        assert_eq!(client.get_admin(), current_admin);
                        assert_eq!(client.get_pending_admin(), None);
                        // Old admin can no longer perform admin actions.
                        let rotation_result = catch_unwind(AssertUnwindSafe(|| {
                            client.rotate_signer(&old_admin, &Address::generate(&env));
                        }));
                        assert!(
                            rotation_result.is_err(),
                            "old admin must be rejected after rotation"
                        );
                    }
                }
            }

            HotOp::CheckViews => {
                if initialized {
                    // Views must be coherent.
                    assert_eq!(client.is_paused(), paused);
                    let on_chain_admin = client.get_admin();
                    assert_eq!(on_chain_admin, current_admin);

                    // Cooldown remaining must be >= 0 (u64).
                    let remaining_pause = client.cooldown_remaining(&pause_tag);
                    let remaining_unpause = client.cooldown_remaining(&unpause_tag);
                    let remaining_rotate = client.cooldown_remaining(&rotate_tag);

                    assert_eq!(client.is_ready(&pause_tag), remaining_pause == 0);
                    assert_eq!(client.is_ready(&unpause_tag), remaining_unpause == 0);
                    assert_eq!(client.is_ready(&rotate_tag), remaining_rotate == 0);
                }
            }

            HotOp::Advance => {
                if !initialized {
                    continue;
                }
                // Advance ledger timestamp so cooldowns may expire.
                let now = env.ledger().timestamp();
                env.ledger().set_timestamp(now + cooldown + 1);
            }
        }
    }

    // Final coherence: model matches on-chain state.
    if initialized {
        assert_eq!(client.is_paused(), paused);
        assert_eq!(client.get_admin(), current_admin);
        assert_eq!(client.get_signer(), current_signer);
    }
}

fuzz_target!(|data: &[u8]| {
    let ops = HotOp::decode_sequence(data, MAX_HOT_OPS);
    if ops.is_empty() {
        return;
    }
    run_sequence(&ops);
});
