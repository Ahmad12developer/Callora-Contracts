# Distribute TTL verification (#1178)

## What is proposed

Keep one named 30/60-day instance TTL policy and refresh it from all nine public
views. Previous public constant names remain aliases. Storage layout, return
values and authorization checks are unchanged. The focused regressions live in
`src/test_ttl.rs`; the original `src/test.rs` tests are untouched relative to main.

## Exact semantics

With the repository's locked Soroban SDK 22.0.11 / host 22.1.3, the host extends
when `remaining_ttl <= threshold`, including equality, and never shortens TTL.
TTL zero is the final live ledger. The boundary test verifies those facts against
the host rather than comparing two copies of an arithmetic expression.

A simulated view does not commit a TTL extension on-chain. Clients which only
use `simulateTransaction` still need a submitted invocation or an explicit TTL
extension transaction. See the official [RPC simulation documentation](https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/simulateTransaction).

## Current result and limitations (September 30, 2026)

**This is not a claim that the unchanged workspace or CI is green.**

Clean main `eb461a5` still fails contract code generation before tests execute:
the macro does not recognize the `SorobanVec` alias in the public batch argument.
The argument's tuple shape is not the problem: spelling it `Vec<(Address, i128)>`
with the SDK's normal `Vec` import compiles. This is the existing #1171 scope.

Once compilation is unblocked, `event_version_v1` constructs
`Symbol::new(env, "callora.v1")`, which fails because a dot is not a valid Symbol
character. This is **not** the crate version `0.1.0`; the earlier PR description
misidentified that literal. The original integration auth snapshot also targets
an older API and is not covered by the `--lib` command below.

For isolated validation, only these two compatibility adjustments were applied
in a disposable verification worktree:

1. Spell the SDK vector `Vec` rather than alias `SorobanVec`. Keep the batch
   function, tuple argument shape, validation and transfer implementation intact.
2. Substitute `callora_v1` for the invalid `callora.v1` event marker. Keep every
   event publication and assertion intact.

Neither adjustment is included in this PR. No functions or security checks were
removed to obtain passing results. These temporary adjustments establish native
TTL behavior but do not certify the current unchanged source or live deployment.

Validation was repeated from **fresh, separate build directories** for the
unchanged and compatibility-adjusted trees. A preliminary shared-target run
reused a prior artifact and was discarded. Do not share `CARGO_TARGET_DIR`
between these two worktrees when comparing their results.

With those explicitly described prerequisites applied:

- **8 focused tests pass, 0 failed, 0 ignored**.
- Boundary matrix: 9 views x 7 exact TTL boundaries x 2 storage states = **126 cases**;
  checks return values, unchanged configuration/events/auth requirements and
  same-ledger idempotence.
- **128 deterministic histories x 16 actual view invocations = 2,048 steps**;
  TTL is checked against an independent schedule oracle after every invocation.
- Explicit regressions check survival past initial expiry, external extension
  preservation, instance isolation, init policy, and unchanged pre-init errors.
- **13/13 deliberately incorrect variants are caught by runtime test assertions**:
  remove each of nine individual view refreshes; restore old 1,000/10,000 values;
  drift the limits-module aliases; reduce threshold by one; reduce target by one.
  A compile error does not count as a detected mutant. The restored candidate
  was rerun and passed all eight tests.

Earlier standalone multi-million arithmetic-loop counts are not evidence that
contract execution was correct and are superseded by these host regressions.
The tests are native local tests, not WASM/mainnet execution or independent agents.

## Reproduce once prerequisite fixes are present

From the repository root:

```sh
CARGO_TARGET_DIR="$(mktemp -d)" CARGO_BUILD_JOBS=2 cargo test --locked -p callora-distribute --lib test_ttl -- --test-threads=1
rustfmt --edition 2021 --check contracts/distribute/src/test_ttl.rs
git diff --check
```

To inspect the current blockers, run the same Cargo command on clean main before
applying any compatibility edits. Do not report a failed or modified-baseline run
as a passing full-workspace run. Resolve prerequisites with the maintainers; the
TTL contribution itself must still receive CI approval and human review.
