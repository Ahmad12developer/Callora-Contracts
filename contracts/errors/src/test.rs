#![cfg(test)]

use super::*;
use soroban_sdk::testutils::storage::Persistent;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::{Address, Env, IntoVal, String, Symbol};

/// Init a fresh contract and return the client + admin.
fn setup(env: &Env) -> (ErrorsContractClient<'_>, Address) {
    env.mock_all_auths();
    let contract_id = env.register_contract(None, ErrorsContract);
    let client = ErrorsContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.init(&admin);
    (client, admin)
}

#[test]
fn test_init_and_register() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    assert_eq!(
        client.try_init(&admin).unwrap_err().unwrap(),
        Error::AlreadyInitialized
    );

    let desc = String::from_str(&env, "Insufficient Balance");
    assert_eq!(client.register_error(&admin, &101, &desc), ());
    assert_eq!(client.get_error_description(&101), Some(desc.clone()));
}

#[test]
fn test_unauthorized_registration() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let fake_admin = Address::generate(&env);

    let desc = String::from_str(&env, "Unauthorized Action");
    assert_eq!(
        client
            .try_register_error(&fake_admin, &102, &desc)
            .unwrap_err()
            .unwrap(),
        Error::Unauthorized
    );

    // A rejected registration must not store anything.
    assert_eq!(client.get_error_description(&102), None);
}

#[test]
fn test_log_error_rejects_unknown_code() {
    let env = Env::default();
    let (client, _admin) = setup(&env);
    let user = Address::generate(&env);

    // 999 was never registered → typed error, nothing recorded.
    assert_eq!(
        client.try_log_error(&user, &999).unwrap_err().unwrap(),
        Error::UnknownErrorCode
    );
    assert_eq!(client.get_recent_error(&user), None);
}

#[test]
fn test_log_error_after_registration() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    let desc = String::from_str(&env, "Insufficient Balance");
    client.register_error(&admin, &101, &desc);

    assert_eq!(client.log_error(&user, &101), ());
    assert_eq!(client.get_recent_error(&user), Some(101));
}

#[test]
fn test_get_error_description_returns_stored_data() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let desc = String::from_str(&env, "Rate Limited");
    client.register_error(&admin, &429, &desc);

    assert_eq!(client.get_error_description(&429), Some(desc.clone()));
    assert_eq!(client.get_error_description(&430), None);
}

#[test]
fn test_log_error_emits_event() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    let desc = String::from_str(&env, "Insufficient Balance");
    client.register_error(&admin, &101, &desc);

    client.log_error(&user, &101);

    let events = env.events().all();
    let event = events.last().unwrap();

    let topics = &event.1;
    assert_eq!(topics.len(), 2);
    let topic0: Symbol = topics.get(0).unwrap().into_val(&env);
    let topic1: Address = topics.get(1).unwrap().into_val(&env);
    assert_eq!(topic0, Symbol::new(&env, "error_logged"));
    assert_eq!(topic1, user);

    let data: u32 = event.2.into_val(&env);
    assert_eq!(data, 101);
}

#[test]
fn test_registry_lookup_extends_ttl() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    let desc = String::from_str(&env, "Insufficient Balance");
    client.register_error(&admin, &101, &desc);
    let key = DataKey::ErrorReg(101);

    // Advance the ledger until the registry entry's remaining TTL has dropped
    // below the refresh threshold, but the entry has not yet expired.
    env.ledger()
        .set_sequence_number(REGISTRY_TTL_BUMP - REGISTRY_TTL_THRESHOLD + 1);

    let ttl_before = env.as_contract(&client.address, || env.storage().persistent().get_ttl(&key));
    assert!(
        ttl_before < REGISTRY_TTL_THRESHOLD,
        "sanity: TTL should be below the bump threshold before the read"
    );

    // The read-through view must refresh the entry.
    assert_eq!(client.get_error_description(&101), Some(desc.clone()));

    let ttl_after = env.as_contract(&client.address, || env.storage().persistent().get_ttl(&key));
    assert_eq!(
        ttl_after, REGISTRY_TTL_BUMP,
        "get_error_description must extend the registry entry's TTL"
    );
}

#[test]
fn test_log_error_extends_registry_ttl() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    let desc = String::from_str(&env, "Insufficient Balance");
    client.register_error(&admin, &101, &desc);
    let key = DataKey::ErrorReg(101);

    env.ledger()
        .set_sequence_number(REGISTRY_TTL_BUMP - REGISTRY_TTL_THRESHOLD + 1);
    let ttl_before = env.as_contract(&client.address, || env.storage().persistent().get_ttl(&key));
    assert!(ttl_before < REGISTRY_TTL_THRESHOLD);

    client.log_error(&user, &101);

    let ttl_after = env.as_contract(&client.address, || env.storage().persistent().get_ttl(&key));
    assert_eq!(
        ttl_after, REGISTRY_TTL_BUMP,
        "log_error must extend the registry entry's TTL for a live code"
    );
}

#[test]
fn test_ttl_constants() {
    assert_eq!(LEDGERS_PER_DAY, 17_280);
    assert_eq!(REGISTRY_TTL_THRESHOLD, LEDGERS_PER_DAY * 30);
    assert_eq!(REGISTRY_TTL_BUMP, LEDGERS_PER_DAY * 60);
}

#[test]
fn test_overflow_protection() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    // Registering u32::MAX makes the code valid, so the arithmetic guard is
    // what rejects it.
    let desc = String::from_str(&env, "Max Code");
    client.register_error(&admin, &u32::MAX, &desc);

    assert_eq!(
        client.try_log_error(&user, &u32::MAX).unwrap_err().unwrap(),
        Error::Overflow
    );
}

#[test]
fn test_overflow_boundary_safe() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    // u32::MAX - 1 is the largest code that does NOT overflow when adding 1.
    let desc = String::from_str(&env, "Boundary");
    client.register_error(&admin, &(u32::MAX - 1), &desc);

    assert_eq!(client.log_error(&user, &(u32::MAX - 1)), ());
}

#[test]
fn test_overflow_zero_code() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    // code = 0: checked_add(1) -> Some(1), no overflow.
    let desc = String::from_str(&env, "Zero");
    client.register_error(&admin, &0u32, &desc);

    assert_eq!(client.log_error(&user, &0u32), ());
}

#[test]
fn test_overflow_edge_round_trip() {
    let env = Env::default();
    let (client, admin) = setup(&env);
    let user = Address::generate(&env);

    let desc = String::from_str(&env, "Boundary");
    client.register_error(&admin, &(u32::MAX - 1), &desc);
    client.register_error(&admin, &u32::MAX, &desc);

    // Boundary: code at max - 1 succeeds, max fails, then max - 1 still succeeds.
    assert_eq!(client.log_error(&user, &(u32::MAX - 1)), ());
    assert_eq!(
        client.try_log_error(&user, &u32::MAX).unwrap_err().unwrap(),
        Error::Overflow
    );
    assert_eq!(client.log_error(&user, &(u32::MAX - 1)), ());
}

#[test]
fn test_overflow_multiple_users_safe() {
    let env = Env::default();
    let (client, admin) = setup(&env);

    // Multiple users logging at different (registered) code values — all safe.
    for i in 0..10u32 {
        let code = u32::MAX - 1 - i;
        let desc = String::from_str(&env, "Range");
        client.register_error(&admin, &code, &desc);

        let user = Address::generate(&env);
        assert_eq!(client.log_error(&user, &code), ());
    }
}

// ---------------------------------------------------------------------------
// Helpers for registration/update tests (#1225)
// ---------------------------------------------------------------------------

/// Initialise a contract with a known admin and return the pieces needed by
/// the tests below.
fn setup_full() -> (Env, Address, ErrorsContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &contract_id);
    client.init(&admin);
    (env, contract_id, client, admin)
}

/// Read the stored description for `code`, if any.
fn stored_desc(env: &Env, contract_id: &Address, code: u32) -> Option<String> {
    env.as_contract(contract_id, || {
        env.storage().persistent().get(&DataKey::ErrorReg(code))
    })
}

/// Remaining TTL (in ledgers) of the persistent `ErrorReg(code)` entry.
fn reg_ttl(env: &Env, contract_id: &Address, code: u32) -> u32 {
    env.as_contract(contract_id, || {
        env.storage().persistent().get_ttl(&DataKey::ErrorReg(code))
    })
}

/// Assert that exactly one event was emitted by `contract_id` and decode it
/// into `(topic0, topic1, (code, desc))`.
fn only_event(env: &Env, contract_id: &Address) -> (Symbol, Address, (u32, String)) {
    let events = env.events().all();
    assert_eq!(events.len(), 1, "expected exactly one event");
    let event = events.get(0).unwrap();
    assert_eq!(
        event.0, *contract_id,
        "event must come from the errors contract"
    );
    let topic0: Symbol = event.1.get(0).unwrap().into_val(env);
    let topic1: Address = event.1.get(1).unwrap().into_val(env);
    let data: (u32, String) = event.2.into_val(env);
    (topic0, topic1, data)
}

fn desc_of_len(env: &Env, len: usize) -> String {
    String::from_str(env, &"x".repeat(len))
}

// ---------------------------------------------------------------------------
// Acceptance criteria (#1225): bounded descriptions
// ---------------------------------------------------------------------------

/// A description of exactly `MAX_DESC_LEN` bytes is accepted (boundary).
#[test]
fn test_register_accepts_description_at_cap() {
    let (env, contract_id, client, admin) = setup_full();
    let desc = desc_of_len(&env, MAX_DESC_LEN as usize);

    assert_eq!(client.register_error(&admin, &101, &desc), ());
    assert_eq!(stored_desc(&env, &contract_id, 101).unwrap(), desc);
}

/// A description of `MAX_DESC_LEN + 1` bytes is rejected with nothing written
/// and no event emitted (long descriptions rejected).
#[test]
fn test_register_rejects_long_description() {
    let (env, contract_id, client, admin) = setup_full();
    let desc = desc_of_len(&env, MAX_DESC_LEN as usize + 1);

    assert_eq!(
        client
            .try_register_error(&admin, &101, &desc)
            .unwrap_err()
            .unwrap(),
        Error::DescriptionTooLong
    );
    assert!(
        stored_desc(&env, &contract_id, 101).is_none(),
        "rejected registration must not write storage"
    );
    assert_eq!(
        env.events().all().len(),
        0,
        "rejected registration must not emit an event"
    );
}

/// The same cap applies on the explicit update path.
#[test]
fn test_update_rejects_long_description() {
    let (env, contract_id, client, admin) = setup_full();
    let original = String::from_str(&env, "original");
    client.register_error(&admin, &101, &original);

    let too_long = desc_of_len(&env, MAX_DESC_LEN as usize + 1);
    assert_eq!(
        client
            .try_update_error(&admin, &101, &too_long)
            .unwrap_err()
            .unwrap(),
        Error::DescriptionTooLong
    );
    assert_eq!(
        stored_desc(&env, &contract_id, 101).unwrap(),
        original,
        "rejected update must leave the stored description untouched"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criteria (#1225): no silent overwrites
// ---------------------------------------------------------------------------

/// Re-registering an existing code is rejected and the original description
/// is preserved (overwrite without update path rejected).
#[test]
fn test_register_rejects_duplicate_and_preserves_original() {
    let (env, contract_id, client, admin) = setup_full();
    let first = String::from_str(&env, "first");
    client.register_error(&admin, &101, &first);

    let second = String::from_str(&env, "second");
    assert_eq!(
        client
            .try_register_error(&admin, &101, &second)
            .unwrap_err()
            .unwrap(),
        Error::AlreadyRegistered
    );
    assert_eq!(
        stored_desc(&env, &contract_id, 101).unwrap(),
        first,
        "duplicate registration must not overwrite the stored description"
    );
    assert_eq!(
        env.events().all().len(),
        0,
        "failed registration must not emit an event"
    );
}

/// `update_error` on a code that was never registered is rejected.
#[test]
fn test_update_requires_existing_code() {
    let (env, _contract_id, client, admin) = setup_full();

    let desc = String::from_str(&env, "phantom");
    assert_eq!(
        client
            .try_update_error(&admin, &999, &desc)
            .unwrap_err()
            .unwrap(),
        Error::NotRegistered
    );
    assert_eq!(
        env.events().all().len(),
        0,
        "failed update must not emit an event"
    );
}

/// Only a non-admin caller is rejected; the stored admin may update.
#[test]
fn test_unauthorized_update() {
    let (env, _contract_id, client, admin) = setup_full();
    client.register_error(&admin, &101, &String::from_str(&env, "old"));

    let fake_admin = Address::generate(&env);
    let desc = String::from_str(&env, "new");
    assert_eq!(
        client
            .try_update_error(&fake_admin, &101, &desc)
            .unwrap_err()
            .unwrap(),
        Error::Unauthorized
    );
}

// ---------------------------------------------------------------------------
// Acceptance criteria (#1225): event emitted and TTL extended
// ---------------------------------------------------------------------------

/// A successful registration emits `error_registered` with the expected
/// topics/payload and leaves the persistent entry at full TTL.
#[test]
fn test_register_emits_event_and_extends_ttl() {
    let (env, contract_id, client, admin) = setup_full();
    let desc = String::from_str(&env, "Insufficient Balance");

    assert_eq!(client.register_error(&admin, &101, &desc), ());

    let (topic0, topic1, (code, payload_desc)) = only_event(&env, &contract_id);
    assert_eq!(topic0, Symbol::new(&env, "error_registered"));
    assert_eq!(topic1, admin);
    assert_eq!(code, 101);
    assert_eq!(payload_desc, desc);
    assert_eq!(
        reg_ttl(&env, &contract_id, 101),
        REGISTRY_TTL_BUMP,
        "registration must extend the persistent entry to full TTL"
    );
}

/// A successful update emits `error_updated`, rewrites the value, and
/// re-extends the TTL after it has aged below the threshold.
#[test]
fn test_update_replaces_description_emits_event_and_extends_ttl() {
    let (env, contract_id, client, admin) = setup_full();
    let original = String::from_str(&env, "old");
    client.register_error(&admin, &101, &original);

    // Age the entry below the extension threshold.
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + REGISTRY_TTL_BUMP - REGISTRY_TTL_THRESHOLD + 10);
    let ttl_before = reg_ttl(&env, &contract_id, 101);
    assert!(
        ttl_before < REGISTRY_TTL_THRESHOLD,
        "precondition: TTL must be below threshold before the update"
    );

    let updated = String::from_str(&env, "new");
    assert_eq!(client.update_error(&admin, &101, &updated), ());

    // NB: the test event buffer is cleared on contract-frame entry (including
    // `as_contract`), so event assertions must run before any storage reads.
    let (topic0, topic1, (code, payload_desc)) = only_event(&env, &contract_id);
    assert_eq!(topic0, Symbol::new(&env, "error_updated"));
    assert_eq!(topic1, admin);
    assert_eq!(code, 101);
    assert_eq!(payload_desc, updated);

    assert_eq!(
        stored_desc(&env, &contract_id, 101).unwrap(),
        updated,
        "update must rewrite the stored description"
    );
    assert_eq!(
        reg_ttl(&env, &contract_id, 101),
        REGISTRY_TTL_BUMP,
        "update must re-extend the persistent entry to full TTL"
    );
}

/// Both write paths reject uninitialised callers with `NotInitialized`.
#[test]
fn test_register_and_update_require_init() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(ErrorsContract, ());
    let client = ErrorsContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let desc = String::from_str(&env, "desc");

    assert_eq!(
        client
            .try_register_error(&admin, &101, &desc)
            .unwrap_err()
            .unwrap(),
        Error::NotInitialized
    );
    assert_eq!(
        client
            .try_update_error(&admin, &101, &desc)
            .unwrap_err()
            .unwrap(),
        Error::NotInitialized
    );
}

// ---------------------------------------------------------------------------
// Acceptance criteria (#1225): log_error behaviour unchanged
// ---------------------------------------------------------------------------

/// `log_error` on a registered code behaves exactly as before: it succeeds,
/// and stores the user's recent code.
#[test]
fn test_log_error_unchanged_for_registered_codes() {
    let (env, contract_id, client, admin) = setup_full();
    let user = Address::generate(&env);

    // Register a description for code 101 so log_error accepts it.
    client.register_error(&admin, &101, &String::from_str(&env, "insufficient"));

    assert_eq!(client.log_error(&user, &101), ());
    let recent: Option<u32> = env.as_contract(&contract_id, || {
        env.storage()
            .temporary()
            .get(&DataKey::RecentErr(user.clone()))
    });
    assert_eq!(recent, Some(101), "log_error must store the recent code");

    // Overflow protection is also unchanged: even a registered description
    // for u32::MAX does not soften the Overflow rejection.
    client.register_error(&admin, &u32::MAX, &String::from_str(&env, "overflow"));
    assert_eq!(
        client.try_log_error(&user, &u32::MAX).unwrap_err().unwrap(),
        Error::Overflow
    );
}
