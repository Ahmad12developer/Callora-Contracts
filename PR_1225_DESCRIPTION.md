Closes #1225

## Summary

- `register_error` now bounds descriptions to `MAX_DESC_LEN` (256 bytes) and **rejects re-registration** of an existing code; the new `update_error` entrypoint is the only mutation path.
- Both write paths emit an event (`error_registered` / `error_updated`) and extend the persistent `ErrorReg(code)` entry to `PERSISTENT_BUMP_AMOUNT` (50,000 ledgers).
- New error variants are **appended** (5–7); existing discriminants 1–4 are untouched per the stability rules in `docs/ERROR_CODES.md`.
- `log_error` and `init` are byte-for-byte unchanged.

## Acceptance criteria → code / tests

| Criterion | Code | Tests |
|---|---|---|
| Long descriptions rejected | `MAX_DESC_LEN` check in `register_error` + `update_error` (`contracts/errors/src/lib.rs`) → `Error::DescriptionTooLong` | `test_register_accepts_description_at_cap` (256 ok), `test_register_rejects_long_description` (257 rejected, no write, no event), `test_update_rejects_long_description` (update path, original preserved) |
| Overwrite without update path rejected | `has(&ErrorReg(code))` guard → `Error::AlreadyRegistered`; new `update_error` → `Error::NotRegistered` for unknown codes | `test_register_rejects_duplicate_and_preserves_original` (stored value provably unchanged), `test_update_requires_existing_code`, `test_unauthorized_update` |
| Event emitted and TTL extended | `events::emit_error_registered` / `emit_error_updated` (`contracts/errors/src/events.rs`), `extend_ttl(50_000, 50_000)` after each write | `test_register_emits_event_and_extends_ttl` (exact topics + payload + TTL), `test_update_replaces_description_emits_event_and_extends_ttl` (re-extend after TTL aged below threshold), `test_update_rejects_*`/duplicate tests assert **0 events** on failure, topic byte-identity snapshots in `events.rs` |
| `log_error` unchanged for registered codes | `log_error` not modified (diff shows no changes to it) | `test_log_error_unchanged_for_registered_codes` (register then log succeeds, temporary entry written, `log_error(u32::MAX)` still `Overflow` even with a `u32::MAX` description registered) |
| Validation `cargo test -p errors` | — | 26 passing: 20 unit + 6 proptest (64 seeded traces + 128 random sequences of up to 48 actions, now including update actions and a `BTreeMap` reference model of the registry) |

## Security & failure-mode handling

- **Auth-first ordering.** Both entrypoints check `require_auth` → stored-admin match *before* any length/existence validation, so a non-admin learns nothing (always `Unauthorized`) about registry contents or input bounds.
- **Transactional atomicity.** Every rejection (`DescriptionTooLong`, `AlreadyRegistered`, `NotRegistered`, `Unauthorized`, `NotInitialized`) returns before any storage write, TTL change, or event; Soroban discards the frame, so no partial state or orphan events are possible. Negative tests assert both empty storage and zero events.
- **Bounded resource usage.** A 256-byte cap bounds persistent entry size and event payload size (aligned with `MAX_METADATA_LEN`/`MAX_MESSAGE_LEN` elsewhere in the repo), closing the unbounded-storage vector called out in the issue.
- **Registry integrity.** Descriptions are now append-once/mutate-explicit, so indexers and frontends can treat `ErrorReg` as a stable public interface; every mutation is observable on-ledger via a distinct event.
- **TTL / archival.** Successful writes bump the persistent entry to 50,000 ledgers (same convention as `refund`/`settlement`); entries untouched by admin activity can still be archived, which is the documented behaviour of persistent storage.
- **No new arithmetic.** The only arithmetic path remains `log_error`'s `checked_add` overflow guard, unchanged.

## Compatibility

- `register_error` signature unchanged; behaviour is strictly stricter (no in-repo callers exist — verified by grep).
- `update_error` is additive; storage keys/tiers unchanged; events are new observables.
- Proptest suite extended (not weakened): the old "repeat registration always succeeds" assumption was replaced by an `AlreadyRegistered` expectation backed by a reference model; wrong-admin invariants still assert `Unauthorized`.
- `contracts/errors/docs/storage.md` updated (immutability, TTL, events; stale key names corrected) and `docs/ERROR_CODES.md` gains the missing `## Errors` table (codes 1–7).

## Validation

- `cargo test -p errors` → **26 passed** (issue-mandated command)
- `cargo fmt -p errors -- --check` → clean
- `cargo clippy -p errors --all-targets -- -D warnings` → clean (also removed the 8 pre-existing `Env::register_contract` deprecation warnings in the touched test file)
- `cargo build -p errors --target wasm32-unknown-unknown --release` → succeeds
- No inline `Symbol::new` in publish call sites; all topic constructors live in `events.rs` with byte-identity tests (`scripts/check-event-shape.sh` only lints vault/settlement/revenue_pool, which are untouched; the script needs GNU grep and runs in CI on Linux)
