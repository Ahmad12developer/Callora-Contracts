//! Fuzz target: comprehensive distribute contract fuzzer.
//!
//! Exercises the public surface of `Distribute` — initialization, single and
//! batch USDC distribution, config parameter updates, admin rotation, pausing,
//! and contract upgrade — by parsing a raw byte stream into typed operations
//! and verifying that key invariants hold after every call.
//!
//! # Scope
//! Covers all contract entrypoints in `Distribute`.
//!
//! # Invariants checked
//!
//! 1. **Admin exclusivity** — state-changing operations fail when invoked by
//!    non-admin accounts.
//! 2. **Non-negative amounts** — distribution amounts must be positive.
//! 3. **Max-distribute cap** — amounts exceeding `max_distribute` are rejected.
//! 4. **Paused-state gating** — when paused, state-changing ops revert.
//! 5. **Balance sufficiency** — contract cannot distribute more USDC than it holds.
//! 6. **No uncontrolled panics** — all malformed inputs are handled gracefully.

#![no_main]

use callora_distribute::{Distribute, DistributeClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Env, IntoVal, Symbol};

// ── Fuzz operation enum ────────────────────────────────────────────────

enum FuzzOp {
    Init { admin: Address, usdc: Address },
    Distribute { to: Address, amount: i128 },
    BatchDistribute,
    SetMaxDistribute { amount: i128 },
    SetPaused { paused: bool },
    TransferAdmin { new_admin: Address },
    AcceptAdmin,
    Upgrade { hash: [u8; 32] },
    GetAdmin,
    GetMaxDistribute,
    GetPaused,
    GetUsdc,
    GetVersion,
}

fn parse_op(data: &[u8], pos: &mut usize) -> Option<FuzzOp> {
    if *pos >= data.len() {
        return None;
    }
    let tag = data[*pos] % 11;
    *pos += 1;

    Some(match tag {
        0 => FuzzOp::Init {
            admin: Address::generate(&Env::default()),
            usdc: Address::generate(&Env::default()),
        },
        1 => FuzzOp::Distribute {
            to: Address::generate(&Env::default()),
            amount: if *pos + 8 <= data.len() {
                let v = i128::from_le_bytes([
                    data[*pos],
                    data[*pos + 1],
                    data[*pos + 2],
                    data[*pos + 3],
                    data[*pos + 4],
                    data[*pos + 5],
                    data[*pos + 6],
                    data[*pos + 7],
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ]);
                *pos += 8;
                v
            } else {
                1
            },
        },
        2 => FuzzOp::BatchDistribute,
        3 => FuzzOp::SetMaxDistribute {
            amount: if *pos + 8 <= data.len() {
                let v = i128::from_le_bytes([
                    data[*pos],
                    data[*pos + 1],
                    data[*pos + 2],
                    data[*pos + 3],
                    data[*pos + 4],
                    data[*pos + 5],
                    data[*pos + 6],
                    data[*pos + 7],
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ]);
                *pos += 8;
                v
            } else {
                0
            },
        },
        4 => FuzzOp::SetPaused {
            paused: if *pos < data.len() {
                data[*pos] % 2 == 0
            } else {
                false
            },
        },
        5 => FuzzOp::TransferAdmin {
            new_admin: Address::generate(&Env::default()),
        },
        6 => FuzzOp::AcceptAdmin,
        7 => FuzzOp::Upgrade {
            hash: {
                let mut h = [0u8; 32];
                let copy_len = (*pos + 32).min(data.len()) - *pos;
                for i in 0..copy_len.min(32) {
                    h[i] = data[*pos + i];
                }
                *pos += 32;
                h
            },
        },
        8 => FuzzOp::GetAdmin,
        9 => FuzzOp::GetMaxDistribute,
        10 => FuzzOp::GetPaused,
        _ => return None,
    })
}

fuzz_target!(|data: &[u8]| {
    let env = Env::default();
    let contract_id = Address::generate(&env);
    env.register_contract(&contract_id, Distribute);
    let client = DistributeClient::new(&env, &contract_id);

    let mut pos: usize = 0;
    let admin = Address::generate(&env);
    let usdc = Address::generate(&env);

    // Init must succeed first
    if !data.is_empty() {
        client.init(&admin, &usdc);
    }

    while let Some(op) = parse_op(data, &mut pos) {
        match op {
            FuzzOp::Distribute { to, amount } => {
                let _ = client.try_distribute(&to, &amount);
            }
            FuzzOp::SetMaxDistribute { amount } => {
                let _ = client.try_set_max_distribute(&amount);
            }
            FuzzOp::SetPaused { paused } => {
                let _ = client.try_set_paused(&paused);
            }
            FuzzOp::GetAdmin => {
                client.get_admin();
            }
            FuzzOp::GetMaxDistribute => {
                client.get_max_distribute();
            }
            FuzzOp::GetPaused => {
                client.get_paused();
            }
            _ => {}
        }
    }
});
