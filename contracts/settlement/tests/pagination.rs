//! # Pagination integration tests
//!
//! Validates the paged persistent developer index introduced in #1132:
//!
//! - Instance storage no longer holds an unbounded developer vector.
//! - Payment registration cost is independent of total developer count
//!   (membership check is O(1) via `DeveloperMember` flag).
//! - Paginated views (`get_developer_balances_page`, `get_developer_balances_cursor`)
//!   return the same developers before and after `migrate_index_to_pages`.
//! - `migrate_index_to_pages` is resumable and idempotent.

#![cfg(test)]

extern crate std;

use callora_settlement::{
    CalloraSettlement, CalloraSettlementClient, StorageKey, INDEX_PAGE_SIZE,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn setup(env: &Env) -> (Address, Address, Address, CalloraSettlementClient<'_>) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let vault = Address::generate(env);
    let token = Address::generate(env);
    let contract_id = env.register(CalloraSettlement, ());
    let client = CalloraSettlementClient::new(env, &contract_id);
    client.init(&admin, &vault);
    client.set_usdc_token(&admin, &token);
    (admin, vault, token, client)
}

/// Credit `count` distinct developers and return their addresses.
fn credit_developers(
    env: &Env,
    client: &CalloraSettlementClient<'_>,
    vault: &Address,
    token: &Address,
    count: u32,
) -> std::vec::Vec<Address> {
    let mut devs = std::vec::Vec::new();
    for i in 0..count {
        let dev = Address::generate(env);
        devs.push(dev.clone());
        // Use a unique ledger_seq per developer to avoid replay-guard conflicts.
        client.receive_payment(vault, &100i128, &false, &Some(dev), token, &(i + 1));
    }
    devs
}

// ─── 1. Instance storage no longer holds an unbounded vector ─────────────────

/// After crediting several developers the legacy `DeveloperIndex` key must be
/// absent from instance storage; registrations now go to `IndexPage` entries
/// in persistent storage.
#[test]
fn instance_storage_has_no_developer_index_vector() {
    let env = Env::default();
    let (_, vault, token, client) = setup(&env);

    for i in 0..5u32 {
        let dev = Address::generate(&env);
        client.receive_payment(&vault, &100i128, &false, &Some(dev), &token, &(i + 1));
    }

    // The contract address is the last registered; we need to inspect its storage.
    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        let has_old_key = env
            .storage()
            .instance()
            .has(&StorageKey::DeveloperIndex);
        assert!(
            !has_old_key,
            "DeveloperIndex must not be in instance storage after #1132"
        );
    });
}

/// Instance storage holds only a small `IndexPageCount` counter, not the
/// growing address list.
#[test]
fn instance_storage_holds_page_count_not_address_list() {
    let env = Env::default();
    let (_, vault, token, client) = setup(&env);

    for i in 0..10u32 {
        let dev = Address::generate(&env);
        client.receive_payment(&vault, &100i128, &false, &Some(dev), &token, &(i + 1));
    }

    let contract_id = client.address.clone();
    env.as_contract(&contract_id, || {
        let page_count: Option<u32> = env
            .storage()
            .instance()
            .get(&StorageKey::IndexPageCount);
        assert!(
            page_count.is_some(),
            "IndexPageCount must be present in instance storage"
        );
        // A u32 is tiny — not an unbounded Vec<Address>.
        assert!(page_count.unwrap() >= 1);
    });
}

// ─── 2. Payment cost is O(1) — membership check via DeveloperMember flag ─────

/// Crediting the same developer twice must not grow any index structure.
/// The `DeveloperMember` flag makes the duplicate check O(1).
#[test]
fn repeated_payment_to_same_developer_does_not_grow_index() {
    let env = Env::default();
    let (_, vault, token, client) = setup(&env);
    let dev = Address::generate(&env);
    let contract_id = client.address.clone();

    client.receive_payment(&vault, &100i128, &false, &Some(dev.clone()), &token, &1u32);

    let page_count_after_first: u32 = env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .get(&StorageKey::IndexPageCount)
            .unwrap_or(0)
    });

    // Pay the same developer again with a different ledger_seq.
    client.receive_payment(&vault, &200i128, &false, &Some(dev.clone()), &token, &2u32);

    let page_count_after_second: u32 = env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .get(&StorageKey::IndexPageCount)
            .unwrap_or(0)
    });

    assert_eq!(
        page_count_after_first, page_count_after_second,
        "page count must not change when the same developer is credited twice"
    );

    // DeveloperMember flag must exist.
    env.as_contract(&contract_id, || {
        let member: Option<bool> = env
            .storage()
            .persistent()
            .get(&StorageKey::DeveloperMember(dev));
        assert_eq!(member, Some(true), "DeveloperMember flag must be set");
    });
}

/// Verify pages stay bounded at `INDEX_PAGE_SIZE`; a new page is created when
/// a page fills up — so `IndexPageCount` increments correctly.
#[test]
fn index_page_allocation_respects_page_size() {
    let env = Env::default();
    let (_, vault, token, client) = setup(&env);
    let contract_id = client.address.clone();

    // Credit INDEX_PAGE_SIZE + 1 distinct developers.
    let count = INDEX_PAGE_SIZE + 1;
    for i in 0..count {
        let dev = Address::generate(&env);
        client.receive_payment(&vault, &1i128, &false, &Some(dev), &token, &(i + 1));
    }

    env.as_contract(&contract_id, || {
        let page_count: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::IndexPageCount)
            .unwrap_or(0);
        assert_eq!(page_count, 2, "should have spilled into a second page");

        let page0: Vec<Address> = env
            .storage()
            .persistent()
            .get(&StorageKey::IndexPage(0))
            .expect("page 0 must exist");
        assert_eq!(
            page0.len(),
            INDEX_PAGE_SIZE,
            "page 0 must be full"
        );

        let page1: Vec<Address> = env
            .storage()
            .persistent()
            .get(&StorageKey::IndexPage(1))
            .expect("page 1 must exist");
        assert_eq!(page1.len(), 1, "page 1 must have exactly 1 entry");
    });
}

// ─── 3. Paginated views return the same developers ────────────────────────────

/// `get_developer_balances_page` returns all credited developers across pages.
#[test]
fn get_developer_balances_page_returns_all_developers() {
    let env = Env::default();
    let (admin, vault, token, client) = setup(&env);

    let devs = credit_developers(&env, &client, &vault, &token, 5);

    let page = client.get_developer_balances_page(&admin, &0u32, &10u32, &token);
    assert_eq!(page.len(), 5, "all 5 developers must appear");

    // Every credited dev must appear in the result.
    let returned_addrs: std::vec::Vec<Address> =
        page.iter().map(|b| b.address.clone()).collect();
    for dev in &devs {
        assert!(
            returned_addrs.contains(dev),
            "developer {:?} missing from page",
            dev
        );
    }
}

/// `get_developer_balances_cursor` pages through all developers across multiple
/// calls and returns each exactly once.
#[test]
fn cursor_pagination_returns_each_developer_exactly_once() {
    let env = Env::default();
    let (admin, vault, token, client) = setup(&env);

    let total = 7u32;
    credit_developers(&env, &client, &vault, &token, total);

    let mut seen = std::collections::HashSet::new();
    let mut cursor: Option<Address> = None;
    let page_size = 3u32;

    loop {
        let (page, next) = client.get_developer_balances_cursor(&admin, &cursor, &page_size, &token);
        for b in page.iter() {
            // Use a byte representation since soroban Address isn't Hash.
            let key = format!("{:?}", b.address);
            let inserted = seen.insert(key);
            assert!(inserted, "developer appeared more than once in pagination");
        }
        if next.is_none() {
            break;
        }
        cursor = next;
    }

    assert_eq!(seen.len(), total as usize, "all developers must be visited");
}

/// `get_developer_balances_page` with `start` offset skips correctly.
#[test]
fn get_developer_balances_page_start_offset_works() {
    let env = Env::default();
    let (admin, vault, token, client) = setup(&env);

    credit_developers(&env, &client, &vault, &token, 6);

    let full = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);
    let page = client.get_developer_balances_page(&admin, &2u32, &3u32, &token);

    assert_eq!(page.len(), 3);
    // Entries must match positions 2..5 of the full list.
    for i in 0..3usize {
        assert_eq!(page.get(i as u32).unwrap().address, full.get((i + 2) as u32).unwrap().address);
    }
}

// ─── 4. migrate_index_to_pages — resumable and idempotent ────────────────────

/// Seed the legacy `DeveloperIndex` key in instance storage (simulating a
/// contract deployed before #1132) then verify `migrate_index_to_pages` moves
/// entries to pages and removes the old key.
#[test]
fn migrate_index_to_pages_moves_entries_and_removes_old_key() {
    let env = Env::default();
    let (admin, _, token, client) = setup(&env);
    let contract_id = client.address.clone();

    // Simulate a pre-#1132 deployment: write the old flat index directly.
    let devs: std::vec::Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
    env.as_contract(&contract_id, || {
        let mut idx = Vec::new(&env);
        for dev in &devs {
            idx.push_back(dev.clone());
            // Also write a V2 balance so the developer has something to show.
            env.storage().persistent().set(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                &100i128,
            );
        }
        env.storage()
            .instance()
            .set(&StorageKey::DeveloperIndex, &idx);
    });

    // Confirm the old key is present before migration.
    env.as_contract(&contract_id, || {
        assert!(
            env.storage().instance().has(&StorageKey::DeveloperIndex),
            "DeveloperIndex must be present before migration"
        );
    });

    // Run migration in one shot.
    let (_, done) = client.migrate_index_to_pages(&admin, &0u32, &50u32);
    assert!(done, "single-batch migration must complete");

    // Old key must be gone.
    env.as_contract(&contract_id, || {
        assert!(
            !env.storage().instance().has(&StorageKey::DeveloperIndex),
            "DeveloperIndex must be removed after migration"
        );
    });

    // All developers must now appear via the paginated view.
    let page = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);
    assert_eq!(page.len(), 5, "all 5 migrated developers must appear in paginated view");
}

/// Running `migrate_index_to_pages` twice is a no-op on the second call.
#[test]
fn migrate_index_to_pages_is_idempotent() {
    let env = Env::default();
    let (admin, _, token, client) = setup(&env);
    let contract_id = client.address.clone();

    let devs: std::vec::Vec<Address> = (0..3).map(|_| Address::generate(&env)).collect();
    env.as_contract(&contract_id, || {
        let mut idx = Vec::new(&env);
        for dev in &devs {
            idx.push_back(dev.clone());
            env.storage().persistent().set(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                &50i128,
            );
        }
        env.storage()
            .instance()
            .set(&StorageKey::DeveloperIndex, &idx);
    });

    let (_, done1) = client.migrate_index_to_pages(&admin, &0u32, &50u32);
    assert!(done1);

    // Second call — must return done immediately without error.
    let (next2, done2) = client.migrate_index_to_pages(&admin, &0u32, &50u32);
    assert!(done2, "second call must report done");
    assert_eq!(next2, 0u32);

    // Developer count must still be 3.
    let page = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);
    assert_eq!(page.len(), 3);
}

/// Paginated migration: process two addresses per call and verify resumability.
#[test]
fn migrate_index_to_pages_is_resumable() {
    let env = Env::default();
    let (admin, _, token, client) = setup(&env);
    let contract_id = client.address.clone();

    let total = 5u32;
    let devs: std::vec::Vec<Address> = (0..total).map(|_| Address::generate(&env)).collect();
    env.as_contract(&contract_id, || {
        let mut idx = Vec::new(&env);
        for dev in &devs {
            idx.push_back(dev.clone());
            env.storage().persistent().set(
                &StorageKey::DeveloperBalance(dev.clone(), token.clone()),
                &10i128,
            );
        }
        env.storage()
            .instance()
            .set(&StorageKey::DeveloperIndex, &idx);
    });

    // Process 2 at a time.
    let (o1, d1) = client.migrate_index_to_pages(&admin, &0u32, &2u32);
    assert!(!d1, "not done after first batch");
    let (o2, d2) = client.migrate_index_to_pages(&admin, &o1, &2u32);
    assert!(!d2, "not done after second batch");
    let (_, d3) = client.migrate_index_to_pages(&admin, &o2, &2u32);
    assert!(d3, "done after third batch");

    // Old key gone.
    env.as_contract(&contract_id, || {
        assert!(!env.storage().instance().has(&StorageKey::DeveloperIndex));
    });

    // All 5 developers visible.
    let page = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);
    assert_eq!(page.len(), total);
}

/// View results after migration must match what they were before migration.
#[test]
fn paginated_views_same_before_and_after_migration() {
    let env = Env::default();
    let (admin, vault, token, client) = setup(&env);
    let contract_id = client.address.clone();

    // Credit 4 developers through normal payment flow (uses new paged index).
    let devs = credit_developers(&env, &client, &vault, &token, 4);

    let before = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);

    // Now simulate the old flat index alongside to test that migrate_index_to_pages
    // skips already-registered members (idempotency of index_insert via DeveloperMember).
    // We write 2 of the same devs into the legacy key.
    env.as_contract(&contract_id, || {
        let mut idx = Vec::new(&env);
        idx.push_back(devs[0].clone());
        idx.push_back(devs[1].clone());
        env.storage()
            .instance()
            .set(&StorageKey::DeveloperIndex, &idx);
    });

    let (_, done) = client.migrate_index_to_pages(&admin, &0u32, &50u32);
    assert!(done);

    let after = client.get_developer_balances_page(&admin, &0u32, &100u32, &token);

    // Count must be the same (no duplicates added).
    assert_eq!(
        before.len(),
        after.len(),
        "migration must not introduce duplicate entries"
    );
}
