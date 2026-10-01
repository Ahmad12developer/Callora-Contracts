# Freeze contract

This package contains the `CalloraFreeze` circuit-breaker contract plus an
error-stability regression test that locks the client-facing `ContractError`
discriminant values in place.

## Overview

`CalloraFreeze` provides a pause/unpause surface with `require_auth` on every
state-changing entrypoint. Every state change emits exactly one structured
event. A `get_freeze_status` view returns the current frozen flag, the
persisted reason, and the timestamp of the last `freeze` call in a single
round-trip.

## Auth model

| Entrypoint            | Authorized by                                        |
|-----------------------|------------------------------------------------------|
| `init`                | `admin.require_auth()`                               |
| `freeze`              | `caller.require_auth()` — admin or freeze operator   |
| `unfreeze`            | `caller.require_auth()` — admin only                 |
| `set_freeze_operator` | `caller.require_auth()` — admin only                 |

## Event catalogue

Every state-changing entrypoint emits **exactly one** event. Topic[1] is
always `"callora_v1"` so indexers can filter on the version marker.

### `freeze_initialized`

Emitted once by `init()` when the admin address is stored.

| Index   | Location | Type    | Description                           |
|---------|----------|---------|---------------------------------------|
| topic 0 | topics   | Symbol  | `"freeze_initialized"`                |
| topic 1 | topics   | Symbol  | `"callora_v1"` (version marker)       |
| topic 2 | topics   | Address | `admin` — initial admin address       |
| data    | data     | `()`    | empty                                 |

```json
{
  "topics": ["freeze_initialized", "callora_v1", "GADMIN..."],
  "data": null
}
```

---

### `freeze_set`

Emitted by `freeze()` when the circuit-breaker is activated. The reason
label and ledger timestamp are included in the structured data payload and
are also persisted in contract storage for the `get_freeze_status` view.

| Index      | Location | Type     | Description                                  |
|------------|----------|----------|----------------------------------------------|
| topic 0    | topics   | Symbol   | `"freeze_set"`                               |
| topic 1    | topics   | Symbol   | `"callora_v1"` (version marker)              |
| topic 2    | topics   | Address  | `caller` — admin or freeze operator          |
| `reason`   | data     | Symbol   | opaque reason label supplied by the caller   |
| `frozen_at`| data     | u64      | `env.ledger().timestamp()` at freeze time    |

```json
{
  "topics": ["freeze_set", "callora_v1", "GCALLER..."],
  "data": { "reason": "exploit_risk", "frozen_at": 1700000000 }
}
```

> After this event `is_frozen()` returns `true` and `get_freeze_status()`
> returns `{ frozen: true, reason: Some("exploit_risk"), frozen_at: Some(1700000000) }`.

---

### `freeze_cleared`

Emitted by `unfreeze()` when the circuit-breaker is deactivated. The persisted
reason and timestamp are cleared from storage atomically.

| Index   | Location | Type    | Description                           |
|---------|----------|---------|---------------------------------------|
| topic 0 | topics   | Symbol  | `"freeze_cleared"`                    |
| topic 1 | topics   | Symbol  | `"callora_v1"` (version marker)       |
| topic 2 | topics   | Address | `caller` — admin who unfroze          |
| data    | data     | `()`    | empty                                 |

```json
{
  "topics": ["freeze_cleared", "callora_v1", "GADMIN..."],
  "data": null
}
```

> After this event `is_frozen()` returns `false` and `get_freeze_status()`
> returns `{ frozen: false, reason: None, frozen_at: None }`.

---

### `freeze_operator_set`

Emitted by `set_freeze_operator()` for **both** set and clear operations.
Either `old_operator` or `new_operator` may be `None` (representing "no
operator"). Passing `None` as `operator` clears the role and emits this event
with `new_operator: None`.

| Index          | Location | Type             | Description                                        |
|----------------|----------|------------------|----------------------------------------------------|
| topic 0        | topics   | Symbol           | `"freeze_operator_set"`                            |
| topic 1        | topics   | Symbol           | `"callora_v1"` (version marker)                    |
| topic 2        | topics   | Address          | `caller` — admin who updated the operator          |
| `old_operator` | data     | `Option<Address>`| operator before this call; `None` if none was set  |
| `new_operator` | data     | `Option<Address>`| operator after this call; `None` if role was cleared|

```json
{
  "topics": ["freeze_operator_set", "callora_v1", "GADMIN..."],
  "data": { "old_operator": null, "new_operator": "GOPERATOR..." }
}
```

**Clear example** (`set_freeze_operator(admin, None)`):

```json
{
  "topics": ["freeze_operator_set", "callora_v1", "GADMIN..."],
  "data": { "old_operator": "GOPERATOR...", "new_operator": null }
}
```

---

## View: `get_freeze_status`

Returns a `FreezeStatus` struct with the current freeze state, persisted
reason, and timestamp in a single call — no separate `is_frozen()` +
`get_freeze_reason()` round-trips needed.

```rust
pub struct FreezeStatus {
    pub frozen: bool,
    pub reason: Option<Symbol>,
    pub frozen_at: Option<u64>,
}
```

`reason` and `frozen_at` are `Some` while the contract is frozen and `None`
after `unfreeze()` or before the first `freeze()`.

Returns `FreezeError::NotInitialized` if `init` has not been called.

## Running tests

```bash
cargo test -p callora-freeze
```
