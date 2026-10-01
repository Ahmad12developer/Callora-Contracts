use crate::{CalloraRecipient, CalloraRecipientClient, RecipientError, MAX_PAGE_SIZE};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String as SorobanString};

/// Helper to create an Env and register the contract, returning (env, admin, client).
fn setup() -> (Env, Address, CalloraRecipientClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_addr = env.register(CalloraRecipient, ());
    let client = CalloraRecipientClient::new(&env, &contract_addr);
    (env, admin, client)
}

/// Helper to create a SorobanString from a &str.
fn name(env: &Env, s: &str) -> SorobanString {
    SorobanString::from_bytes(env, s.as_bytes())
}

// ---------------------------------------------------------------------------
// Initialization tests
// ---------------------------------------------------------------------------

#[test]
fn init_sets_admin_and_count() {
    let (_env, admin, client) = setup();
    client.init(&admin);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_recipient_count(), 0u32);
}

#[test]
fn init_double_init_rejected() {
    let (_env, admin, client) = setup();
    client.init(&admin);
    let err = client.try_init(&admin);
    assert_eq!(err, Err(Ok(RecipientError::AlreadyInitialized)));
}

#[test]
fn init_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_addr = env.register(CalloraRecipient, ());
    let client = CalloraRecipientClient::new(&env, &contract_addr);

    // Without mock_all_auths, require_auth should fail.
    let err = client.try_init(&admin);
    assert!(err.is_err());
}

// ---------------------------------------------------------------------------
// register_recipient tests
// ---------------------------------------------------------------------------

#[test]
fn register_recipient_happy_path() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    let recipient_name = name(&env, "treasury");

    client.register_recipient(&admin, &recipient_name, &addr);

    assert!(client.has_recipient(&recipient_name));
    let record = client.get_recipient(&recipient_name);
    assert_eq!(record.address, addr);
    assert_eq!(record.name, recipient_name);
    assert_eq!(client.get_recipient_count(), 1u32);
}

#[test]
fn register_recipient_duplicate_rejected() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    let addr2 = Address::generate(&env);
    let recipient_name = name(&env, "ops");

    client.register_recipient(&admin, &recipient_name, &addr);
    let err = client.try_register_recipient(&admin, &recipient_name, &addr2);
    assert_eq!(err, Err(Ok(RecipientError::AlreadyRegistered)));
}

#[test]
fn register_recipient_empty_name_rejected() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    let empty_name = name(&env, "");

    let err = client.try_register_recipient(&admin, &empty_name, &addr);
    assert_eq!(err, Err(Ok(RecipientError::InvalidName)));
}

#[test]
fn register_recipient_unauthorized() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_addr = env.register(CalloraRecipient, ());
    let client = CalloraRecipientClient::new(&env, &contract_addr);

    env.mock_all_auths();
    client.init(&admin);

    let outsider = Address::generate(&env);
    let addr = Address::generate(&env);
    let recipient_name = name(&env, "x");

    // Clear mock auths so outsider's require_auth fails.
    env.set_auths(&[]);
    let err = client.try_register_recipient(&outsider, &recipient_name, &addr);
    assert!(err.is_err());
}

// ---------------------------------------------------------------------------
// update_recipient tests
// ---------------------------------------------------------------------------

#[test]
fn update_recipient_happy_path() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr1 = Address::generate(&env);
    let addr2 = Address::generate(&env);
    let recipient_name = name(&env, "vendor");

    client.register_recipient(&admin, &recipient_name, &addr1);
    client.update_recipient(&admin, &recipient_name, &addr2);

    let record = client.get_recipient(&recipient_name);
    assert_eq!(record.address, addr2);
    assert_eq!(client.get_recipient_count(), 1u32);
}

#[test]
fn update_recipient_not_found() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    let missing = name(&env, "nope");

    let err = client.try_update_recipient(&admin, &missing, &addr);
    assert_eq!(err, Err(Ok(RecipientError::NotFound)));
}

// ---------------------------------------------------------------------------
// remove_recipient tests
// ---------------------------------------------------------------------------

#[test]
fn remove_recipient_happy_path() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    let recipient_name = name(&env, "temp");

    client.register_recipient(&admin, &recipient_name, &addr);
    assert_eq!(client.get_recipient_count(), 1u32);

    client.remove_recipient(&admin, &recipient_name);
    assert!(!client.has_recipient(&recipient_name));
    assert_eq!(client.get_recipient_count(), 0u32);
}

#[test]
fn remove_recipient_not_found() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let missing = name(&env, "ghost");
    let err = client.try_remove_recipient(&admin, &missing);
    assert_eq!(err, Err(Ok(RecipientError::NotFound)));
}

// ---------------------------------------------------------------------------
// View-only tests
// ---------------------------------------------------------------------------

#[test]
fn get_recipient_not_found() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let missing = name(&env, "nobody");
    let err = client.try_get_recipient(&missing);
    assert_eq!(err, Err(Ok(RecipientError::NotFound)));
}

#[test]
fn has_recipient_returns_false_for_missing() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let missing = name(&env, "nope");
    assert!(!client.has_recipient(&missing));
}

#[test]
fn get_recipient_count_starts_at_zero() {
    let (_env, admin, client) = setup();
    client.init(&admin);
    assert_eq!(client.get_recipient_count(), 0u32);
}

#[test]
fn uninitialized_contract_returns_not_initialized() {
    let env = Env::default();
    let contract_addr = env.register(CalloraRecipient, ());
    let client = CalloraRecipientClient::new(&env, &contract_addr);

    let err = client.try_get_admin();
    assert_eq!(err, Err(Ok(RecipientError::NotInitialized)));
}

// ---------------------------------------------------------------------------
// list_recipients tests
// ---------------------------------------------------------------------------

#[test]
fn list_recipients_empty() {
    let (_env, admin, client) = setup();
    client.init(&admin);

    let page = client.list_recipients(&0, &10);
    assert!(page.is_empty());
}

#[test]
fn list_recipients_single_page() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let n_a = name(&env, "alpha");
    let n_b = name(&env, "beta");
    let n_c = name(&env, "gamma");

    for n in [&n_a, &n_b, &n_c] {
        let addr = Address::generate(&env);
        client.register_recipient(&admin, n, &addr);
    }

    let page = client.list_recipients(&0, &10);
    assert_eq!(page.len(), 3);

    // All registered names must appear somewhere in the page.
    let contains = |needle: &SorobanString| {
        (0..page.len()).any(|i| page.get(i).unwrap() == *needle)
    };
    assert!(contains(&n_a), "alpha missing");
    assert!(contains(&n_b), "beta missing");
    assert!(contains(&n_c), "gamma missing");
}

#[test]
fn list_recipients_pagination() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let names = [
        name(&env, "r0"),
        name(&env, "r1"),
        name(&env, "r2"),
        name(&env, "r3"),
        name(&env, "r4"),
    ];
    for n in &names {
        let addr = Address::generate(&env);
        client.register_recipient(&admin, n, &addr);
    }

    // Page size 2, walk all pages.
    let page0 = client.list_recipients(&0, &2);
    let page1 = client.list_recipients(&2, &2);
    let page2 = client.list_recipients(&4, &2);

    assert_eq!(page0.len(), 2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 1); // only one entry left

    // Collect all results and verify no duplicates.
    let mut seen: std::vec::Vec<SorobanString> = std::vec::Vec::new();
    for page in [&page0, &page1, &page2] {
        for i in 0..page.len() {
            let entry = page.get(i).unwrap();
            assert!(
                !seen.iter().any(|s| s == &entry),
                "duplicate name across pages"
            );
            seen.push(entry);
        }
    }
    assert_eq!(seen.len(), 5);
}

#[test]
fn list_recipients_page_size_capped() {
    let (env, admin, client) = setup();
    client.init(&admin);

    // Register MAX_PAGE_SIZE + 5 entries.
    for i in 0u32..(MAX_PAGE_SIZE + 5) {
        // Build a unique name like "n000", "n001", ...
        let s = std::format!("n{:03}", i);
        let n = name(&env, &s);
        let addr = Address::generate(&env);
        client.register_recipient(&admin, &n, &addr);
    }

    // Even with a huge limit, result must not exceed MAX_PAGE_SIZE.
    let page = client.list_recipients(&0, &1000);
    assert_eq!(page.len() as u32, MAX_PAGE_SIZE);
}

#[test]
fn list_recipients_start_beyond_count_returns_empty() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let addr = Address::generate(&env);
    client.register_recipient(&admin, &name(&env, "only"), &addr);

    let page = client.list_recipients(&99, &10);
    assert!(page.is_empty());
}

#[test]
fn list_recipients_removed_names_absent() {
    let (env, admin, client) = setup();
    client.init(&admin);

    let n_a = name(&env, "aaa");
    let n_b = name(&env, "bbb");
    let n_c = name(&env, "ccc");

    for n in [&n_a, &n_b, &n_c] {
        let addr = Address::generate(&env);
        client.register_recipient(&admin, n, &addr);
    }

    client.remove_recipient(&admin, &n_b);

    let page = client.list_recipients(&0, &10);
    assert_eq!(page.len(), 2);
    for i in 0..page.len() {
        assert_ne!(
            page.get(i).unwrap(),
            n_b,
            "removed name still appears in listing"
        );
    }
}

#[test]
fn list_recipients_no_auth_required() {
    // list_recipients must be callable without any authentication.
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_addr = env.register(CalloraRecipient, ());
    let client = CalloraRecipientClient::new(&env, &contract_addr);

    env.mock_all_auths();
    client.init(&admin);
    let addr = Address::generate(&env);
    client.register_recipient(&admin, &name(&env, "pub"), &addr);
    env.set_auths(&[]);

    // Must succeed without any auth.
    let page = client.list_recipients(&0, &10);
    assert_eq!(page.len(), 1);
}

#[test]
fn list_recipients_zero_limit_uses_cap() {
    let (env, admin, client) = setup();
    client.init(&admin);

    for n in ["x0", "x1", "x2", "x3", "x4"] {
        let addr = Address::generate(&env);
        client.register_recipient(&admin, &name(&env, n), &addr);
    }

    // limit=0 should fall back to MAX_PAGE_SIZE, returning all 5.
    let page = client.list_recipients(&0, &0);
    assert_eq!(page.len(), 5);
}

// ---------------------------------------------------------------------------
// Rustdoc coverage test
// ---------------------------------------------------------------------------

#[test]
fn every_public_fn_in_lib_has_rustdoc() {
    let source = include_str!("lib.rs")
        .split("// ---------------------------------------------------------------------------\n// Test modules")
        .next()
        .expect("lib.rs contains test module marker");
    let lines: std::vec::Vec<&str> = source.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub(crate) fn ")
            || trimmed.starts_with("pub(super) fn "))
        {
            continue;
        }

        let has_rustdoc = lines[..idx]
            .iter()
            .rev()
            .map(|candidate| candidate.trim_start())
            .find(|candidate| !candidate.is_empty())
            .map(|candidate| candidate.starts_with("///"))
            .unwrap_or(false);

        assert!(
            has_rustdoc,
            "public function on line {} is missing /// rustdoc: {}",
            idx + 1,
            trimmed
        );
    }
}
