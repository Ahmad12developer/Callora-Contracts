//! Metadata and offering-id byte-length and encoding validation for `callora-registry`.
//!
//! Issue #1065: value-bearing entry points must bound metadata byte length and
//! reject invalid encodings *before* any cross-contract call or state change.
//!
//! Issue #1208: tests are parameterised over both `register_offering` and
//! `register_offering_with_gate` so that fixes to one path are automatically
//! verified on the other.
//!
//! These tests pin the behaviour introduced by routing `validate_metadata`
//! through `callora_validators::normalize_visible_ascii`:
//! - Reject empty, over-length, control-character, non-visible-ASCII
//!   (including UTF-8 multibyte), and leading/trailing-whitespace metadata.
//! - Accept bounded visible-ASCII metadata (including exactly 256 bytes).
//! - A rejected call leaves no partial state (not registered, count unchanged,
//!   catalog `put_offering` never called).
//!
//! Offering ids are additionally validated through
//! `callora_validators::normalize_visible_ascii` so that control bytes,
//! leading/trailing whitespace, and non-visible-ASCII ids are rejected with
//! `InvalidOfferingId` (not `InvalidMetadata`).
//!
//! Before the fix only emptiness and length (256) were checked, so the
//! control-character / whitespace / non-ASCII cases below were *accepted*.

extern crate std;

use callora_registry::{admin, CalloraRegistry, CalloraRegistryClient, RegistryError};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{contract, contractimpl, Address, Env, String};

// ---------------------------------------------------------------------------
// Mock catalog that records whether `put_offering` was invoked
// ---------------------------------------------------------------------------

pub mod ok_catalog {
    use super::*;

    #[contract]
    pub struct OkCatalog;

    #[contractimpl]
    impl OkCatalog {
        pub fn put_offering(
            _env: Env,
            _registry: Address,
            _offering_id: String,
            _metadata: String,
        ) {
        }
    }
}

// ---------------------------------------------------------------------------
// Registration-variant abstraction
// ---------------------------------------------------------------------------

/// Drives either `register_offering` or `register_offering_with_gate` through
/// the same test cases so that every metadata-validation rule is exercised on
/// both entrypoints.
enum RegisterVariant {
    /// Plain registration — no balance gate.
    Plain,
    /// Balance-gated registration.
    ///
    /// The mock token is pre-funded with `10_000` units so that the gate is
    /// always satisfied when the metadata itself is the thing under test.
    /// Tests that specifically target the balance gate use `try_register_with_gate`
    /// directly in `xcontract.rs` / `overflow_safe.rs`.
    Gated,
}

impl RegisterVariant {
    fn name(&self) -> &'static str {
        match self {
            RegisterVariant::Plain => "register_offering",
            RegisterVariant::Gated => "register_offering_with_gate",
        }
    }

    /// Returns `true` if the registration succeeded, `false` if it returned
    /// `InvalidMetadata`, or panics with a diagnostic if any other outcome
    /// occurs.
    ///
    /// This design avoids fighting the Soroban SDK's generated `try_*` return
    /// type in a generic function signature, while still giving callers enough
    /// information to assert on.
    fn try_register_expect_invalid_metadata_or_ok(
        &self,
        env: &Env,
        client: &CalloraRegistryClient<'_>,
        admin: &Address,
        developer: &Address,
        offering_id: &String,
        meta: &String,
    ) -> bool {
        match self {
            RegisterVariant::Plain => {
                match client.try_register_offering(admin, developer, offering_id, meta) {
                    Ok(_) => true,
                    Err(Ok(RegistryError::InvalidMetadata)) => false,
                    other => panic!(
                        "{}: unexpected result: {:?}",
                        self.name(),
                        other
                    ),
                }
            }
            RegisterVariant::Gated => {
                let owner = Address::generate(env);
                let sac = env.register_stellar_asset_contract_v2(owner);
                let token_addr = sac.address();
                soroban_sdk::token::StellarAssetClient::new(env, &token_addr)
                    .mint(developer, &10_000);
                match client.try_register_offering_with_gate(
                    admin,
                    developer,
                    &token_addr,
                    &100i128,
                    offering_id,
                    meta,
                ) {
                    Ok(_) => true,
                    Err(Ok(RegistryError::InvalidMetadata)) => false,
                    other => panic!(
                        "{}: unexpected result: {:?}",
                        self.name(),
                        other
                    ),
                }
            }
        }
    }

    /// Call the appropriate entrypoint and panic on any error.
    fn register(
        &self,
        env: &Env,
        client: &CalloraRegistryClient<'_>,
        admin: &Address,
        developer: &Address,
        offering_id: &String,
        meta: &String,
    ) {
        let ok = self.try_register_expect_invalid_metadata_or_ok(
            env, client, admin, developer, offering_id, meta,
        );
        assert!(ok, "{}: expected registration to succeed", self.name());
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn offering_id(env: &Env, suffix: &str) -> String {
    String::from_str(env, &std::format!("offering-{suffix}"))
}

fn metadata(env: &Env, s: &str) -> String {
    String::from_str(env, s)
}

fn setup_registry(env: &Env) -> (Address, CalloraRegistryClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let developer = Address::generate(env);
    let catalog = env.register(ok_catalog::OkCatalog, ());
    let registry_id = env.register(CalloraRegistry, ());
    let client = CalloraRegistryClient::new(env, &registry_id);
    client.init(&admin, &catalog);
    (admin, client, developer)
}

/// Advance the ledger timestamp past the admin cooldown window so the next
/// admin-gated action is not blocked.
fn advance_past_cooldown(env: &Env) {
    let current = env.ledger().get().timestamp;
    env.ledger().set(LedgerInfo {
        timestamp: current + admin::COOLDOWN_SECONDS + 1,
        ..env.ledger().get()
    });
}

// ---------------------------------------------------------------------------
// Parameterised test bodies
// ---------------------------------------------------------------------------

/// Shared assertion: `meta_str` must be rejected with `InvalidMetadata` and
/// the registry must remain untouched.
fn assert_invalid_metadata(variant: &RegisterVariant, meta_str: &str, oid_suffix: &str) {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = offering_id(&env, oid_suffix);
    let meta = metadata(&env, meta_str);

    let accepted = variant.try_register_expect_invalid_metadata_or_ok(
        &env, &client, &admin, &developer, &oid, &meta,
    );
    assert!(
        !accepted,
        "{}: expected InvalidMetadata for {:?}",
        variant.name(),
        meta_str,
    );
    assert!(
        !client.is_offering_registered(&oid),
        "{}: offering must not be persisted after rejection",
        variant.name(),
    );
    assert_eq!(
        client.registered_count(),
        0,
        "{}: count must remain 0 after rejection",
        variant.name(),
    );
}

/// Shared assertion: `meta_str` must be accepted by `variant`.
fn assert_valid_metadata(variant: &RegisterVariant, meta_str: &str, oid_suffix: &str) {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = offering_id(&env, oid_suffix);
    let meta = metadata(&env, meta_str);

    variant.register(&env, &client, &admin, &developer, &oid, &meta);
    assert!(
        client.is_offering_registered(&oid),
        "{}: offering must be registered after success",
        variant.name(),
    );
    assert_eq!(
        client.registered_count(),
        1,
        "{}: count must be 1 after success",
        variant.name(),
    );
}

// ---------------------------------------------------------------------------
// Reject invalid encodings (control bytes, non-visible ASCII, whitespace)
// ---------------------------------------------------------------------------

#[test]
fn rejects_control_character_in_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, "bad\u{0000}metadata", "ctl-plain");
}

#[test]
fn rejects_control_character_in_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "bad\u{0000}metadata", "ctl-gated");
}

#[test]
fn rejects_line_break_in_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, "line1\nline2", "nl-plain");
}

#[test]
fn rejects_line_break_in_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "line1\nline2", "nl-gated");
}

#[test]
fn rejects_non_ascii_multibyte_metadata_plain() {
    // Non-ASCII UTF-8 byte (U+00E9 / é) is not visible ASCII.
    assert_invalid_metadata(&RegisterVariant::Plain, "caf\u{e9}", "uni-plain");
}

#[test]
fn rejects_non_ascii_multibyte_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "caf\u{e9}", "uni-gated");
}

#[test]
fn rejects_leading_whitespace_in_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, " ipfs://cid", "lead-plain");
}

#[test]
fn rejects_leading_whitespace_in_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, " ipfs://cid", "lead-gated");
}

#[test]
fn rejects_trailing_whitespace_in_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, "ipfs://cid ", "trail-plain");
}

#[test]
fn rejects_trailing_whitespace_in_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "ipfs://cid ", "trail-gated");
}

#[test]
fn rejects_whitespace_only_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, "   ", "ws-only-plain");
}

#[test]
fn rejects_whitespace_only_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "   ", "ws-only-gated");
}

#[test]
fn rejects_empty_metadata_plain() {
    assert_invalid_metadata(&RegisterVariant::Plain, "", "empty-plain");
}

#[test]
fn rejects_empty_metadata_gated() {
    assert_invalid_metadata(&RegisterVariant::Gated, "", "empty-gated");
}

#[test]
fn rejects_over_length_metadata_plain() {
    // 257 bytes of visible ASCII exceeds the 256-byte bound.
    let long: std::string::String = "a".repeat(257);
    assert_invalid_metadata(&RegisterVariant::Plain, &long, "long-plain");
}

#[test]
fn rejects_over_length_metadata_gated() {
    let long: std::string::String = "a".repeat(257);
    assert_invalid_metadata(&RegisterVariant::Gated, &long, "long-gated");
}

// ---------------------------------------------------------------------------
// Legacy single-variant test (updated for #1208 ordering)
//
// After #1208 the gated path checks the developer balance *before* delegating
// to do_register.  The test below confirms that metadata validation still
// fires (via do_register) when the balance gate is satisfied, using a
// properly funded SAC token.
// ---------------------------------------------------------------------------

#[test]
fn rejects_invalid_encoding_in_with_gate_variant() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = offering_id(&env, "gate-ctl");
    let meta = metadata(&env, "bad\u{0001}meta");

    // Use a real SAC so the balance gate passes; only the metadata is invalid.
    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner);
    let token_addr = sac.address();
    soroban_sdk::token::StellarAssetClient::new(&env, &token_addr).mint(&developer, &10_000);

    let result = client.try_register_offering_with_gate(
        &admin,
        &developer,
        &token_addr,
        &100i128,
        &oid,
        &meta,
    );
    assert!(matches!(result, Err(Ok(RegistryError::InvalidMetadata))));
    assert!(!client.is_offering_registered(&oid));
    assert_eq!(client.registered_count(), 0);
}

// ---------------------------------------------------------------------------
// Accept valid bounded visible-ASCII metadata
// ---------------------------------------------------------------------------

#[test]
fn accepts_visible_ascii_metadata_plain() {
    assert_valid_metadata(&RegisterVariant::Plain, "ipfs://QmAccept", "ok-plain");
}

#[test]
fn accepts_visible_ascii_metadata_gated() {
    assert_valid_metadata(&RegisterVariant::Gated, "ipfs://QmAccept", "ok-gated");
}

#[test]
fn accepts_exactly_max_length_metadata_plain() {
    let exact: std::string::String = "x".repeat(256);
    assert_valid_metadata(&RegisterVariant::Plain, &exact, "max-plain");
}

#[test]
fn accepts_exactly_max_length_metadata_gated() {
    let exact: std::string::String = "x".repeat(256);
    assert_valid_metadata(&RegisterVariant::Gated, &exact, "max-gated");
}

// ---------------------------------------------------------------------------
// Rejection is atomic: no catalog call, count unchanged, nothing persisted
// ---------------------------------------------------------------------------

#[test]
fn rejected_metadata_leaves_no_partial_state_plain() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);

    // Valid registration increments the count.
    client.register_offering(
        &admin,
        &developer,
        &offering_id(&env, "valid-plain"),
        &metadata(&env, "ipfs://cid"),
    );
    assert_eq!(client.registered_count(), 1);

    // A rejected registration after the cooldown must not change anything.
    advance_past_cooldown(&env);
    let oid_bad = offering_id(&env, "bad-plain");
    let result = client.try_register_offering(
        &admin,
        &developer,
        &oid_bad,
        &metadata(&env, "has\nnewline"),
    );
    assert!(matches!(result, Err(Ok(RegistryError::InvalidMetadata))));
    assert!(!client.is_offering_registered(&oid_bad));
    assert_eq!(client.registered_count(), 1);
}

#[test]
fn rejected_metadata_leaves_no_partial_state_gated() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);

    // Mint enough balance for the gate so only the metadata is invalid.
    let owner = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(owner);
    let token_addr = sac.address();
    soroban_sdk::token::StellarAssetClient::new(&env, &token_addr).mint(&developer, &10_000);

    // Valid registration increments the count.
    client.register_offering_with_gate(
        &admin,
        &developer,
        &token_addr,
        &100i128,
        &offering_id(&env, "valid-gated"),
        &metadata(&env, "ipfs://cid"),
    );
    assert_eq!(client.registered_count(), 1);

    // A rejected registration after the cooldown must not change anything.
    advance_past_cooldown(&env);
    let oid_bad = offering_id(&env, "bad-gated");
    let result = client.try_register_offering_with_gate(
        &admin,
        &developer,
        &token_addr,
        &100i128,
        &oid_bad,
        &metadata(&env, "has\nnewline"),
    );
    assert!(matches!(result, Err(Ok(RegistryError::InvalidMetadata))));
    assert!(!client.is_offering_registered(&oid_bad));
    assert_eq!(client.registered_count(), 1);
}

// ---------------------------------------------------------------------------
// Reject invalid offering ids (control bytes, whitespace, non-visible ASCII)
// ---------------------------------------------------------------------------

#[test]
fn rejects_control_character_in_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "bad\u{0000}id");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(
        matches!(result, Err(Ok(RegistryError::InvalidOfferingId))),
        "control char offering id must be rejected with InvalidOfferingId, got {:?}",
        result
    );
    assert!(!client.is_offering_registered(&oid));
    assert_eq!(client.registered_count(), 0);
}

#[test]
fn rejects_line_break_in_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "line1\nline2");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_non_ascii_multibyte_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "caf\u{e9}");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_leading_whitespace_in_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, " offering-1");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_trailing_whitespace_in_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "offering-1 ");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_whitespace_only_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "   ");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_empty_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "");
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_over_length_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let long: std::string::String = "a".repeat(65);
    let oid = metadata(&env, &long);
    let meta = metadata(&env, "ipfs://cid");

    let result = client.try_register_offering(&admin, &developer, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
}

#[test]
fn rejects_invalid_offering_id_in_with_gate_variant() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = metadata(&env, "bad\u{0001}id");
    let token = Address::generate(&env);
    let meta = metadata(&env, "ipfs://cid");

    let result =
        client.try_register_offering_with_gate(&admin, &developer, &token, &100i128, &oid, &meta);
    assert!(matches!(result, Err(Ok(RegistryError::InvalidOfferingId))));
    assert!(!client.is_offering_registered(&oid));
    assert_eq!(client.registered_count(), 0);
}

// ---------------------------------------------------------------------------
// Accept valid offering ids
// ---------------------------------------------------------------------------

#[test]
fn accepts_valid_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let oid = offering_id(&env, "ok");
    let meta = metadata(&env, "ipfs://cid");

    client.register_offering(&admin, &developer, &oid, &meta);
    assert!(client.is_offering_registered(&oid));
    assert_eq!(client.registered_count(), 1);
}

#[test]
fn accepts_exactly_max_length_offering_id() {
    let env = Env::default();
    let (admin, client, developer) = setup_registry(&env);
    let exact: std::string::String = "a".repeat(64);
    let oid = metadata(&env, &exact);
    let meta = metadata(&env, "ipfs://cid");

    client.register_offering(&admin, &developer, &oid, &meta);
    assert!(client.is_offering_registered(&oid));
    assert_eq!(client.registered_count(), 1);
}
