//! Property tests for the storage-migration upgrade gate (#1229).
//!
//! # Invariants under test
//!
//! Across arbitrary `(current, target, allow_rollback, backup)` quadrants —
//! i.e. the full documented accept/reject matrix of
//! [`StorageMigrationValidator::validate_before_upgrade`] — the validator must:
//!
//! 1. **Version-skip gate** — `target > current + 1` is always rejected with
//!    [`StorageMigrationError::VersionSkip`], regardless of `allow_rollback`
//!    or backup state. No migration step may ever be skipped.
//! 2. **Rollback gate** — `target < current` is rejected with
//!    [`StorageMigrationError::RollbackNotAllowed`] unless `allow_rollback`
//!    is set, and with [`StorageMigrationError::BackupMissing`] when the flag
//!    is set but no backup marker is present. Only
//!    `allow_rollback && BackupPresent` authorizes a rollback.
//! 3. **Same-version redeploy** — `target == current` is accepted only when
//!    the new layout hash matches the recorded one; a changed layout without
//!    a version bump is rejected with
//!    [`StorageMigrationError::SilentLayoutChange`] (asserted directly in the
//!    proptest for arbitrary currents).
//! 4. **Single-step forward** — `target == current + 1` is accepted when the
//!    source layout is consistent (legacy sentinel, or recorded hash match).
//! 5. **No writes on rejection** — a rejected validation must leave storage
//!    completely untouched: no `AuthorizedUpgrade` record, and the version,
//!    schema hash, and backup markers identical to their pre-call state
//!    (snapshotted via a host-storage fingerprint before and after).
//! 6. **Authorization on acceptance** — an accepted validation records
//!    exactly `(target_version, wasm_hash)` in `AuthorizedUpgrade` and
//!    `is_upgrade_authorized` agrees.
//!
//! # Strategy
//!
//! - `proptest` drives ≥ 512 random cases over a bounded version window
//!   (`0..=MAX_PROBED_VERSION`) that covers every ordering relation
//!   (`<`, `==`, `+1`, `>+1`). `u32`-boundary corners (`u32::MAX`) are
//!   covered by the deterministic seeded cases below.
//! - Deterministic seeded cases run every documented boundary quadrant
//!   (`LEGACY_VERSION` → 1, skip from 0, rollback from `u32::MAX`,
//!   same-version redeploy at the ceiling) so reproducibility does not
//!   depend on proptest shrinking alone.
//! - The Soroban test harness requires storage access inside a contract
//!   invocation, so every case runs under a registered inert contract and
//!   observes storage from within it (same pattern as `src/lib.rs` tests).
//!
//! Validation: `cargo test -p callora-storage-migration`
//!
//! Closes CalloraOrg/Callora-Contracts#1229.

extern crate std;

use callora_storage_migration::{
    zero_layout_hash, StorageKey, StorageMigrationError, StorageMigrationValidator, LEGACY_VERSION,
};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use soroban_sdk::{contract, contractimpl, Address, BytesN, Env};

/// Upper bound of the probed version window. Small enough to keep pairs
/// dense around the ordering boundaries, large enough to exercise skips of
/// many sizes. `u32`-boundary corners are covered by the seeded cases below.
const MAX_PROBED_VERSION: u32 = 12;

/// Minimum number of proptest cases (acceptance criterion: ≥ 512).
const PROPTEST_CASES: u32 = 512;

/// The Soroban test harness requires storage access to happen inside a
/// contract invocation, so every case runs its body under a registered
/// (otherwise inert) contract — same pattern as `src/lib.rs` unit tests.
#[contract]
pub struct Dummy;

#[contractimpl]
impl Dummy {
    #[allow(clippy::unused_self)]
    pub fn ping(_env: Env) {}
}

/// Register the inert host contract once per environment so that every stage
/// of a case (seed → validate → finalize) observes the *same* instance's
/// storage — mirroring a real deployed contract.
fn host_contract(env: &Env) -> Address {
    env.register(Dummy, ())
}

/// Deterministic 32-byte hash with a single non-zero lead byte, so distinct
/// tags yield distinct layout hashes and a zero tag yields the all-zero
/// sentinel (`zero_layout_hash`).
fn tag_hash(env: &Env, tag: u8) -> BytesN<32> {
    let mut a = [0u8; 32];
    a[0] = tag;
    BytesN::from_array(env, &a)
}

/// Whether a non-zero expected-source-layout hash is passed for this case.
///
/// A recorded layout always exists in this proptest, so a pinned source hash
/// is only consistent when it equals the recorded one; the zero sentinel is
/// always allowed (opt-out). Deliberately mismatched pinned hashes exercise
/// [`StorageMigrationError::SchemaMismatch`] and get their own seeded test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceLayout {
    /// Pass the legacy sentinel (skip the source-layout check).
    Sentinel,
    /// Pin the recorded hash (must match — always consistent here).
    Pinned,
}

use SourceLayout::{Pinned, Sentinel};

/// The full outcome oracle: the *only* result the documented matrix permits
/// for this `(current, target, allow_rollback, backup)` input. (With a
/// consistent source layout, the schema-compatibility stage never rejects.)
fn expected_outcome(
    current: u32,
    target: u32,
    allow_rollback: bool,
    backup_present: bool,
) -> Result<(), StorageMigrationError> {
    // ── Ordering / rollback boundaries ─────────────────────────────
    if target < current {
        if !allow_rollback {
            return Err(StorageMigrationError::RollbackNotAllowed);
        }
        if !backup_present {
            return Err(StorageMigrationError::BackupMissing);
        }
    } else if target > current.saturating_add(1) {
        return Err(StorageMigrationError::VersionSkip);
    }
    Ok(())
}

/// Everything the no-write invariant needs, observed from inside the contract.
#[derive(Clone, Debug)]
struct StorageFingerprint {
    authorized: Option<(u32, BytesN<32>)>,
    version: Option<u32>,
    schema_hash: Option<BytesN<32>>,
    backup: Option<bool>,
}

fn fingerprint(env: &Env) -> StorageFingerprint {
    StorageFingerprint {
        authorized: env
            .storage()
            .instance()
            .get::<_, (u32, BytesN<32>)>(&StorageKey::AuthorizedUpgrade),
        version: env.storage().instance().get::<_, u32>(&StorageKey::Version),
        schema_hash: env
            .storage()
            .instance()
            .get::<_, BytesN<32>>(&StorageKey::SchemaLayoutHash),
        backup: env
            .storage()
            .instance()
            .get::<_, bool>(&StorageKey::BackupPresent),
    }
}

/// Full observable outcome of one matrix case, collected entirely inside the
/// contract invocation (storage access is not permitted outside of it).
#[derive(Clone, Debug)]
struct CaseOutcome {
    /// What `validate_before_upgrade` returned (flattened to `Result<(), _>`).
    result: Result<(), StorageMigrationError>,
    /// Storage state before the validation call.
    before: StorageFingerprint,
    /// Storage state after the validation call (accepted or rejected).
    after: StorageFingerprint,
    /// `is_upgrade_authorized(target, wasm_hash)` observed after the call.
    auth_for_target: bool,
    /// `is_upgrade_authorized(target + 1, wasm_hash)` — must be false.
    auth_for_wrong_version: bool,
    /// `is_upgrade_authorized(target, wasm_tag + 1)` — must be false.
    auth_for_wrong_wasm: bool,
}

/// Seed storage at `current` (layout tag = low byte of `current`), validate
/// the upgrade to `target`, and observe the storage fingerprint before and
/// after — all inside the contract invocation.
#[allow(clippy::too_many_arguments)]
fn run_case(
    env: &Env,
    contract: &Address,
    current: u32,
    target: u32,
    allow_rollback: bool,
    backup_present: bool,
    source: SourceLayout,
    wasm_tag: u8,
) -> CaseOutcome {
    env.as_contract(contract, || {
        // Seed the recorded schema at `current` with layout tag `current`.
        StorageMigrationValidator::record_deployed_schema(
            env,
            current,
            &tag_hash(env, current as u8),
        )
        .unwrap_or_else(|e| panic!("seeding version {current} failed: {e:?}"));
        // `set_backup_present(false)` writes an explicit `false` marker; only
        // a `true` marker satisfies the BackupPresent gate. Both fingerprints
        // are taken after seeding, so "untouched" is well-defined either way.
        StorageMigrationValidator::set_backup_present(env, backup_present);

        let before = fingerprint(env);
        assert!(
            before.authorized.is_none(),
            "precondition: no authorization may exist before validation"
        );

        let expected_source = match source {
            Sentinel => zero_layout_hash(env),
            Pinned => tag_hash(env, current as u8), // equals the recorded hash
        };
        let wasm_hash = tag_hash(env, wasm_tag);

        let result = StorageMigrationValidator::validate_before_upgrade(
            env,
            target,
            &tag_hash(env, target as u8),
            &expected_source,
            &wasm_hash,
            allow_rollback,
        );

        let after = fingerprint(env);
        let auth_for_target =
            StorageMigrationValidator::is_upgrade_authorized(env, target, &wasm_hash);
        let auth_for_wrong_version = StorageMigrationValidator::is_upgrade_authorized(
            env,
            target.wrapping_add(1),
            &wasm_hash,
        );
        let auth_for_wrong_wasm = StorageMigrationValidator::is_upgrade_authorized(
            env,
            target,
            &tag_hash(env, wasm_tag.wrapping_add(1)),
        );

        CaseOutcome {
            result: result.map(|_| ()),
            before,
            after,
            auth_for_target,
            auth_for_wrong_version,
            auth_for_wrong_wasm,
        }
    })
}

/// Assert the documented matrix outcome plus the no-write / authorization
/// invariants for one completed case. Shared by the proptest and the seeded
/// edge-case sweep so both exercise byte-identical assertions.
fn assert_case_invariants(
    outcome: &CaseOutcome,
    current: u32,
    target: u32,
    allow_rollback: bool,
    backup_present: bool,
    wasm_tag: u8,
) -> Result<(), TestCaseError> {
    let expected = expected_outcome(current, target, allow_rollback, backup_present);
    prop_assert_eq!(
        outcome.result,
        expected,
        "matrix violation at current={} target={} allow_rollback={} backup={}",
        current,
        target,
        allow_rollback,
        backup_present
    );

    if expected.is_err() {
        // Invariant 5: rejection is a pure read — no marker may change and no
        // `AuthorizedUpgrade` may appear.
        prop_assert!(
            outcome.after.authorized.is_none(),
            "rejected validation wrote AuthorizedUpgrade at current={} target={}",
            current,
            target
        );
        prop_assert_eq!(
            outcome.after.version,
            outcome.before.version,
            "version mutated on reject"
        );
        prop_assert_eq!(
            &outcome.after.schema_hash,
            &outcome.before.schema_hash,
            "schema hash mutated on reject"
        );
        prop_assert_eq!(
            outcome.after.backup,
            outcome.before.backup,
            "backup marker mutated on reject"
        );
        prop_assert!(!outcome.auth_for_target, "rejected case reports authorized");
    } else {
        // Invariant 6: acceptance records exactly (target, wasm_hash).
        prop_assert_eq!(
            &outcome.after.authorized,
            &Some((target, tag_hash_for_lookup(wasm_tag))),
            "accepted validation must record (target, wasm_hash)"
        );
        prop_assert!(
            outcome.auth_for_target,
            "is_upgrade_authorized must agree after acceptance"
        );
        // The authorization must be version- and wasm-specific.
        prop_assert!(
            !outcome.auth_for_wrong_version,
            "authorization verified for wrong version"
        );
        prop_assert!(
            !outcome.auth_for_wrong_wasm,
            "authorization verified for wrong wasm hash"
        );
        // Version and schema hash are untouched pre-finalize.
        prop_assert_eq!(
            outcome.after.version,
            outcome.before.version,
            "version mutated on accept"
        );
        prop_assert_eq!(
            &outcome.after.schema_hash,
            &outcome.before.schema_hash,
            "schema hash mutated on accept"
        );
    }
    Ok(())
}

/// Rebuild the wasm hash used by `run_case` for authorization comparisons.
fn tag_hash_for_lookup(tag: u8) -> BytesN<32> {
    let mut a = [0u8; 32];
    a[0] = tag;
    // The proptest constructs this against an `Env`; the comparison in
    // `assert_case_invariants` needs a value-only rebuild, which is
    // environment-independent for `BytesN` (validated in CI).
    BytesN::from_array(&Env::default(), &a)
}

// ─────────────────────────────────────────────────────────────────────────────
// Proptest: the full accept/reject matrix over arbitrary version pairs
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(PROPTEST_CASES))]

    /// Invariants 1, 2, 5, 6 over arbitrary `(current, target)` pairs plus
    /// random `allow_rollback` / backup flags: the result must be *exactly*
    /// the documented matrix outcome, rejected cases must not write anything
    /// (in particular no `AuthorizedUpgrade`), and accepted cases must record
    /// the authorization.
    #[test]
    fn proptest_upgrade_gate_matrix(
        current in 0_u32..=MAX_PROBED_VERSION,
        target in 0_u32..=MAX_PROBED_VERSION,
        allow_rollback in any::<bool>(),
        backup_present in any::<bool>(),
        wasm_tag in 1_u8..=u8::MAX,
    ) {
        let env = Env::default();
        let contract = host_contract(&env);
        let outcome = run_case(
            &env,
            &contract,
            current,
            target,
            allow_rollback,
            backup_present,
            Sentinel,
            wasm_tag,
        );
        assert_case_invariants(&outcome, current, target, allow_rollback, backup_present, wasm_tag)?;
    }

    /// Invariant 3 (acceptance criterion): a same-version redeploy whose new
    /// layout hash differs from the recorded one must return
    /// `SilentLayoutChange` for *any* current version, never authorize, and
    /// never write.
    #[test]
    fn proptest_same_version_redeploy_changed_layout_is_silent_layout_change(
        current in 0_u32..=MAX_PROBED_VERSION,
        allow_rollback in any::<bool>(),
        backup_present in any::<bool>(),
        wasm_tag in 1_u8..=u8::MAX,
    ) {
        let env = Env::default();
        let contract = host_contract(&env);
        let outcome = env.as_contract(&contract, || {
            StorageMigrationValidator::record_deployed_schema(
                &env,
                current,
                &tag_hash(&env, current as u8),
            )
            .unwrap();
            StorageMigrationValidator::set_backup_present(&env, backup_present);
            let before = fingerprint(&env);

            // New code ships a *changed* layout at the same version. The low
            // bytes of `current` and `current + 1` differ for every value in
            // the probed window, so the layouts are genuinely distinct.
            let changed_layout = tag_hash(&env, current.wrapping_add(1) as u8);
            let res = StorageMigrationValidator::validate_before_upgrade(
                &env,
                current, // target == current
                &changed_layout,
                &zero_layout_hash(&env),
                &tag_hash(&env, wasm_tag),
                allow_rollback,
            );
            let after = fingerprint(&env);
            let auth = StorageMigrationValidator::authorized_upgrade(&env);
            (res.map(|_| ()), before, after, auth)
        });
        let (result, before, after, auth) = outcome;

        prop_assert_ne!(
            tag_hash(&env, current.wrapping_add(1) as u8),
            tag_hash(&env, current as u8),
            "changed layout must differ from the recorded one"
        );
        prop_assert_eq!(
            result,
            Err(StorageMigrationError::SilentLayoutChange),
            "changed layout at same version must be SilentLayoutChange (current={})",
            current
        );

        // Rejection must not write anything.
        prop_assert!(
            after.authorized.is_none() && auth.is_none(),
            "SilentLayoutChange rejection wrote AuthorizedUpgrade"
        );
        prop_assert_eq!(after.version, before.version);
        prop_assert_eq!(after.schema_hash, before.schema_hash);
        prop_assert_eq!(after.backup, before.backup);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Seeded deterministic edge cases (reproducible without proptest shrinking)
// ─────────────────────────────────────────────────────────────────────────────

/// Every documented boundary quadrant, enumerated deterministically:
/// legacy → first tracked version, saturated skips at `u32::MAX`, rollback
/// flags × backup presence, pinned vs sentinel source layout.
#[test]
fn seeded_matrix_edge_cases_match_documented_matrix() {
    let corners: [(u32, u32); 8] = [
        (LEGACY_VERSION, 1),        // legacy first tracked upgrade
        (LEGACY_VERSION, 2),        // skip from legacy
        (1, 2),                     // canonical single step
        (1, 3),                     // canonical skip
        (u32::MAX - 1, u32::MAX),   // single step at the ceiling
        (u32::MAX, u32::MAX),       // same-version redeploy at the ceiling
        (u32::MAX, u32::MAX - 1),   // rollback from the ceiling
        (u32::MAX, LEGACY_VERSION), // rollback to legacy
    ];

    for &(current, target) in &corners {
        for &allow_rollback in &[false, true] {
            for &backup_present in &[false, true] {
                for &source in &[Sentinel, Pinned] {
                    let env = Env::default();
                    let contract = host_contract(&env);
                    let outcome = run_case(
                        &env,
                        &contract,
                        current,
                        target,
                        allow_rollback,
                        backup_present,
                        source,
                        0xA5,
                    );
                    assert_case_invariants(
                        &outcome,
                        current,
                        target,
                        allow_rollback,
                        backup_present,
                        0xA5,
                    )
                    .unwrap();
                }
            }
        }
    }
}

/// Pinned source layout that does not match the recorded one is rejected with
/// `SchemaMismatch` for true migrations (target != current) — and leaves no
/// authorization behind. (A pinned source on a same-version redeploy never
/// reaches the source-layout comparison; the same-version layout check has
/// already gated it, which `proptest_same_version_redeploy_…` covers.)
#[test]
fn seeded_schema_mismatch_leaves_storage_untouched() {
    let pairs: [(u32, u32); 3] = [(3, 4), (5, 6), (0, 1)];
    for &(current, target) in &pairs {
        let env = Env::default();
        let contract = host_contract(&env);
        let (result, before, after) = env.as_contract(&contract, || {
            StorageMigrationValidator::record_deployed_schema(
                &env,
                current,
                &tag_hash(&env, current as u8),
            )
            .unwrap();
            let before = fingerprint(&env);
            // Pin a source layout that differs from the recorded one.
            let wrong_source = tag_hash(&env, (current as u8) ^ 0xFF);
            let res = StorageMigrationValidator::validate_before_upgrade(
                &env,
                target,
                &tag_hash(&env, target as u8),
                &wrong_source,
                &tag_hash(&env, 0x77),
                false,
            );
            let after = fingerprint(&env);
            (res.map(|_| ()), before, after)
        });
        assert_eq!(result, Err(StorageMigrationError::SchemaMismatch));
        assert!(after.authorized.is_none());
        assert_eq!(after.version, before.version);
        assert_eq!(after.schema_hash, before.schema_hash);
        assert_eq!(after.backup, before.backup);
    }
}

/// Full-cycle consistency: whenever the matrix accepts an upgrade, the
/// recorded authorization must satisfy `finalize_migration` in the new code,
/// which then commits `target` + new layout. Whenever it rejects, `finalize`
/// must refuse with `UnauthorizedUpgradeState`.
#[test]
fn seeded_full_cycle_accept_then_finalize_reject_then_finalize() {
    let cases: [(u32, u32, bool, bool); 6] = [
        (LEGACY_VERSION, 1, false, false), // accept
        (1, 2, false, false),              // accept
        (2, 1, true, true),                // accept (sanctioned rollback)
        (2, 1, true, false),               // reject: BackupMissing
        (1, 3, false, false),              // reject: VersionSkip
        (2, 1, false, false),              // reject: RollbackNotAllowed
    ];

    for &(current, target, allow_rollback, backup) in &cases {
        let env = Env::default();
        let contract = host_contract(&env);
        let outcome = run_case(
            &env,
            &contract,
            current,
            target,
            allow_rollback,
            backup,
            Sentinel,
            0x5A,
        );
        assert!(outcome.result.is_ok() || outcome.after.authorized.is_none());

        let new_layout = tag_hash(&env, (target as u8) ^ 0x42);
        let wasm_hash = tag_hash(&env, 0x5A);
        let (finalize_result, version_now) = env.as_contract(&contract, || {
            let res = StorageMigrationValidator::finalize_migration(
                &env,
                target,
                &new_layout,
                &wasm_hash,
            );
            (res, StorageMigrationValidator::current_version(&env))
        });

        if outcome.result.is_ok() {
            // New code finalizes with its own (different) layout.
            let report = finalize_result
                .unwrap_or_else(|e| panic!("finalize after accepted upgrade failed: {e:?}"));
            assert!(report.authorized);
            assert_eq!(version_now, target);
        } else {
            assert_eq!(
                finalize_result,
                Err(StorageMigrationError::UnauthorizedUpgradeState)
            );
            assert_eq!(version_now, current);
        }
    }
}
