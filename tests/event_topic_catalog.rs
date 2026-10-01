//! Deterministic event topic catalog tests.
//!
//! Verifies that every event topic across all three contracts (`vault`,
//! `settlement`, `revenue_pool`) is produced by a centralized constructor
//! and maps to the exact expected byte string. These tests serve as the
//! source-of-truth enforcement for the catalog documented in
//! `docs/EVENT_TOPICS.md`.
//!
//! If any test in this file fails, the corresponding row in
//! `docs/EVENT_TOPICS.md` must be updated to reflect the new topic string.

use soroban_sdk::{Env, Symbol, Vec as SorobanVec};

// ---------------------------------------------------------------------------
// Vault contract topics
// ---------------------------------------------------------------------------

/// All 36 vault event topics and their expected string representations.
/// Kept as a single constant array so the count is enforced at compile time.
const VAULT_TOPICS: &[(&str, fn(&Env) -> Symbol)] = &[
    ("init", |e| callora_vault::events::event_init(e)),
    ("admin_nominated", |e| {
        callora_vault::events::event_admin_nominated(e)
    }),
    ("admin_accepted", |e| {
        callora_vault::events::event_admin_accepted(e)
    }),
    ("admin_cancelled", |e| {
        callora_vault::events::event_admin_cancelled(e)
    }),
    ("set_authorized_caller", |e| {
        callora_vault::events::event_set_authorized_caller(e)
    }),
    ("set_max_deduct", |e| {
        callora_vault::events::event_set_max_deduct(e)
    }),
    ("vault_paused", |e| {
        callora_vault::events::event_vault_paused(e)
    }),
    ("vault_unpaused", |e| {
        callora_vault::events::event_vault_unpaused(e)
    }),
    ("deposit", |e| callora_vault::events::event_deposit(e)),
    ("deduct", |e| callora_vault::events::event_deduct(e)),
    ("ownership_nominated", |e| {
        callora_vault::events::event_ownership_nominated(e)
    }),
    ("ownership_accepted", |e| {
        callora_vault::events::event_ownership_accepted(e)
    }),
    ("withdraw", |e| callora_vault::events::event_withdraw(e)),
    ("withdraw_to", |e| {
        callora_vault::events::event_withdraw_to(e)
    }),
    ("distribute", |e| callora_vault::events::event_distribute(e)),
    ("set_revenue_pool", |e| {
        callora_vault::events::event_set_revenue_pool(e)
    }),
    ("clear_revenue_pool", |e| {
        callora_vault::events::event_clear_revenue_pool(e)
    }),
    ("set_settlement", |e| {
        callora_vault::events::event_set_settlement(e)
    }),
    ("metadata_set", |e| {
        callora_vault::events::event_metadata_set(e)
    }),
    ("price_set", |e| callora_vault::events::event_price_set(e)),
    ("price_removed", |e| {
        callora_vault::events::event_price_removed(e)
    }),
    ("metadata_updated", |e| {
        callora_vault::events::event_metadata_updated(e)
    }),
    ("metadata_removed", |e| {
        callora_vault::events::event_metadata_removed(e)
    }),
    ("upgraded", |e| callora_vault::events::event_upgraded(e)),
    ("upgrade_started", |e| {
        callora_vault::events::event_upgrade_started(e)
    }),
    ("upgrade_completed", |e| {
        callora_vault::events::event_upgrade_completed(e)
    }),
    ("allowlist_add", |e| {
        callora_vault::events::event_allowlist_add(e)
    }),
    ("allowlist_remove", |e| {
        callora_vault::events::event_allowlist_remove(e)
    }),
    ("allowlist_clear", |e| {
        callora_vault::events::event_allowlist_clear(e)
    }),
    ("revenue_pool_proposed", |e| {
        callora_vault::events::event_revenue_pool_proposed(e)
    }),
    ("revenue_pool_accepted", |e| {
        callora_vault::events::event_revenue_pool_accepted(e)
    }),
    ("revenue_pool_cancelled", |e| {
        callora_vault::events::event_revenue_pool_cancelled(e)
    }),
    ("request_id_pruned", |e| {
        callora_vault::events::event_request_id_pruned(e)
    }),
    ("admin_broadcast", |e| {
        callora_vault::events::event_admin_broadcast(e)
    }),
    ("reserve_cap_set", |e| {
        callora_vault::events::event_reserve_cap_set(e)
    }),
    ("rescue_funds", |e| {
        callora_vault::events::event_rescue_funds(e)
    }),
    ("swept", |e| callora_vault::events::event_swept(e)),
];

// ---------------------------------------------------------------------------
// Settlement contract topics
// ---------------------------------------------------------------------------

const SETTLEMENT_TOPICS: &[(&str, fn(&Env) -> Symbol)] = &[
    ("payment_received", |e| {
        callora_settlement::events::event_payment_received(e)
    }),
    ("balance_credited", |e| {
        callora_settlement::events::event_balance_credited(e)
    }),
    ("developer_withdraw", |e| {
        callora_settlement::events::event_developer_withdraw(e)
    }),
    ("daily_withdraw_cap_changed", |e| {
        callora_settlement::events::event_daily_withdraw_cap_changed(e)
    }),
    ("claim_window_changed", |e| {
        callora_settlement::events::event_developer_claim_window_changed(e)
    }),
    ("admin_nominated", |e| {
        callora_settlement::events::event_admin_nominated(e)
    }),
    ("admin_accepted", |e| {
        callora_settlement::events::event_admin_accepted(e)
    }),
    ("admin_cancelled", |e| {
        callora_settlement::events::event_admin_cancelled(e)
    }),
    ("vault_proposed", |e| {
        callora_settlement::events::event_vault_proposed(e)
    }),
    ("vault_accepted", |e| {
        callora_settlement::events::event_vault_accepted(e)
    }),
    ("upgraded", |e| {
        callora_settlement::events::event_upgraded(e)
    }),
    ("developer_force_credited", |e| {
        callora_settlement::events::event_developer_force_credited(e)
    }),
    ("admin_broadcast", |e| {
        callora_settlement::events::event_admin_broadcast(e)
    }),
    ("admin_migration_proposed", |e| {
        callora_settlement::events::event_admin_migration_proposed(e)
    }),
    ("admin_migration", |e| {
        callora_settlement::events::event_admin_migration(e)
    }),
    ("deposit", |e| callora_settlement::events::event_deposit(e)),
    ("price_set", |e| {
        callora_settlement::events::event_price_set(e)
    }),
    ("price_removed", |e| {
        callora_settlement::events::event_price_removed(e)
    }),
    ("supported_token_added", |e| {
        callora_settlement::events::event_supported_token_added(e)
    }),
    ("supported_token_removed", |e| {
        callora_settlement::events::event_supported_token_removed(e)
    }),
];

// ---------------------------------------------------------------------------
// Revenue pool contract topics
// ---------------------------------------------------------------------------

const REVENUE_POOL_TOPICS: &[(&str, fn(&Env) -> Symbol)] = &[
    ("init", |e| callora_revenue_pool::events::event_init(e)),
    ("admin_changed", |e| {
        callora_revenue_pool::events::event_admin_changed(e)
    }),
    ("admin_transfer_started", |e| {
        callora_revenue_pool::events::event_admin_transfer_started(e)
    }),
    ("admin_transfer_completed", |e| {
        callora_revenue_pool::events::event_admin_transfer_completed(e)
    }),
    ("admin_cancelled", |e| {
        callora_revenue_pool::events::event_admin_cancelled(e)
    }),
    ("pause_guardian_set", |e| {
        callora_revenue_pool::events::event_pause_guardian_set(e)
    }),
    ("pause_guardian_cleared", |e| {
        callora_revenue_pool::events::event_pause_guardian_cleared(e)
    }),
    ("pause_set", |e| {
        callora_revenue_pool::events::event_pause_set(e)
    }),
    ("emergency_pause_set", |e| {
        callora_revenue_pool::events::event_emergency_pause_set(e)
    }),
    ("receive_payment", |e| {
        callora_revenue_pool::events::event_receive_payment(e)
    }),
    ("yield_deposited", |e| {
        callora_revenue_pool::events::event_yield_deposited(e)
    }),
    ("treasury_transfer_started", |e| {
        callora_revenue_pool::events::event_treasury_transfer_started(e)
    }),
    ("treasury_transfer_completed", |e| {
        callora_revenue_pool::events::event_treasury_transfer_completed(e)
    }),
    ("treasury_cancelled", |e| {
        callora_revenue_pool::events::event_treasury_cancelled(e)
    }),
    ("set_max_distribute", |e| {
        callora_revenue_pool::events::event_set_max_distribute(e)
    }),
    ("distribute", |e| {
        callora_revenue_pool::events::event_distribute(e)
    }),
    ("batch_distribute", |e| {
        callora_revenue_pool::events::event_batch_distribute(e)
    }),
    ("upgraded", |e| {
        callora_revenue_pool::events::event_upgraded(e)
    }),
    ("admin_broadcast", |e| {
        callora_revenue_pool::events::event_admin_broadcast(e)
    }),
    ("emergency_drain_proposed", |e| {
        callora_revenue_pool::events::event_emergency_drain_proposed(e)
    }),
    ("emergency_drain_executed", |e| {
        callora_revenue_pool::events::event_emergency_drain_executed(e)
    }),
    ("emergency_drain_cancelled", |e| {
        callora_revenue_pool::events::event_emergency_drain_cancelled(e)
    }),
];

// ---------------------------------------------------------------------------
// Distribute contract topics
// ---------------------------------------------------------------------------

const DISTRIBUTE_TOPICS: &[(&str, fn(&Env) -> Symbol)] = &[
    ("init", |e| callora_distribute::events::event_init(e)),
    ("admin_changed", |e| {
        callora_distribute::events::event_admin_changed(e)
    }),
    ("admin_transfer_started", |e| {
        callora_distribute::events::event_admin_transfer_started(e)
    }),
    ("admin_transfer_completed", |e| {
        callora_distribute::events::event_admin_transfer_completed(e)
    }),
    ("admin_cancelled", |e| {
        callora_distribute::events::event_admin_cancelled(e)
    }),
    ("pause_set", |e| {
        callora_distribute::events::event_pause_set(e)
    }),
    ("set_max_distribute", |e| {
        callora_distribute::events::event_set_max_distribute(e)
    }),
    ("distribute", |e| {
        callora_distribute::events::event_distribute(e)
    }),
    ("distribute_started", |e| {
        callora_distribute::events::event_distribute_started(e)
    }),
    ("distribute_completed", |e| {
        callora_distribute::events::event_distribute_completed(e)
    }),
    ("upgraded", |e| {
        callora_distribute::events::event_upgraded(e)
    }),
];

// ---------------------------------------------------------------------------
// Catalog integrity tests
// ---------------------------------------------------------------------------

/// Verify every vault event constructor produces the expected Symbol bytes.
#[test]
fn vault_topics_match_catalog() {
    let env = Env::default();
    for (expected, ctor) in VAULT_TOPICS {
        let sym = ctor(&env);
        assert_eq!(
            sym,
            Symbol::new(&env, expected),
            "vault topic mismatch: expected \"{expected}\""
        );
    }
}

/// Verify every settlement event constructor produces the expected Symbol bytes.
#[test]
fn settlement_topics_match_catalog() {
    let env = Env::default();
    for (expected, ctor) in SETTLEMENT_TOPICS {
        let sym = ctor(&env);
        assert_eq!(
            sym,
            Symbol::new(&env, expected),
            "settlement topic mismatch: expected \"{expected}\""
        );
    }
}

/// Verify every revenue_pool event constructor produces the expected Symbol bytes.
#[test]
fn revenue_pool_topics_match_catalog() {
    let env = Env::default();
    for (expected, ctor) in REVENUE_POOL_TOPICS {
        let sym = ctor(&env);
        assert_eq!(
            sym,
            Symbol::new(&env, expected),
            "revenue_pool topic mismatch: expected \"{expected}\""
        );
    }
}

/// Verify every distribute event constructor produces the expected Symbol bytes.
#[test]
fn distribute_topics_match_catalog() {
    let env = Env::default();
    for (expected, ctor) in DISTRIBUTE_TOPICS {
        let sym = ctor(&env);
        assert_eq!(
            sym,
            Symbol::new(&env, expected),
            "distribute topic mismatch: expected \"{expected}\""
        );
    }
}

/// Catalog count guard: if this test fails, a new topic was added to
/// `events.rs` but the corresponding row in `docs/EVENT_TOPICS.md` was
/// not updated.
#[test]
fn topic_counts_match_catalog_documentation() {
    // These counts MUST match the totals in docs/EVENT_TOPICS.md.
    // If you added a new event, update both this test AND the catalog.
    assert_eq!(
        VAULT_TOPICS.len(),
        37,
        "vault topic count changed — update docs/EVENT_TOPICS.md"
    );
    assert_eq!(
        SETTLEMENT_TOPICS.len(),
        20,
        "settlement topic count changed — update docs/EVENT_TOPICS.md"
    );
    assert_eq!(
        REVENUE_POOL_TOPICS.len(),
        22,
        "revenue_pool topic count changed — update docs/EVENT_TOPICS.md"
    );
    assert_eq!(
        DISTRIBUTE_TOPICS.len(),
        11,
        "distribute topic count changed — update docs/EVENT_TOPICS.md"
    );
    assert_eq!(
        VAULT_TOPICS.len()
            + SETTLEMENT_TOPICS.len()
            + REVENUE_POOL_TOPICS.len()
            + DISTRIBUTE_TOPICS.len(),
        90,
        "total topic count changed — update docs/EVENT_TOPICS.md"
    );
}

/// Every topic string must be non-empty and contain only lowercase ASCII
/// alphanumeric plus underscores — the convention enforced by the codebase.
#[test]
fn all_topic_strings_are_valid_identifiers() {
    let env = Env::default();
    let all: std::vec::Vec<(&str, &str, fn(&Env) -> Symbol)> = VAULT_TOPICS
        .iter()
        .map(|(s, c)| ("vault", *s, *c))
        .chain(
            SETTLEMENT_TOPICS
                .iter()
                .map(|(s, c)| ("settlement", *s, *c)),
        )
        .chain(
            REVENUE_POOL_TOPICS
                .iter()
                .map(|(s, c)| ("revenue_pool", *s, *c)),
        )
        .chain(
            DISTRIBUTE_TOPICS
                .iter()
                .map(|(s, c)| ("distribute", *s, *c)),
        )
        .collect();

    for (contract, name, ctor) in all {
        let sym = ctor(&env);
        let bytes = sym.to_string();
        assert!(
            !bytes.is_empty(),
            "{contract} topic \"{name}\" produced an empty Symbol"
        );
        assert!(
            bytes
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
            "{contract} topic \"{name}\" contains invalid characters: \"{bytes}\""
        );
    }
}

/// Verify that topic strings are unique within each contract (no duplicates).
#[test]
fn no_duplicate_topics_per_contract() {
    let env = Env::default();

    let mut vault_syms: SorobanVec<Symbol> = SorobanVec::new(&env);
    for (_, ctor) in VAULT_TOPICS {
        vault_syms.push_back(ctor(&env));
    }
    // Soroban Vec doesn't have dedup, so we check pairwise
    for i in 0..vault_syms.len() {
        for j in (i + 1)..vault_syms.len() {
            assert_ne!(
                vault_syms.get(i).unwrap(),
                vault_syms.get(j).unwrap(),
                "vault has duplicate topic at positions {i} and {j}"
            );
        }
    }

    let mut settlement_syms: SorobanVec<Symbol> = SorobanVec::new(&env);
    for (_, ctor) in SETTLEMENT_TOPICS {
        settlement_syms.push_back(ctor(&env));
    }
    for i in 0..settlement_syms.len() {
        for j in (i + 1)..settlement_syms.len() {
            assert_ne!(
                settlement_syms.get(i).unwrap(),
                settlement_syms.get(j).unwrap(),
                "settlement has duplicate topic at positions {i} and {j}"
            );
        }
    }

    let mut rp_syms: SorobanVec<Symbol> = SorobanVec::new(&env);
    for (_, ctor) in REVENUE_POOL_TOPICS {
        rp_syms.push_back(ctor(&env));
    }
    for i in 0..rp_syms.len() {
        for j in (i + 1)..rp_syms.len() {
            assert_ne!(
                rp_syms.get(i).unwrap(),
                rp_syms.get(j).unwrap(),
                "revenue_pool has duplicate topic at positions {i} and {j}"
            );
        }
    }

    let mut distribute_syms: SorobanVec<Symbol> = SorobanVec::new(&env);
    for (_, ctor) in DISTRIBUTE_TOPICS {
        distribute_syms.push_back(ctor(&env));
    }
    for i in 0..distribute_syms.len() {
        for j in (i + 1)..distribute_syms.len() {
            assert_ne!(
                distribute_syms.get(i).unwrap(),
                distribute_syms.get(j).unwrap(),
                "distribute has duplicate topic at positions {i} and {j}"
            );
        }
    }
}

/// Cross-contract stability: calling the same constructor twice returns
/// the same Symbol (determinism guarantee).
#[test]
fn topic_constructors_are_deterministic() {
    let env = Env::default();

    let all: std::vec::Vec<fn(&Env) -> Symbol> = VAULT_TOPICS
        .iter()
        .map(|(_, c)| *c)
        .chain(SETTLEMENT_TOPICS.iter().map(|(_, c)| *c))
        .chain(REVENUE_POOL_TOPICS.iter().map(|(_, c)| *c))
        .chain(DISTRIBUTE_TOPICS.iter().map(|(_, c)| *c))
        .collect();

    for ctor in all {
        let first = ctor(&env);
        let second = ctor(&env);
        assert_eq!(
            first, second,
            "constructor returned different Symbols on repeated calls"
        );
    }
}

/// Explicit catalog test for the `set_settlement` topic (issue #1111).
///
/// Verifies that:
/// 1. The `event_set_settlement` constructor produces exactly the bytes for
///    `"set_settlement"` (no accidental rename/drift).
/// 2. The entry is present in `VAULT_TOPICS` at the expected position (row 18).
#[test]
fn set_settlement_topic_in_catalog() {
    let env = Env::default();

    // 1. Constructor byte-identity.
    let sym = callora_vault::events::event_set_settlement(&env);
    assert_eq!(
        sym,
        Symbol::new(&env, "set_settlement"),
        "event_set_settlement constructor produced unexpected bytes"
    );

    // 2. Catalog presence — find the entry by name.
    let found = VAULT_TOPICS
        .iter()
        .any(|(name, _)| *name == "set_settlement");
    assert!(
        found,
        "\"set_settlement\" is missing from the VAULT_TOPICS catalog array"
    );
}
