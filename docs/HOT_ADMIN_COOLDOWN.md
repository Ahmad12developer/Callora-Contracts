# Hot Contract — Admin Cool-off (Cooldown) for Critical Actions

Implements [issue #743](https://github.com/CalloraOrg/Callora-Contracts/issues/743):
a cool-off window between admin actions on the `hot` contract to prevent rapid
abuse of an online ("hot") admin key.

Extends [issue #901](https://github.com/CalloraOrg/Callora-Contracts/issues/901):
a focused `pause.rs` module providing an admin-gated circuit-breaker that halts
state-changing operations while allowing read-only views to continue serving.

## Overview

The `hot` contract is the operational control surface where privileged keys
perform time-sensitive, high-impact operations. Because those keys are kept
online, they are the most exposed to compromise. To bound the blast radius of a
leaked or misused key, every **critical action** is rate-limited by a
per-action **cool-off window**: after an action runs, another invocation of the
*same* action is rejected until the window elapses.

The window is measured against the ledger **timestamp** (wall-clock seconds), so
it is independent of block cadence.

## Design

- **Per-action tracking.** Each critical action is identified by a short
  `Symbol` tag (`"pause"`, `"unpause"`, `"rotate"`). The last-execution
  timestamp is stored per tag, so cooling one action never blocks an unrelated
  one.
- **Configurable window.** A single global cooldown (seconds) is held in
  instance storage and bounded by `MIN_COOLDOWN_SECS`..=`MAX_COOLDOWN_SECS`
  (1 second .. 30 days). The default at `init` is `DEFAULT_COOLDOWN_SECS`
  (1 hour).
- **Overflow-safe.** All arithmetic uses checked/saturating operations. There
  are no `unwrap()` calls on production paths.
- **Auth at the edge.** `require_auth` and the admin check run in the contract
  entrypoints; `admin.rs` is pure cool-off bookkeeping and is unit-tested in
  isolation.
- **Pause circuit-breaker.** The `pause.rs` module contains the focused pause /
  unpause logic. It guards against redundant state transitions — attempting to
  `pause` an already-paused contract returns `AlreadyPaused`; attempting to
  `unpause` a non-paused contract returns `NotPaused` — before the cool-off
  guard is evaluated.

## Module structure

| Module | Responsibility |
|--------|---------------|
| `lib.rs` | Contract entrypoints, auth, and dispatch |
| `admin.rs` | Cool-off bookkeeping (per-action timestamps, window validation) |
| `pause.rs` | Circuit-breaker state (`do_pause`, `do_unpause`, `is_paused`) |
| `events.rs` | Event topic `Symbol` constructors |
| `errors.rs` | Stable numeric error codes |

## Entrypoints (API)

| Function | Auth | Cool-off tag | Description |
|----------|------|--------------|-------------|
| `init(admin, signer, cooldown_secs: Option<u64>)` | `admin` | — | One-time setup. `None` adopts the 1-hour default. |
| `set_cooldown(caller, secs)` | admin | — | Update the global window (validated). |
| `get_cooldown() -> u64` | — (view) | — | Current window in seconds. |
| `cooldown_remaining(action) -> u64` | — (view) | — | Seconds until `action` is available (0 = now). |
| `is_ready(action) -> bool` | — (view) | — | Whether `action` may run now. |
| `pause(caller)` | admin | **none** | Activate the circuit-breaker. Always available immediately. Returns `AlreadyPaused` when already paused. |
| `unpause(caller)` | admin | `"unpause"` | Deactivate the circuit-breaker. Returns `NotPaused` when not paused. |
| `is_paused() -> bool` | — (view) | — | Current pause state. |
| `rotate_signer(caller, new_signer)` | admin | `"rotate"` | Replace the hot signer. |
| `set_admin(caller, new_admin)` | admin | — | Nominate a new admin (two-step). |
| `accept_admin(caller)` | pending admin | — | Complete the two-step transfer. |
| `get_admin() -> Address` | — (view) | — | Current admin. |
| `get_pending_admin() -> Option<Address>` | — (view) | — | Pending nominee, if any. |
| `get_signer() -> Address` | — (view) | — | Current hot signer. |

Every state-changing entrypoint calls `require_auth` on its `caller`.

## Pause / unpause semantics (pause.rs)

The `pause.rs` module exposes three functions:

```rust
pub fn is_paused(env: &Env) -> bool
pub fn do_pause(env: &Env, caller: &Address) -> Result<(), HotError>
pub fn do_unpause(env: &Env, caller: &Address, action: &Symbol) -> Result<(), HotError>
```

### Asymmetric cooldown

`do_pause` is **exempt from cooldown gating**. `do_unpause` **retains** its
cooldown. This asymmetry is deliberate:

- A circuit-breaker must be available **instantly**. If an attacker triggers an
  `unpause` (or the admin toggles during an incident), a pending `"pause"`
  cooldown window would keep the contract live — defeating the circuit-breaker's
  entire purpose.
- Re-opening the contract (`unpause`) is a high-impact, deliberate action that
  benefits from rate-limiting to prevent oscillation attacks.
- Signer rotation also retains its cooldown (unchanged).

Evaluation order inside `do_pause`:

1. **State guard** — `AlreadyPaused` is checked first.
2. **State update** — the `Paused` flag is set in instance storage.
3. **Event emission** — the `paused` topic is published.

No cooldown timestamp is written for `pause`.

Evaluation order inside `do_unpause`:

1. **State guard** — `NotPaused` is checked first.
2. **Cool-off guard** — `admin::guard` enforces the rate-limit window and
   records `last_run_ts = now`.
3. **State update** — the `Paused` flag is cleared.
4. **Event emission** — the `unpaused` topic is published.

## Cool-off semantics

For a given action tag:

```
ready_at   = last_run_ts + cooldown           (saturating; 0 if never run)
remaining  = max(0, ready_at - now)
is_ready   = remaining == 0
```

`guard()` enforces `is_ready` and, on success, stamps `last_run_ts = now`,
arming a fresh window. A blocked call returns `HotError::CooldownActive` and
makes **no** state change.

Reducing the window via `set_cooldown` takes effect immediately for the *next*
readiness check (it is recomputed from the stored `last_run_ts`), allowing an
admin to safely shorten the cool-off in a genuine emergency.

## Error codes

| Code | Variant | Meaning |
|------|---------|---------|
| 1 | `NotInitialized` | Contract not initialized |
| 2 | `AlreadyInitialized` | `init` called twice |
| 3 | `Unauthorized` | Caller is not the required admin/pending admin |
| 4 | `CooldownActive` | Action is still inside its cool-off window |
| 5 | `InvalidCooldown` | Proposed window outside `[1s, 30d]` |
| 6 | `NoPendingAdmin` | No admin transfer pending |
| 7 | `Overflow` | Arithmetic overflow detected |
| 8 | `AlreadyPaused` | `pause` called when contract is already paused |
| 9 | `NotPaused` | `unpause` called when contract is not paused |

## Events

| Topic | Data | When |
|-------|------|------|
| `init` | `cooldown: u64` | On initialization (topic includes `admin`). |
| `cooldown_set` | `secs: u64` | On `set_cooldown` (topic includes `caller`). |
| `action` | `tag: Symbol` | On each successful guarded action (topic includes `caller`). |
| `paused` | `()` | On `pause` (topic includes `caller`). Dedicated topic from `pause.rs`. |
| `unpaused` | `()` | On `unpause` (topic includes `caller`). Dedicated topic from `pause.rs`. |
| `admin_nominated` | `new_admin: Address` | On `set_admin`. |
| `admin_accepted` | `new_admin: Address` | On `accept_admin`. |

## Security notes

- The cool-off caps the *rate* of critical actions; it is a defense-in-depth
  layer complementing the two-step admin rotation, not a replacement for key
  hygiene.
- The `AlreadyPaused` / `NotPaused` guards prevent silent no-ops: an operator
  trying to double-pause gets an explicit error rather than a successful
  transaction that changed nothing.
- **Distinct action tags are independently cooled** so that an emergency `pause`
  is never blocked by a recent `rotate`, and vice versa. `pause` carries no
  cooldown tag at all, so it is always instantly available regardless of any
  prior admin action.
- All windows are bounded, so a configuration mistake cannot brick critical
  actions for longer than `MAX_COOLDOWN_SECS` (30 days).
- `is_paused` is a pure read and does not require initialization; it returns
  `false` when the storage key is absent (pre-`init`).
