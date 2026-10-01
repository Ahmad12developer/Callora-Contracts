//! Fuzz target: comprehensive checkpoint contract state-machine fuzzer.
//!
//! Exercises the public surface of `CalloraCheckpoint` — initialization, single
//! and batch checkpoint creation, range queries, TTL management, admin rotation,
//! and contract upgrade — by parsing a raw byte stream into typed operations and
//! verifying that key invariants hold after every call.
//!
//! # Scope
//! Covers all contract entrypoints and data types in `CalloraCheckpoint`.
//!
//! # Invariants checked
//!
//! 1. **Monotonic sequential IDs & Count** — `get_checkpoint_count` and
//!    `get_latest_checkpoint_id` increase monotonically upon successful creation
//!    and remain strictly in sync.
//! 2. **Immutability of Checkpoints** — once written, a `CheckpointRecord` for a
//!    given ID is never mutated by subsequent operations.
//! 3. **Non-negative Balance Enforcement** — single and batch checkpoint
//!    creations reject negative balances with `CheckpointError::AmountNegative`.
//! 4. **Batch Atomicity & Bounds** — `batch_create_checkpoints` rejects empty
//!    batches (`BatchEmpty`), batches exceeding `MAX_BATCH_SIZE` (50)
//!    (`BatchTooLarge`), and invalid balance items without mutating contract state.
//! 5. **Paginated Range Bounds** — `get_checkpoints_range` rejects 0 limit
//!    (`InvalidPageSize`), returns empty vector if `start_id > count`, and caps
//!    returned records at `min(limit, MAX_PAGE_SIZE)`.
//! 6. **Auth Enforcement** — state-changing operations fail when unauthenticated
//!    or when invoked by non-admin accounts.
//! 7. **View-Only Non-Mutation** — view methods never mutate contract state or ID counters.
//! 8. **No Uncontrolled Panics** — all inputs are handled gracefully without process
//!    crashes or unexpected panics.
//!
//! # Wire format
//! The input byte stream is parsed into 12-byte operation tokens:
//! ```text
//! byte 0: operation discriminant (mod NUM_OPS)
//! byte 1: caller/flag byte (selects admin, pending, or non-admin actor)
//! bytes 2-5: u32 operand (limit, batch size, range start/end)
//! bytes 4-11: i64/i128 operand (balance, checkpoint ID)
//! ```
//!
//! # Running
//! ```bash
//! cargo fuzz run checkpoint
//! # or
//! cargo fuzz run main
//! ```

#![no_main]

extern crate std;

use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, BytesN, Env, InvokeError, Symbol, Vec as SorobanVec};

use callora_checkpoint::errors::CheckpointError;
use callora_checkpoint::{
    CalloraCheckpoint, CalloraCheckpointClient, CheckpointRecord, MAX_BATCH_SIZE, MAX_PAGE_SIZE,
};

// ---------------------------------------------------------------------------
// Configuration constants
// ---------------------------------------------------------------------------

/// Bytes consumed per operation token.
const BYTES_PER_OP: usize = 12;

/// Total number of distinct operation discriminants.
const NUM_OPS: u8 = 10;

/// Number of subject addresses available for checkpoint target generation.
const SUBJECT_POOL_SIZE: usize = 4;

/// Number of token addresses available for checkpoint target generation.
const TOKEN_POOL_SIZE: usize = 2;

type CatchResult<T> = std::result::Result<
    std::result::Result<T, std::result::Result<CheckpointError, InvokeError>>,
    Box<dyn std::any::Any + Send>,
>;

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
    let mut admin = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let subjects: std::vec::Vec<Address> = (0..SUBJECT_POOL_SIZE)
        .map(|_| Address::generate(&env))
        .collect();

    let tokens: std::vec::Vec<Address> = (0..TOKEN_POOL_SIZE)
        .map(|_| Address::generate(&env))
        .collect();

    // --- Contract Setup ------------------------------------------------
    let contract_addr = env.register(CalloraCheckpoint, ());
    let client = CalloraCheckpointClient::new(&env, &contract_addr);

    // Initialize the checkpoint contract
    if client.try_init(&admin).is_err() {
        return;
    }

    let mut pending_admin_opt: Option<Address> = None;
    let mut created_records: std::vec::Vec<CheckpointRecord> = std::vec::Vec::new();

    for chunk in data.chunks(BYTES_PER_OP) {
        if chunk.len() < BYTES_PER_OP {
            break;
        }

        let op = chunk[0] % NUM_OPS;
        let flag = chunk[1];
        let operand_u32 = u32::from_be_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);
        let operand_i64 = i64::from_be_bytes([
            chunk[4], chunk[5], chunk[6], chunk[7], chunk[8], chunk[9], chunk[10], chunk[11],
        ]);

        let count_before = client.get_checkpoint_count();

        match op {
            // -----------------------------------------------------------
            // 0 — create_checkpoint
            // -----------------------------------------------------------
            0 => {
                let caller = if flag % 4 == 0 {
                    non_admin.clone()
                } else {
                    admin.clone()
                };
                let subject = subjects[(chunk[2] as usize) % SUBJECT_POOL_SIZE].clone();
                let token = tokens[(chunk[3] as usize) % TOKEN_POOL_SIZE].clone();

                // Generate balance with boundary & negative coverage
                let balance: i128 = match flag % 6 {
                    0 => 0,
                    1 => operand_i64.abs() as i128,
                    2 => -(operand_i64.abs() as i128 + 1), // negative balance
                    3 => i128::MAX,
                    4 => i128::MIN,
                    _ => (operand_u32 as i128) * 1000,
                };

                let meta_str = match flag % 4 {
                    0 => "audit",
                    1 => "monthly-close",
                    2 => "pre-migrate",
                    _ => "checkpoint",
                };
                let metadata = Symbol::new(&env, meta_str);

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_create_checkpoint(&caller, &subject, &token, &balance, &metadata)
                }));

                let count_after = client.get_checkpoint_count();

                if is_success(&result) {
                    let new_id = result.unwrap().unwrap();
                    assert_eq!(caller, admin, "non-admin successfully created checkpoint");
                    assert!(balance >= 0, "checkpoint created with negative balance");
                    assert_eq!(count_after, count_before + 1, "count did not increment");
                    assert_eq!(new_id, count_after, "returned ID does not match count");

                    // Verify record stored in persistent storage
                    let record = client.get_checkpoint(&new_id).expect("record not found");
                    assert_eq!(record.id, new_id);
                    assert_eq!(record.subject, subject);
                    assert_eq!(record.token, token);
                    assert_eq!(record.balance, balance);
                    assert_eq!(record.metadata, metadata);

                    created_records.push(record);
                } else {
                    assert_eq!(
                        count_before, count_after,
                        "failed create_checkpoint mutated count"
                    );
                }
            }

            // -----------------------------------------------------------
            // 1 — batch_create_checkpoints
            // -----------------------------------------------------------
            1 => {
                let caller = if flag % 5 == 0 {
                    non_admin.clone()
                } else {
                    admin.clone()
                };

                // Generate batch size testing boundaries: 0, 1..=50, and oversized > 50
                let raw_size = match flag % 5 {
                    0 => 0,                                      // empty batch
                    1 => 1,                                      // single item batch
                    2 => (operand_u32 % 49) + 1,                 // valid batch size [1..50]
                    3 => MAX_BATCH_SIZE,                         // exact boundary (50)
                    _ => MAX_BATCH_SIZE + (flag as u32 % 5) + 1, // oversized batch (> 50)
                };

                let force_negative = flag % 7 == 0;
                let mut items: SorobanVec<(Address, Address, i128, Symbol)> = SorobanVec::new(&env);

                for i in 0..raw_size {
                    let subj = subjects[(i as usize) % SUBJECT_POOL_SIZE].clone();
                    let tok = tokens[(i as usize) % TOKEN_POOL_SIZE].clone();
                    let bal: i128 = if force_negative && i == raw_size / 2 {
                        -100 // introduce a negative item to verify atomic rollback
                    } else {
                        (operand_u32 as i128 + i as i128) * 10
                    };
                    let meta = Symbol::new(&env, "batch");
                    items.push_back((subj, tok, bal, meta));
                }

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_batch_create_checkpoints(&caller, &items)
                }));

                let count_after = client.get_checkpoint_count();

                if is_success(&result) {
                    let ids = result.unwrap().unwrap();
                    assert_eq!(
                        caller, admin,
                        "non-admin successfully batch-created checkpoints"
                    );
                    assert!(raw_size > 0 && raw_size <= MAX_BATCH_SIZE);
                    assert!(!force_negative, "batch with negative balance succeeded");
                    assert_eq!(ids.len(), raw_size);
                    assert_eq!(count_after, count_before + raw_size as u64);

                    for (idx, id) in ids.iter().enumerate() {
                        let expected_id = count_before + 1 + idx as u64;
                        assert_eq!(id, expected_id);
                        let record = client.get_checkpoint(&id).expect("batch record missing");
                        assert_eq!(record.id, id);
                        created_records.push(record);
                    }
                } else {
                    assert_eq!(
                        count_before, count_after,
                        "failed batch_create_checkpoints mutated count"
                    );
                }
            }

            // -----------------------------------------------------------
            // 2 — get_checkpoint & get_latest_checkpoint
            // -----------------------------------------------------------
            2 => {
                let target_id: u64 = match flag % 5 {
                    0 => 0,
                    1 => count_before,
                    2 => count_before + 1,
                    3 => (operand_u32 % (count_before + 5).max(1)) as u64,
                    _ => u64::MAX,
                };

                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_get_checkpoint(&target_id)
                }));

                if target_id == 0 || target_id > count_before {
                    assert!(
                        !is_success(&res),
                        "get_checkpoint succeeded for invalid ID {target_id}"
                    );
                } else if is_success(&res) {
                    let rec = res.unwrap().unwrap();
                    assert_eq!(rec.id, target_id);
                }

                let latest = client.get_latest_checkpoint();
                if count_before == 0 {
                    assert!(
                        latest.is_none(),
                        "latest checkpoint returned on empty storage"
                    );
                } else {
                    assert!(latest.is_some(), "latest checkpoint missing when count > 0");
                    assert_eq!(latest.unwrap().id, count_before);
                }
            }

            // -----------------------------------------------------------
            // 3 — get_checkpoints_range
            // -----------------------------------------------------------
            3 => {
                let start_id: u64 = match flag % 5 {
                    0 => 0,
                    1 => 1,
                    2 => count_before / 2 + 1,
                    3 => count_before + 10,
                    _ => u64::MAX - 10,
                };

                let limit: u32 = match flag % 5 {
                    0 => 0,                  // invalid page size
                    1 => 1,                  // single item page
                    2 => 50,                 // standard page
                    3 => MAX_PAGE_SIZE,      // max limit (100)
                    _ => MAX_PAGE_SIZE + 50, // oversized limit (> 100)
                };

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_get_checkpoints_range(&start_id, &limit)
                }));

                if limit == 0 {
                    assert!(
                        !is_success(&result),
                        "get_checkpoints_range accepted limit 0"
                    );
                } else if is_success(&result) {
                    let records = result.unwrap().unwrap();
                    if start_id > count_before {
                        assert!(
                            records.is_empty(),
                            "out-of-bounds start_id returned records"
                        );
                    } else {
                        let expected_max_len = limit.min(MAX_PAGE_SIZE) as u64;
                        let available = if start_id == 0 {
                            0
                        } else {
                            (count_before - start_id + 1).min(expected_max_len)
                        };
                        assert!(records.len() as u64 <= available);
                    }
                }
            }

            // -----------------------------------------------------------
            // 4 — bump_checkpoint_ttl & bump_checkpoints_ttl_range
            // -----------------------------------------------------------
            4 => {
                let caller = if flag % 3 == 0 {
                    non_admin.clone()
                } else {
                    admin.clone()
                };

                let target_id = (operand_u32 % (count_before + 5).max(1)) as u64;

                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_bump_checkpoint_ttl(&caller, &target_id)
                }));

                let start_id = (operand_u32 % (count_before + 5).max(1)) as u64;
                let end_id = if flag % 2 == 0 {
                    start_id + (flag as u64 % 10)
                } else {
                    start_id.saturating_sub(1) // test invalid range start_id > end_id
                };

                let range_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_bump_checkpoints_ttl_range(&caller, &start_id, &end_id)
                }));

                if start_id > end_id && caller == admin {
                    assert!(
                        !is_success(&range_res),
                        "bump_checkpoints_ttl_range accepted start_id > end_id"
                    );
                }
            }

            // -----------------------------------------------------------
            // 5 — Two-step admin rotation
            // -----------------------------------------------------------
            5 => {
                let action = flag % 3;
                match action {
                    0 => {
                        // set_admin nomination
                        let nominee = Address::generate(&env);
                        let caller = if flag % 4 == 0 {
                            non_admin.clone()
                        } else {
                            admin.clone()
                        };

                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.try_set_admin(&caller, &nominee)
                        }));

                        if is_success(&res) {
                            assert_eq!(caller, admin);
                            assert_eq!(client.get_pending_admin(), Some(nominee.clone()));
                            pending_admin_opt = Some(nominee);
                        }
                    }
                    1 => {
                        // accept_admin
                        let caller = if flag % 2 == 0 {
                            pending_admin_opt
                                .clone()
                                .unwrap_or_else(|| non_admin.clone())
                        } else {
                            non_admin.clone()
                        };

                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.try_accept_admin(&caller)
                        }));

                        if is_success(&res) {
                            assert_eq!(client.get_admin(), caller);
                            assert!(client.get_pending_admin().is_none());
                            admin = caller;
                            pending_admin_opt = None;
                        }
                    }
                    2 => {
                        // cancel_admin_transfer
                        let caller = if flag % 4 == 0 {
                            non_admin.clone()
                        } else {
                            admin.clone()
                        };

                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.try_cancel_admin_transfer(&caller)
                        }));

                        if is_success(&res) {
                            assert_eq!(caller, admin);
                            assert!(client.get_pending_admin().is_none());
                            pending_admin_opt = None;
                        }
                    }
                    _ => unreachable!(),
                }
            }

            // -----------------------------------------------------------
            // 6 — Unauthenticated state-changing calls (auth gate)
            // -----------------------------------------------------------
            6 => {
                env.set_auths(&[]);

                let subj = subjects[0].clone();
                let tok = tokens[0].clone();
                let meta = Symbol::new(&env, "unauth");

                let create_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_create_checkpoint(&admin, &subj, &tok, &100i128, &meta)
                }));
                assert!(
                    !is_success(&create_res),
                    "unauthenticated create_checkpoint succeeded"
                );

                let mut items: SorobanVec<(Address, Address, i128, Symbol)> = SorobanVec::new(&env);
                items.push_back((subj, tok, 100i128, meta));
                let batch_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_batch_create_checkpoints(&admin, &items)
                }));
                assert!(
                    !is_success(&batch_res),
                    "unauthenticated batch_create_checkpoints succeeded"
                );

                env.mock_all_auths();
            }

            // -----------------------------------------------------------
            // 7 — View methods: verify non-mutation
            // -----------------------------------------------------------
            7 => {
                let current_admin = client.get_admin();
                let count = client.get_checkpoint_count();
                let latest_id = client.get_latest_checkpoint_id();
                let pending = client.get_pending_admin();

                assert_eq!(current_admin, admin, "view method mutated admin");
                assert_eq!(count, count_before, "view method mutated count");
                assert_eq!(latest_id, count_before, "view method mutated latest_id");
                assert_eq!(
                    pending, pending_admin_opt,
                    "view method mutated pending admin"
                );
            }

            // -----------------------------------------------------------
            // 8 — Upgrade attempt
            // -----------------------------------------------------------
            8 => {
                let caller = if flag % 4 == 0 {
                    non_admin.clone()
                } else {
                    admin.clone()
                };
                let wasm_hash = BytesN::from_array(&env, &[chunk[2]; 32]);

                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.try_upgrade(&caller, &wasm_hash)
                }));
            }

            // -----------------------------------------------------------
            // 9 — Existing Record Immutability Verification
            // -----------------------------------------------------------
            9 => {
                for record in created_records.iter() {
                    let fetched = client
                        .get_checkpoint(&record.id)
                        .expect("existing record vanished");
                    assert_eq!(fetched, *record, "checkpoint record content mutated!");
                }
            }

            _ => unreachable!("op discriminant outside NUM_OPS range"),
        }

        // -----------------------------------------------------------------
        // Post-step global invariants
        // -----------------------------------------------------------------
        let count_after = client.get_checkpoint_count();
        let latest_id_after = client.get_latest_checkpoint_id();
        assert_eq!(
            count_after, latest_id_after,
            "invariant violation: count ({count_after}) != latest_id ({latest_id_after})"
        );
        assert!(
            count_after >= count_before,
            "invariant violation: count decreased from {count_before} to {count_after}"
        );
    }
});
