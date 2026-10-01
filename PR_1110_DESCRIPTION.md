# fix(vault): unify `is_authorized_depositor` with the allowlist storage

**Closes #1110**

## Summary

`is_authorized_depositor` read `DataKey::Depositor(caller)` — a key nothing in the
contract ever writes — while `deposit`, `add_address`, and `clear_all` all operate on
the `StorageKey::AllowedDepositors` vector. The view therefore always returned
`false`, even for allowlisted addresses, so any frontend or backend that pre-checked
eligibility through it (as the README advertises) would refuse perfectly valid
deposits.

The view and the deposit gate now share one private helper, making this class of
drift structurally impossible.

## What changed

### `contracts/vault/src/lib.rs`

- **New private helper `allowlist_allows(env, caller)`** — the single source of truth
  for the deposit authorization gate: `true` iff `caller` is the vault owner **or**
  is present in `StorageKey::AllowedDepositors`; `false` before `init`.
- **`is_authorized_depositor`** now calls the helper (keeping its instance-TTL bump).
  Rustdoc documents: owner counts as authorized (mirroring `deposit`'s owner bypass),
  the view is exactly `deposit`'s caller-dimension gate, and pause/`min_deposit` are
  deliberately out of scope for an eligibility pre-check.
- **`deposit`** now calls the same helper instead of its own inline owner+vector
  logic. Behavior is byte-for-byte identical (still `VaultError::CallerNotInAllowlist`,
  code 44, on rejection).
- **Removed unused `DataKey::Depositor(Address)` and `DataKey::AllowedDepositorsList`**
  plus the stale commented block that referenced them. Removal is safe: Soroban
  `#[contracttype]` enums encode *variant names* in XDR (not ordinals), so no other
  variant's encoding shifts, and neither key was ever written, so no orphaned state
  exists.

### `contracts/vault/src/test_allowlist.rs` (new)

Six compiled tests covering every acceptance criterion:

| Criterion | Test |
| --- | --- |
| After `add_address`, view returns `true` | `view_returns_true_after_add_address` |
| After `clear_all`, view returns `false` | `view_returns_false_after_clear_all` (also asserts `deposit` then fails with code 44) |
| Property: view matches deposit acceptance for random addresses | `view_matches_deposit_acceptance_for_random_addresses` |
| Unused `DataKey` variants removed/documented | code change + `STORAGE.md` |

The property test runs 8 deterministic seeded xorshift traces of 12 random
`add_address` / duplicate-add / `clear_all` operations over a 6-address pool, and for
every pool member, the owner, and a freshly generated stranger asserts:

- `is_authorized_depositor(addr) == (addr == owner || allowlist.contains(addr))`
- a real `mint` + `try_deposit` **succeeds iff the view returned `true`**, and is
  rejected with `CallerNotInAllowlist` (44) — leaving the tracked balance untouched —
  iff the view returned `false`
- `get_allowlist()` matches the model list

Plus `owner_is_authorized_regardless_of_allowlist_state` (documented owner bypass),
`duplicate_add_is_idempotent_for_view_and_list`, and `view_returns_false_before_init`
(the view never panics pre-init).

### Documentation

- `README.md` — `is_authorized_depositor` entry now describes the real semantics and
  drops the false "panics if uninitialized" claim.
- `contracts/vault/STORAGE.md` — removed/stale `Depositor(Address)` / `DepositorIndex`
  entries marked **Removed (#1110)**, canonical `AllowedDepositors` row added, access
  table corrected, version-history entry 1.5 added.

## State & invariant changes

- **No storage layout change**: same keys, same types, same writers, no migration.
- New invariant, enforced by construction: for all addresses,
  `is_authorized_depositor(a) ≡ (a == owner ∨ AllowedDepositors.contains(a)) ≡ deposit's gate`.

## Security & failure-mode handling

- The view remains auth-free, read-only, and TTL-bumping; no new write path or auth
  surface — the shared helper removes a *drift* bug rather than adding capability.
- Pre-init reads return `false` (no panic) — documented.
- Pause / min-deposit / balance are intentionally not reflected (the view answers
  eligibility only); documented in rustdoc and README, and the property test runs
  unpaused with a valid amount.
- Rejection path unchanged: code 44, balance untouched — asserted in tests.

## Compatibility

- Public API surface unchanged (signatures, events, error codes); only the view's
  return values change — that is the fix.
- No repo-local references to the removed `DataKey` variants existed (Rust, TS, or
  scripts); upgrade-safe with no migration.

## Validation

```
cargo test -p callora-vault allowlist   # 9 passed (issue-mandated command)
cargo test -p callora-vault             # 160 passed, 0 failed
```

rustfmt: all files touched by this PR are clean; clippy: no new warnings.

## Pre-existing issues encountered (out of scope, verified on `main`)

1. `test_value_conservation.rs` did not import the `Events` testutils trait, so the
   entire vault test target failed to compile on `main` — fixed here because the
   mandated validation command cannot run otherwise.
2. Two value-conservation tests compared `events().all()` across invocations; the
   Soroban host's event buffer only covers the most recent top-level invocation
   (verified empirically). Reasserted correctly as "the failed call published 0
   events" — a stronger check than the original.
3. `cargo test -p callora-allowlist` fails to compile on `main` (tests call
   `set_allowed_depositor` / `clear_allowed_depositors` / `get_allowed_depositors`,
   which no longer exist, and a 7-arg `init`). Unrelated to this issue; flagged for a
   separate ticket.
