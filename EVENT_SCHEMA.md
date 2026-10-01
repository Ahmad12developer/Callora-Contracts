# Event Schema

Events emitted by all Callora contracts for indexers, frontends, and auditors.
All topic/data types refer to Soroban/Stellar XDR values.

## Change Note (2026-04)

The `workspace-members-dedup` hardening patch does not introduce event additions, removals, or payload shape changes.

## Change Note (2026-09)

**Two-step admin rotation events (Issue #1163).**
`set_admin()` now publishes `admin_transfer_started` only. The explicit
`admin_changed` event moved to `accept_admin()` / `claim_admin()`, where it is
emitted with `(previous_admin, new_admin)` immediately before
`admin_transfer_completed`. A nomination that is cancelled therefore leaves no
`admin_changed` event behind, so indexers keyed on that topic no longer record
an admin change that never happened. Applies to `callora-revenue-pool` and
`callora-distribute`.

## Change Note (2026-06)

**Event topic centralization (PR: task/event-symbol-catalog).**
All inline `Symbol::new(&env, "...")` event topic literals have been extracted from
`lib.rs` call sites into dedicated `src/events.rs` modules per crate:

- [`contracts/vault/src/events.rs`](contracts/vault/src/events.rs) — 23 topics
- [`contracts/settlement/src/events.rs`](contracts/settlement/src/events.rs) — 8 topics
- [`contracts/revenue_pool/src/events.rs`](contracts/revenue_pool/src/events.rs) — 12 topics

Each module exports one `pub fn event_*(&env) -> Symbol` function per topic and includes
a `#[cfg(test)]` snapshot block asserting byte-level identity to the original literal.
No topic strings were renamed; this refactor is a zero-semantic-change migration.

## Change Note (2026-09) — Issue #1118: version topic added to six vault events

Six vault events previously published without the `"callora_v1"` version marker at
topic[1]. Indexers filtering on the version topic were silently dropping these
fund-moving events. All six now carry `"callora_v1"` at topic[1]:

| Event | Function | Old topic count | New topic count |
|-------|----------|-----------------|-----------------|
| `withdraw` | `withdraw()` | 2 | 3 |
| `withdraw_to` | `withdraw_to()` | 3 | 4 |
| `distribute` | `distribute()` | 2 | 3 |
| `rescue_funds` | `admin_rescue()` | 3 | 4 |
| `reserve_cap_set` | `set_reserve_cap()` | 3 | 4 |
| `request_id_pruned` | `prune_processed_requests()` | 2 | 3 |

The version symbol string is `"callora_v1"` (underscore, not dot — Soroban `Symbol`
only allows `a-zA-Z0-9_`; the previous `"callora.v1"` string was invalid and would
panic when passed through the Soroban host's XDR layer).

**Breaking change for indexers:** Any consumer matching on topic count or positional
topic index for these six events must update its filters. The `"callora_v1"` symbol
is always at topic[1]; the subject address (owner/caller/token) shifts to topic[2]+.


## Contract: Callora Vault

### `init`

Emitted once when the vault is initialized.

| Index   | Location | Type    | Description         |
|---------|----------|---------|---------------------|
| topic 0 | topics   | Symbol  | `"init"`            |
| topic 1 | topics   | Address | vault owner         |
| data    | data     | i128    | initial balance     |

```json
{
  "topics": ["init", "GOWNER..."],
  "data": 1000000
}
```

---

### `deposit`

Emitted when a depositor increases the vault balance.

| Index   | Location | Type         | Description                   |
|---------|----------|--------------|-------------------------------|
| topic 0 | topics   | Symbol       | `"deposit"`                   |
| topic 1 | topics   | Address      | caller (depositor)            |
| data    | data     | (i128, i128) | (amount, new_balance)         |

```json
{
  "topics": ["deposit", "GDEPOSITOR..."],
  "data": [500000, 1500000]
}
```

---

### `deduct`

Emitted on each deduction â€” once per `deduct()` call and once per item in `batch_deduct()`.

| Index   | Location | Type         | Description                                    |
|---------|----------|--------------|------------------------------------------------|
| topic 0 | topics   | Symbol       | `"deduct"`                                     |
| topic 1 | topics   | Address      | caller                                         |
| topic 2 | topics   | Symbol       | `request_id` (empty Symbol if not provided)    |
| data    | data     | (i128, i128) | (amount, new_balance)                          |

```json
{
  "topics": ["deduct", "GCALLER...", "req_abc123"],
  "data": [100000, 900000]
}
```

**`request_id` encoding (indexer contract):**

- **Topic is always present**: the vault always emits **exactly 3 topics** for `deduct`.
- **No optional topic**: Soroban events do not carry an `Option` topic value; instead the vault uses a **sentinel**.
- **Sentinel for â€œno request_idâ€**: when the input `request_id` is `None`, topic 2 is `Symbol("")` (an empty symbol).
- **Indexer rule**: treat `Symbol("")` as â€œno request_id providedâ€.
- **Ambiguity note**: `Some(Symbol(""))` is indistinguishable from `None` on-chain. Clients **SHOULD NOT** intentionally pass an empty symbol as a real request id.

**Precondition (Issue #263):** `deduct` / `batch_deduct` require a settlement
address to be configured via `set_settlement`. If the settlement address is
not set, the call panics with `"settlement address not set"` **before** any
`deduct` event is emitted â€” indexers will therefore never observe a `deduct`
event for a call that lacked a configured settlement destination.

**Idempotency guard (Issue #249):** when `request_id` is `Some(Symbol)`, the
value is single-use across successful `deduct` and `batch_deduct` calls.
Reusing a previously accepted value, or repeating the same value twice inside
one batch, panics with `"duplicate request_id"` before any balance update,
transfer, or `deduct` event is emitted.

---

### `withdraw`

Emitted when the vault owner withdraws to their own address.

| Index   | Location | Type         | Description                         |
|---------|----------|--------------|-------------------------------------|
| topic 0 | topics   | Symbol       | `"withdraw"`                        |
| topic 1 | topics   | Symbol       | `"callora_v1"` (version marker)     |
| topic 2 | topics   | Address      | vault owner                         |
| data    | data     | (i128, i128) | (amount, new_balance)               |

```json
{
  "topics": ["withdraw", "callora_v1", "GOWNER..."],
  "data": [200000, 700000]
}
```

---

### `withdraw_to`

Emitted when the vault owner withdraws to a designated recipient.

| Index   | Location | Type         | Description                         |
|---------|----------|--------------|-------------------------------------|
| topic 0 | topics   | Symbol       | `"withdraw_to"`                     |
| topic 1 | topics   | Symbol       | `"callora_v1"` (version marker)     |
| topic 2 | topics   | Address      | vault owner                         |
| topic 3 | topics   | Address      | recipient                           |
| data    | data     | (i128, i128) | (amount, new_balance)               |

```json
{
  "topics": ["withdraw_to", "callora_v1", "GOWNER...", "GRECIPIENT..."],
  "data": [150000, 550000]
}
```

---

### `vault_paused`

Emitted when the vault is paused by the admin or owner.

| Index   | Location | Type    | Description          |
|---------|----------|---------|----------------------|
| topic 0 | topics   | Symbol  | `"vault_paused"`     |
| topic 1 | topics   | Address | caller (admin/owner) |
| data    | data     | ()      | empty                |

```json
{
  "topics": ["vault_paused", "GADMIN..."],
  "data": null
}
```

---

### `vault_unpaused`

Emitted when the vault is unpaused by the admin or owner.

| Index   | Location | Type    | Description          |
|---------|----------|---------|----------------------|
| topic 0 | topics   | Symbol  | `"vault_unpaused"`   |
| topic 1 | topics   | Address | caller (admin/owner) |
| data    | data     | ()      | empty                |

```json
{
  "topics": ["vault_unpaused", "GADMIN..."],
  "data": null
}
```

---

### `ownership_nominated`

Emitted when the owner starts a two-step ownership transfer.

| Index   | Location | Type    | Description   |
|---------|----------|---------|---------------|
| topic 0 | topics   | Symbol  | `"ownership_nominated"` |
| topic 1 | topics   | Address | current owner |
| topic 2 | topics   | Address | nominee       |
| data    | data     | ()      | empty         |

```json
{
  "topics": ["ownership_nominated", "GOWNER...", "GNOMINEE..."],
  "data": null
}
```

---

### `ownership_accepted`

Emitted when the nominee accepts ownership.

| Index   | Location | Type    | Description   |
|---------|----------|---------|---------------|
| topic 0 | topics   | Symbol  | `"ownership_accepted"` |
| topic 1 | topics   | Address | old owner     |
| topic 2 | topics   | Address | new owner     |
| data    | data     | ()      | empty         |

```json
{
  "topics": ["ownership_accepted", "GOWNER...", "GNEWOWNER..."],
  "data": null
}
```

---

### `admin_nominated`

Emitted when the admin starts a two-step admin transfer.

| Index   | Location | Type    | Description   |
|---------|----------|---------|---------------|
| topic 0 | topics   | Symbol  | `"admin_nominated"` |
| topic 1 | topics   | Address | current admin |
| topic 2 | topics   | Address | nominee       |
| data    | data     | ()      | empty         |

```json
{
  "topics": ["admin_nominated", "GADMIN...", "GNOMINEE..."],
  "data": null
}
```

---

### `admin_accepted`

- **OwnershipTransfer**: not present in current vault; would list old_owner, new_owner.

---

### `vault_paused`

Emitted when the vault circuit-breaker is activated by admin or owner.

| Field   | Location | Type    | Description                                      |
|---------|----------|---------|--------------------------------------------------|
| topic 0 | topics   | Symbol  | `"vault_paused"`                                 |
| topic 1 | topics   | Address | `caller` â€” admin or owner who triggered pause   |
| data    | data     | ()      | empty                                            |

**Indexer Note:** After this event is emitted, `is_paused()` view function returns `true`.
The following operations are blocked until unpause: `deposit()`, `deduct()`, `batch_deduct()`.

---

### `vault_unpaused`

Emitted when the vault circuit-breaker is deactivated by admin or owner.

| Field   | Location | Type    | Description                                      |
|---------|----------|---------|--------------------------------------------------|
| topic 0 | topics   | Symbol  | `"vault_unpaused"`                               |
| topic 1 | topics   | Address | `caller` â€” admin or owner who triggered unpause |
| data    | data     | ()      | empty                                            |

**Indexer Note:** After this event is emitted, `is_paused()` view function returns `false`.
All vault operations are restored: `deposit()`, `deduct()`, `batch_deduct()`.

---

### View Function: `is_paused()`

The vault exposes a read-only view function for off-chain systems to query the current pause state.

**Signature:** `pub fn is_paused(env: Env) -> bool`

**Return Value:**
- `true` â€” Vault is currently paused (circuit-breaker active)
- `false` â€” Vault is operational (normal state)

**Safety Guarantees:**
- **Read-only**: No state mutation or side effects
- **Deterministic**: Identical state always produces identical output
- **Non-panicking**: Never panics, even before initialization
- **Safe default**: Returns `false` when pause state is unset

**Indexer Usage:**
```javascript
// Check if vault is paused before processing transactions
const isPaused = await vault.isPaused();
if (isPaused) {
  // Vault is paused - deposits and deductions are blocked
  // Only admin/owner operations like withdraw() are allowed
} else {
  // Vault is operational - all functions available
}
```

**Consistency with Events:**
- `vault_paused` event emitted â†’ `is_paused()` returns `true`
- `vault_unpaused` event emitted â†’ `is_paused()` returns `false`

Indexers should use `is_paused()` for current state queries and subscribe to
`vault_paused`/`vault_unpaused` events for state change notifications.

---

### `set_revenue_pool`

Emitted when the admin sets a revenue pool address.

| Index   | Location | Type    | Description        |
|---------|-----------|---------|--------------------|
| topic 0 | topics   | Symbol  | `"set_revenue_pool"` |
| topic 1 | topics   | Address | caller (admin)     |
| data    | data     | Address | new revenue pool   |

```json
{
  "topics": ["set_revenue_pool", "GADMIN..."],
  "data": "GPOOL..."
}
```

---

### `clear_revenue_pool`

Emitted when the admin clears the revenue pool address.

| Index   | Location | Type    | Description    |
|---------|----------|---------|----------------|
| topic 0 | topics   | Symbol  | `"clear_revenue_pool"` |
| topic 1 | topics   | Address | caller (admin) |
| data    | data     | ()      | empty          |

```json
{
  "topics": ["clear_revenue_pool", "GADMIN..."],
  "data": null
}
```

---

### `metadata_set`

Emitted when offering metadata is stored for the first time.

| Index   | Location | Type    | Description               |
|---------|----------|---------|---------------------------|
| topic 0 | topics   | Symbol  | `"metadata_set"`          |
| topic 1 | topics   | String  | offering_id               |
| topic 2 | topics   | Address | caller (owner)            |
| data    | data     | String  | metadata (IPFS CID / URI) |

```json
{
  "topics": ["metadata_set", "offering-001", "GOWNER..."],
  "data": "ipfs://bafybeigdyrzt..."
}
```

---

### `metadata_updated`

Emitted when existing offering metadata is replaced.

| Index   | Location | Type             | Description                    |
|---------|----------|------------------|--------------------------------|
| topic 0 | topics   | Symbol           | `"metadata_updated"`           |
| topic 1 | topics   | String           | offering_id                    |
| topic 2 | topics   | Address          | caller (owner)                 |
| data    | data     | (String, String) | (old_metadata, new_metadata)   |

```json
{
  "topics": ["metadata_updated", "offering-001", "GOWNER..."],
  "data": ["ipfs://old...", "ipfs://new..."]
}
```

---

### `metadata_removed`

Emitted when the owner deletes a stale offering's metadata from instance storage.

| Index   | Location | Type    | Description         |
|---------|----------|---------|---------------------|
| topic 0 | topics   | Symbol  | `"metadata_removed"` |
| topic 1 | topics   | String  | offering_id         |
| topic 2 | topics   | Address | caller (owner)      |
| data    | data     | ()      | empty               |

```json
{
  "topics": ["metadata_removed", "offering-001", "GOWNER..."],
  "data": null
}
```

**Indexer note:** After this event, `get_metadata(offering_id)` returns `None`.
The call is idempotent — removing a key that was never set (or was already removed)
emits the event and returns `Ok(())` without error.

---

### `set_authorized_caller`

Emitted when the owner updates the authorized caller address.

| Index   | Location | Type                              | Description                                  |
|---------|----------|-----------------------------------|----------------------------------------------|
| topic 0 | topics   | Symbol                            | `"set_authorized_caller"`                   |
| topic 1 | topics   | Address                           | vault owner                                  |
| data    | data     | (Option<Address>, Option<Address>) | (old_authorized_caller, new_authorized_caller) |

```json
{
  "topics": ["set_authorized_caller", "GOWNER..."],
  "data": [null, "GCALLER..."]
}
```

---

### `tl_window_changed`

Emitted when the admin updates the timelock window length via `set_timelock_window()`.

The payload records both the **previous** and the **new** window length so auditors
can detect shortening of the escape-hatch delay without replaying storage history.

| Index   | Location | Type       | Description                                         |
|---------|----------|------------|-----------------------------------------------------|
| topic 0 | topics   | Symbol     | `"tl_window_changed"`                               |
| topic 1 | topics   | Symbol     | `"callora.v1"` (version marker)                     |
| topic 2 | topics   | Address    | `caller` — admin who changed the window             |
| data    | data     | (u64, u64) | `(old_window_seconds, new_window_seconds)`          |

**Payload order:** `data[0]` is always the **old** (previous) value; `data[1]` is the
**new** (just-committed) value. Auditors monitoring for shortening attacks should
assert `data[1] >= data[0]`.

Valid window range: [`MIN_TIMELOCK_SECONDS` (3 600 s / 1 h), `MAX_TIMELOCK_SECONDS` (2 592 000 s / 30 d)].
Default at deployment: `DEFAULT_TIMELOCK_SECONDS` = 172 800 s (48 h).

```json
{
  "topics": ["tl_window_changed", "callora.v1", "GADMIN..."],
  "data": [172800, 3600]
}
```

> **Security note:** a `tl_window_changed` event where `data[1] < data[0]` means
> the admin shortened the escape-hatch delay. Indexers and monitoring systems
> SHOULD alert on this condition because a shortened window reduces the time
> available for community reaction to a malicious admin action.

---

---

### `admin_nominated`

Emitted when the current admin nominates a successor.

| Field   | Location | Type   | Description   |
|---------|----------|--------|-----------------------|
| topic 0 | topics   | Symbol | `"admin_nominated"` |
| topic 1 | topics   | Address| current admin |
| topic 2 | topics   | Address| nominee       |
| data    | data     | ()     | empty         |

---

### `admin_accepted`

Emitted when the nominee accepts the admin role.

| Field   | Location | Type   | Description   |
|---------|----------|--------|-----------------------|
| topic 0 | topics   | Symbol | `"admin_accepted"` |
| topic 1 | topics   | Address| old admin     |
| topic 2 | topics   | Address| new admin     |
| data    | data     | ()     | empty         |

---

### `distribute` (vault)

Emitted when the admin distributes on-ledger USDC surplus to a recipient via
`distribute()`. Updated in Issue #1118 to include the version marker at topic[1].

| Index   | Location | Type    | Description                          |
|---------|----------|---------|--------------------------------------|
| topic 0 | topics   | Symbol  | `"distribute"`                       |
| topic 1 | topics   | Symbol  | `"callora_v1"` (version marker)      |
| topic 2 | topics   | Address | `to` — recipient address             |
| data    | data     | i128    | `amount` in USDC micro-units         |

```json
{
  "topics": ["distribute", "callora_v1", "GRECIPIENT..."],
  "data": 1000000
}
```

---

### `rescue_funds`

Emitted when the admin rescues tokens from the vault via `admin_rescue()`.
Updated in Issue #1118 to include the version marker at topic[1].

| Index   | Location | Type    | Description                               |
|---------|----------|---------|-------------------------------------------|
| topic 0 | topics   | Symbol  | `"rescue_funds"`                          |
| topic 1 | topics   | Symbol  | `"callora_v1"` (version marker)           |
| topic 2 | topics   | Address | `caller` — admin address                  |
| topic 3 | topics   | Address | `token_address` — token being rescued     |
| data    | data     | (Address, i128) | `(to, amount)` — destination and amount |

```json
{
  "topics": ["rescue_funds", "callora_v1", "GADMIN...", "GTOKEN..."],
  "data": ["GRECIPIENT...", 3000000]
}
```

---

### `reserve_cap_set`

Emitted when the owner sets or updates the deposit reserve cap for a token via
`set_reserve_cap()`. Updated in Issue #1118 to include the version marker at topic[1].

| Index   | Location | Type         | Description                             |
|---------|----------|--------------|-----------------------------------------|
| topic 0 | topics   | Symbol       | `"reserve_cap_set"`                     |
| topic 1 | topics   | Symbol       | `"callora_v1"` (version marker)         |
| topic 2 | topics   | Address      | `caller` — owner address                |
| topic 3 | topics   | Address      | `token` — token the cap applies to      |
| data    | data     | (Option\<i128\>, i128) | `(prev_cap, new_cap)` — previous cap (None if unset) and new cap |

```json
{
  "topics": ["reserve_cap_set", "callora_v1", "GOWNER...", "GTOKEN..."],
  "data": [null, 999999]
}
```

---

### `request_id_pruned`

Emitted once per successfully removed idempotency marker when the owner calls
`prune_processed_requests()`. Updated in Issue #1118 to include the version marker
at topic[1].

| Index   | Location | Type    | Description                              |
|---------|----------|---------|------------------------------------------|
| topic 0 | topics   | Symbol  | `"request_id_pruned"`                    |
| topic 1 | topics   | Symbol  | `"callora_v1"` (version marker)          |
| topic 2 | topics   | Symbol  | `id` — the pruned request ID             |
| data    | data     | ()      | empty                                    |

```json
{
  "topics": ["request_id_pruned", "callora_v1", "req_abc123"],
  "data": null
}
```

**Indexer note:** One `request_id_pruned` event is emitted per marker that was
actually found and removed. IDs not present in storage are silently skipped (no
event).

---

## Contract: `callora-revenue-pool` (v0.0.1)

The revenue pool receives USDC forwarded by the vault on every `deduct` / `batch_deduct`
call and lets the admin distribute those funds to developers.

### `init`

Emitted once when the revenue pool is initialized.

| Index   | Location | Type    | Description                          |
|---------|----------|---------|--------------------------------------|
| topic 0 | topics   | Symbol  | `"init"`                             |
| topic 1 | topics   | Address | `admin` â€” initial admin address      |
| data    | data     | Address | `usdc_token` â€” token contract address|

```json
{
  "topics": ["init", "GADMIN..."],
  "data": "GUSDC_TOKEN..."
}
```

> **Security note:** `usdc_token` is immutable after `init`. Verify it matches the
> canonical Stellar USDC contract before deployment.

---

### `admin_transfer_started`

Emitted when the current admin nominates a successor (step 1 of 2).

| Index   | Location | Type    | Description                              |
|---------|----------|---------|------------------------------------------|
| topic 0 | topics   | Symbol  | `"admin_transfer_started"`               |
| topic 1 | topics   | Address | `current_admin` â€” the nominator          |
| data    | data     | Address | `pending_admin` â€” nominee who must accept|

```json
{
  "topics": ["admin_transfer_started", "GCURRENT_ADMIN..."],
  "data": "GPENDING_ADMIN..."
}
```

> This is the only event published by `set_admin()`; no `admin_changed` event
> accompanies a nomination (Issue #1163). Indexers should treat the pool as
> still under `current_admin` control until `admin_changed` /
> `admin_transfer_completed` is observed.

---

### `admin_changed`

Emitted when the nominee accepts the admin role (step 2 of 2), after the admin
slot has been updated and immediately before `admin_transfer_completed`.
`set_admin()` does **not** emit this event — nomination publishes only
`admin_transfer_started`, so a transfer that is cancelled never produces a
change event (Issue #1163).

| Index   | Location | Type               | Description                            |
|---------|----------|--------------------|----------------------------------------|
| topic 0 | topics   | Symbol             | `"admin_changed"`                      |
| topic 1 | topics   | Address            | `previous_admin` — outgoing admin      |
| data    | data     | (Address, Address) | `(previous_admin, new_admin)`          |

```json
{
  "topics": ["admin_changed", "GPREVIOUS_ADMIN..."],
  "data": ["GPREVIOUS_ADMIN...", "GNEW_ADMIN..."]
}
```

> Topic 1 and `data[0]` are the admin that held the role immediately before
> this event; `data[1]` is the incoming admin. One event therefore records the
> complete handover.

---

### `admin_transfer_completed`

Emitted when the nominee accepts the admin role (step 2 of 2).

| Index   | Location | Type    | Description                        |
|---------|-----------|---------|------------------------------------|
| topic 0 | topics   | Symbol  | `"admin_transfer_completed"`       |
| topic 1 | topics   | Address | `new_admin` â€” the accepted admin   |
| data    | data     | ()      | empty                              |

```json
{
  "topics": ["admin_transfer_completed", "GNEW_ADMIN..."],
  "data": null
}
```

> After this event, only `new_admin` can call `distribute`, `batch_distribute`,
> `receive_payment`, and `set_admin`.

---

### `pause_guardian_set`

Emitted when the admin sets or replaces the emergency pause guardian.

| Index   | Location | Type    | Description                              |
|---------|----------|---------|------------------------------------------|
| topic 0 | topics   | Symbol  | `"pause_guardian_set"`                   |
| topic 1 | topics   | Address | `caller` — current admin                 |
| data    | data     | Address | `guardian` — address allowed to pause    |

```json
{
  "topics": ["pause_guardian_set", "GADMIN..."],
  "data": "GGUARDIAN..."
}
```

---

### `pause_guardian_cleared`

Emitted when the admin clears the emergency pause guardian role.

| Index   | Location | Type    | Description                              |
|---------|----------|---------|------------------------------------------|
| topic 0 | topics   | Symbol  | `"pause_guardian_cleared"`               |
| topic 1 | topics   | Address | `caller` — current admin                 |
| data    | data     | Address | previous guardian address                |

```json
{
  "topics": ["pause_guardian_cleared", "GADMIN..."],
  "data": "GOLD_GUARDIAN..."
}
```

---

### `receive_payment`

Emitted when the admin logs an inbound payment from the vault.

> **Note:** This is an **event-only helper** â€” it does not move tokens. USDC
> arrives via a direct token transfer from the vault. Call `receive_payment` to
> emit this event for indexer alignment.

| Index   | Location | Type         | Description                                     |
|---------|-----------|--------------|-------------------------------------------------|
| topic 0 | topics   | Symbol       | `"receive_payment"`                             |
| topic 1 | topics   | Address      | `caller` â€” typically admin                      |
| data    | data     | (i128, bool) | `(amount, from_vault)` â€” amount in stroops; `from_vault=true` when source is the vault |

```json
{
  "topics": ["receive_payment", "GADMIN..."],
  "data": [5000000, true]
}
```

**Example â€” manual top-up (not from vault):**

```json
{
  "topics": ["receive_payment", "GADMIN..."],
  "data": [1000000, false]
}
```

> Indexers tracking total inflows should subscribe to this event and filter on
> `from_vault` to distinguish vault-originated payments from manual top-ups.

---

### `distribute`

Emitted when the admin distributes USDC to a single developer.

| Index   | Location | Type    | Description              |
|---------|----------|---------|--------------------------|
| topic 0 | topics   | Symbol  | `"distribute"`           |
| topic 1 | topics   | Address | `to` â€” developer address |
| data    | data     | i128    | `amount` in stroops      |

```json
{
  "topics": ["distribute", "GDEVELOPER..."],
  "data": 2500000
}
```

> A `distribute` event guarantees the token transfer succeeded â€” the USDC has
> left the pool contract and arrived at `to`.

---

### `set_max_distribute`

Emitted when the admin updates the per-leg distribution cap.

| Index   | Location | Type    | Description                    |
|---------|----------|---------|--------------------------------|
| topic 0 | topics   | Symbol  | `"set_max_distribute"`        |
| topic 1 | topics   | Address | admin address                  |
| data    | data     | (i128, i128) | `(old_max, new_max)`       |

```json
{
  "topics": ["set_max_distribute", "GADMIN..."],
  "data": [9223372036854775807, 500]
}
```

---

### `batch_distribute`

Emitted **once per payment** during a `batch_distribute()` call. If a batch has
three payments, three `batch_distribute` events are emitted in order.

| Index   | Location | Type    | Description              |
|---------|----------|---------|--------------------------|
| topic 0 | topics   | Symbol  | `"batch_distribute"`     |
| topic 1 | topics   | Address | `to` â€” developer address |
| data    | data     | i128    | `amount` in stroops      |

```json
{
  "topics": ["batch_distribute", "GDEVELOPER_A..."],
  "data": 1000000
}
```

**Example â€” 3-payment batch produces 3 events:**

```json
[
  { "topics": ["batch_distribute", "GDEV_A..."], "data": 1000000 },
  { "topics": ["batch_distribute", "GDEV_B..."], "data": 2000000 },
  { "topics": ["batch_distribute", "GDEV_C..."], "data": 500000  }
]
```

> `batch_distribute` is atomic â€” either all payments succeed and all events are
> emitted, or none are. Indexers can verify atomicity by checking that all events
> share the same ledger sequence number.

---



---

### `pause_set`

Emitted by both `pause()` (data = `true`) and `unpause()` (data = `false`) to signal
a change in the pool's pause state. Only the admin may trigger either function.

| Index   | Location | Type    | Description                                      |
|---------|----------|---------|--------------------------------------------------|
| topic 0 | topics   | Symbol  | `"pause_set"`                                    |
| topic 1 | topics   | Address | `caller` -- the admin who called pause/unpause   |
| data    | data     | bool    | `true` = pool is now paused; `false` = unpaused  |

```json
{ "topics": ["pause_set", "GADMIN..."], "data": true }
```

> While paused, `distribute` and `batch_distribute` are blocked.
> Admin rotation (`set_admin`, `claim_admin`) remains available.

---

### `admin_cancelled`

Emitted when the current admin cancels a pending two-step admin transfer via
`cancel_admin_transfer()`. Both the current and the pending admin are recorded as topics
so indexers can link the cancellation to the in-flight handover without a data decode.

| Index   | Location | Type    | Description                                        |
|---------|-----------|---------|----------------------------------------------------|
| topic 0 | topics    | Symbol  | `"admin_cancelled"`                                |
| topic 1 | topics    | Address | `current_admin` -- admin who issued the cancel     |
| topic 2 | topics    | Address | `pending_admin` -- nominee whose claim is revoked  |
| data    | data      | ()      | empty                                              |

```json
{
  "topics": ["admin_cancelled", "GCURRENT_ADMIN...", "GPENDING_ADMIN..."],
  "data": null
}
```

> After this event `get_pending_admin()` returns `None`. The current admin remains
> unchanged and may initiate a new transfer at any time.

---

### `upgrade_started`, `upgrade_completed`, `upgraded`

A successful `upgrade()` call publishes three events in this order:

1. `upgrade_started` — published *before* the host swaps the contract WASM.
2. `upgrade_completed` — published *after* the WASM swap and the
   `ContractVersion` storage write.
3. `upgraded` — legacy single-event shape retained for backwards
   compatibility with off-chain subscribers written against the
   pre-lifecycle schema. New indexers should subscribe to the structured
   pair above.

Receipt of `upgrade_started` without a matching `upgrade_completed` at the
same `(ledger, timestamp)` means the host trapped between the two emits;
the WASM swap and `ContractVersion` write were rolled back.

#### `upgrade_started` and `upgrade_completed`

| Index   | Location | Type           | Description                                                  |
|---------|----------|----------------|--------------------------------------------------------------|
| topic 0 | topics   | Symbol         | `"upgrade_started"` or `"upgrade_completed"`                 |
| topic 1 | topics   | Address        | `caller` -- the address that authorized `upgrade()`          |
| data    | data     | `UpgradeEvent` | structured payload (see below)                               |

`UpgradeEvent` fields:

| Field           | Type                  | Description                                                                 |
|-----------------|-----------------------|-----------------------------------------------------------------------------|
| `caller`        | `Address`             | Same as topic 1; included in data for indexers that store the payload only. |
| `previous_wasm` | `Option<BytesN<32>>`  | Hash recorded by the prior `upgrade()`. `None` on the first upgrade.        |
| `new_wasm`      | `BytesN<32>`          | Hash being deployed by this call.                                           |
| `ledger`        | `u32`                 | `env.ledger().sequence()` captured before the WASM swap.                    |
| `timestamp`     | `u64`                 | `env.ledger().timestamp()` captured before the WASM swap.                   |

```json
{
  "topics": ["upgrade_completed", "GADMIN..."],
  "data": {
    "caller": "GADMIN...",
    "previous_wasm": "a1b2c3d4...",
    "new_wasm": "f0e1d2c3...",
    "ledger": 1234567,
    "timestamp": 1700000000
  }
}
```

### `upgraded` (legacy)

| Index   | Location | Type       | Description                                       |
|---------|----------|------------|---------------------------------------------------|
| topic 0 | topics   | Symbol     | `"upgraded"`                                      |
| topic 1 | topics   | Address    | `caller` -- admin who executed the upgrade        |
| data    | data     | BytesN<32> | `new_wasm_hash` -- hash of the deployed WASM blob |

```json
{
  "topics": ["upgraded", "GADMIN..."],
  "data": "a1b2c3d4e5f6..."
}
```

> `get_version()` returns the new hash immediately after the transaction. Only
> one WASM version is stored; calling `upgrade()` again overwrites the
> previous value (which is then visible to consumers as the next event's
> `previous_wasm`).
---


### `treasury_transfer_started`

Emitted when the admin nominates a new treasury via `set_treasury()`. The nominee
must call `accept_treasury()` before it becomes authorized to call `deposit_yield()`.

| Index   | Location | Type    | Description                                  |
|---------|----------|---------|----------------------------------------------|
| topic 0 | topics   | Symbol  | `"treasury_transfer_started"`                 |
| topic 1 | topics   | Address | `caller` -- current admin that nominated the treasury |
| data[0] | data     | Address | `old_treasury` -- currently active treasury   |
| data[1] | data     | Address | `new_treasury` -- nominated treasury          |

---

### `treasury_transfer_completed`

Emitted when the pending treasury accepts the nomination via `accept_treasury()`.

| Index   | Location | Type    | Description                                  |
|---------|----------|---------|----------------------------------------------|
| topic 0 | topics   | Symbol  | `"treasury_transfer_completed"`               |
| topic 1 | topics   | Address | `new_treasury` -- accepting treasury          |
| data    | data     | Address | `old_treasury` -- previously active treasury  |

---

### `treasury_cancelled`

Emitted when the admin cancels a pending treasury nomination via
`cancel_treasury_transfer()`.

| Index   | Location | Type    | Description                                  |
|---------|----------|---------|----------------------------------------------|
| topic 0 | topics   | Symbol  | `"treasury_cancelled"`                        |
| topic 1 | topics   | Address | `caller` -- current admin that cancelled      |
| data    | data     | Address | `pending_treasury` -- cancelled nominee       |

---

### `yield_deposited`

Emitted when the treasury deposits accumulated protocol yield into the revenue pool
via `deposit_yield()`. The cumulative tracker is updated atomically with the transfer.

| Index   | Location | Type    | Description                                            |
|---------|----------|---------|--------------------------------------------------------|
| topic 0 | topics   | Symbol  | `"yield_deposited"`                                    |
| topic 1 | topics   | Address | `treasury` -- configured treasury that called `deposit_yield` |
| data[0] | data     | i128    | `amount` -- USDC deposited in this call (stroops)       |
| data[1] | data     | Symbol  | `source` -- short label, e.g. `"fees"` or `"yield"`    |
| data[2] | data     | i128    | `cumulative_yield_deposited` -- running total after deposit |

```json
{
  "topics": ["yield_deposited", "GTREASURY..."],
  "data": [5000000, "fees", 42000000]
}
```

> `cumulative_yield_deposited` equals `get_cumulative_yield_deposited()` immediately
> after the emitting transaction. It never decreases and panics on `i128` overflow.

---

### `admin_broadcast`

Emitted when the admin publishes an emergency message via `broadcast()`.
No tokens are moved; this is an out-of-band signaling channel for indexers and frontends.

| Index   | Location | Type             | Description                                    |
|---------|----------|------------------|------------------------------------------------|
| topic 0 | topics   | Symbol           | `"admin_broadcast"`                            |
| topic 1 | topics   | Address          | `caller` -- must be current admin               |
| data    | data     | `AdminBroadcast`   | struct with `severity` and `message` fields    |

`AdminBroadcast` struct fields:

| Field      | Type     | Description                                      |
|------------|----------|--------------------------------------------------|
| `severity` | Severity | One of `Info`, `Warn`, or `Crit`                 |
| `message`  | String   | Broadcast text; max 256 characters, never empty  |

```json
{
  "topics": ["admin_broadcast", "GADMIN..."],
  "data": { "severity": "Crit", "message": "Emergency: pausing distribution pending audit." }
}
```

> Indexers SHOULD alert on `severity = Crit`. The `message` field is capped at
> 256 characters; longer strings are rejected before the event is emitted.

---

### `emergency_drain_proposed`

Emitted when the admin proposes a timelocked emergency drain via
`propose_emergency_drain()`. The drain cannot be executed until the 24-hour
timelock has elapsed.

| Index   | Location | Type                    | Description                                  |
|---------|----------|-------------------------|----------------------------------------------|
| topic 0 | topics   | Symbol                  | `"emergency_drain_proposed"`                 |
| topic 1 | topics   | Address                 | `admin` — current admin that proposed        |
| data    | data     | `PendingEmergencyDrain` | proposal details (see below)                 |

`PendingEmergencyDrain` fields:

| Field           | Type      | Description                                              |
|-----------------|-----------|----------------------------------------------------------|
| `to`            | `Address` | Destination address for the drained USDC                 |
| `amount`        | `i128`    | Amount of USDC to drain                                  |
| `proposed_at`   | `u64`     | Ledger timestamp when the proposal was created           |
| `execute_after` | `u64`     | Earliest timestamp at which the drain may be executed    |

```json
{
  "topics": ["emergency_drain_proposed", "GADMIN..."],
  "data": {
    "to": "GTREASURY...",
    "amount": 10000000,
    "proposed_at": 1700000000,
    "execute_after": 1700086400
  }
}
```

> Only one emergency drain may be pending at a time. Re-proposing replaces
> the previous proposal and restarts the timelock.

---

### `emergency_drain_executed`

Emitted when the admin executes a pending emergency drain after the
timelock has expired via `execute_emergency_drain()`.

| Index   | Location | Type                    | Description                                  |
|---------|----------|-------------------------|----------------------------------------------|
| topic 0 | topics   | Symbol                  | `"emergency_drain_executed"`                 |
| topic 1 | topics   | Address                 | `admin` — current admin that executed        |
| data    | data     | `PendingEmergencyDrain` | proposal details at time of execution         |

```json
{
  "topics": ["emergency_drain_executed", "GADMIN..."],
  "data": {
    "to": "GTREASURY...",
    "amount": 10000000,
    "proposed_at": 1700000000,
    "execute_after": 1700086400
  }
}
```

> After this event, the pending drain is cleared. The USDC has been
> transferred to `to`.

---

### `emergency_drain_cancelled`

Emitted when the admin cancels a pending emergency drain via
`cancel_emergency_drain()`.

| Index   | Location | Type                    | Description                                  |
|---------|----------|-------------------------|----------------------------------------------|
| topic 0 | topics   | Symbol                  | `"emergency_drain_cancelled"`                |
| topic 1 | topics   | Address                 | `admin` — current admin that cancelled       |
| data    | data     | `PendingEmergencyDrain` | proposal details of the cancelled drain       |

```json
{
  "topics": ["emergency_drain_cancelled", "GADMIN..."],
  "data": {
    "to": "GTREASURY...",
    "amount": 10000000,
    "proposed_at": 1700000000,
    "execute_after": 1700086400
  }
}
```

> After this event, `get_pending_emergency_drain()` returns `None`.

---

### `swept`

Emitted when the vault owner sweeps surplus USDC to a sibling contract via
`sweep_idle_balance()`. Tokens are moved and `meta.balance` is decremented
atomically in the same transaction.

| Index   | Location | Type               | Description                                         |
|---------|----------|--------------------|-----------------------------------------------------|
| topic 0 | topics   | Symbol             | `"swept"`                                           |
| topic 1 | topics   | Address            | `owner` — vault owner who called `sweep_idle_balance` |
| topic 2 | topics   | SweepDestination   | `Settlement` or `RevenuePool` variant               |
| data    | data     | (i128, i128)       | `(amount, new_balance)` after sweep                 |

```json
{
  "topics": ["swept", "GOWNER...", "Settlement"],
  "data": [300000, 700000]
}
```

**`SweepDestination` encoding:**
- `Settlement` — USDC was forwarded to the address stored under `StorageKey::Settlement`.
- `RevenuePool` — USDC was forwarded to the address stored under `StorageKey::RevenuePool`.

**Preconditions (no event emitted if these fail):**
- Vault must not be paused (`VaultError::Paused`).
- Caller must be the vault owner (`VaultError::Unauthorized`).
- `amount > 0` (`VaultError::AmountNotPositive`).
- `amount ≤ meta.balance` (`VaultError::InsufficientBalance`).
- For `Settlement`: `set_settlement` must have been called (`VaultError::SettlementNotSet`).
- For `RevenuePool`: a revenue pool must be configured (`VaultError::NotInitialized`).

**Indexer note:** After this event, `balance()` returns `new_balance`. The USDC
has left the vault on-ledger; `sweep_idle_balance` does **not** call
`settlement.receive_payment()` — it is a raw token transfer only.

---

### `emergency_drain_proposed`

Emitted when the admin proposes a timelocked emergency drain of USDC to a
designated address via `propose_emergency_drain()`. No tokens are moved yet;
the drain becomes executable after `EMERGENCY_DRAIN_TIMELOCK_SECONDS` (24 hours).

| Index   | Location | Type                  | Description                                    |
|---------|----------|-----------------------|-------------------------------------------------|
| topic 0 | topics   | Symbol                | `"emergency_drain_proposed"`                    |
| topic 1 | topics   | Address                | `admin` — current admin who proposed the drain  |
| data    | data     | `PendingEmergencyDrain`| struct with `to`, `amount`, `proposed_at`, `execute_after` |

```json
{
  "topics": ["emergency_drain_proposed", "GADMIN..."],
  "data": { "to": "GTREASURY...", "amount": 5000000, "proposed_at": 1700000000, "execute_after": 1700086400 }
}
```

> Re-proposing while a drain is already pending replaces the prior proposal
> and restarts the timelock.

---

### `emergency_drain_executed`

Emitted when the admin executes a previously proposed emergency drain after
its timelock has expired via `execute_emergency_drain()`. The proposed USDC
amount is transferred from the contract to the destination address, and the
proposal is consumed to prevent replay.

| Index   | Location | Type    | Description                                          |
|---------|----------|---------|-------------------------------------------------------|
| topic 0 | topics   | Symbol  | `"emergency_drain_executed"`                          |
| topic 1 | topics   | Address | `admin` — current admin who executed the drain        |
| data    | data     | `(Address, i128, u64, u64)` | `(to, amount, proposed_at, executed_at)` |

```json
{
  "topics": ["emergency_drain_executed", "GADMIN..."],
  "data": ["GTREASURY...", 5000000, 1700000000, 1700086400]
}
```

---

### `emergency_drain_cancelled`

Emitted when the admin cancels a pending emergency drain proposal via
`cancel_emergency_drain()` before it is executed. No tokens are moved.

| Index   | Location | Type                   | Description                                     |
|---------|----------|------------------------|--------------------------------------------------|
| topic 0 | topics   | Symbol                 | `"emergency_drain_cancelled"`                    |
| topic 1 | topics   | Address                 | `admin` — current admin who cancelled the drain  |
| data    | data     | `PendingEmergencyDrain` | the cancelled proposal, unchanged                |

```json
{
  "topics": ["emergency_drain_cancelled", "GADMIN..."],
  "data": { "to": "GTREASURY...", "amount": 5000000, "proposed_at": 1700000000, "execute_after": 1700086400 }
}
```

---

## Contract: `callora-settlement` (v0.1.0)

Source: [`contracts/settlement/src/lib.rs`](contracts/settlement/src/lib.rs).

**Amount units.** All `amount` / `new_balance` fields are `i128` in USDC
micro-units (7-decimal scaled integers), matching the Stellar USDC contract.
Legacy text elsewhere in this document calls this "stroops" â€” same scalar type,
same integer semantics; the settlement contract never handles native XLM.

**Data payload encoding.** The `data` column describes the Soroban
`contracttype` struct published by `env.events().publish(...)`. On the wire
each struct is a single XDR value whose field names match the Rust struct;
the JSON examples below are the logical field view an indexer sees after
decoding, not a raw array. The struct layouts live in `lib.rs`:
`PaymentReceivedEvent` and `BalanceCreditedEvent`.

**Emit atomicity and ordering.** Both events originate inside one
`receive_payment()` call, so they share the same transaction and ledger
sequence. When `to_pool = false`, `payment_received` is always emitted
**before** `balance_credited`. If any guard panics (see "Panic modes" below)
no events are emitted and state is rolled back.

**Panic modes (no events emitted).**
- Caller is not the registered vault or admin (`require_authorized_caller`).
- `amount <= 0` â€” `"amount must be positive"`.
- `to_pool = true` with `developer = Some(_)` â€” `"developer address must be None when to_pool=true"`.
- `to_pool = false` with `developer = None` â€” `"developer address required when to_pool=false"`.
- Arithmetic overflow on pool or developer balance â€” `"pool balance overflow"` / `"developer balance overflow"`.

---

### `deduction_recorded`

Emitted once after a successful `record_deduction(amount, request_id)` call.
This records an accounting update only; it does not transfer tokens or credit
the global pool or a developer balance.

| Index | Location | Type | Description |
|-------|----------|------|-------------|
| topic 0 | topics | Symbol | `"deduction_recorded"` |
| `amount` | data | i128 | Positive amount added to `TotalReceived` |
| `request_id` | data | u64 | Opaque, unique deduction request identifier |

The payload is `DeductionRecordedEvent` from `contracts/settlement/src/types.rs`.
Only the configured vault may authorize this entrypoint. Non-positive amounts
fail with `AmountNotPositive` (4), duplicate IDs with `DuplicateRequestId` (43),
and cumulative overflow with `PoolOverflow` (7). Failed calls leave accounting
and request markers unchanged and emit no successful contract event.

Request markers are persistent entries keyed by `DeductionRequest(request_id)`
and extended using `PERSISTENT_BUMP_THRESHOLD` / `PERSISTENT_BUMP_AMOUNT`
(50,000 ledgers). Archived persistent entries must be restored rather than
treated as unused IDs. Markers begin with this upgrade; historical deductions
made before replay protection was introduced cannot be deduplicated retroactively.

### `initialized`

Emitted once by `init()` when the settlement contract is first configured.

| Index   | Location | Type    | Description                                     |
|---------|----------|---------|-------------------------------------------------|
| topic 0 | topics   | Symbol  | `"initialized"`                                 |
| topic 1 | topics   | Address | `admin` — initial admin address                 |
| topic 2 | topics   | Address | `vault_address` — initial authorized vault      |
| data    | data     | GlobalPool | pool snapshot at init time (`total_balance=0`) |

```json
{
  "topics": ["initialized", "GADMIN...", "GVAULT..."],
  "data": { "total_balance": 0, "last_updated": 1700000000 }
}
```

**Indexer guidance.**
- `initialized` is emitted exactly once per contract lifetime; a second `init`
  call panics with `AlreadyInitialized` before reaching this emit.
- Use this event to index the contract's admin and vault addresses from genesis
  without querying `get_admin()` / `get_vault()` separately.

---

### `payment_received`

Emitted by `receive_payment()` for every successful inbound payment,
regardless of routing.

| Index        | Location | Type              | Description                                                                       |
|--------------|----------|-------------------|-----------------------------------------------------------------------------------|
| topic 0      | topics   | Symbol            | `"payment_received"`                                                              |
| topic 1      | topics   | Address           | `caller` â€” authorized vault or admin address (same as `from_vault` field)         |
| `from_vault` | data     | Address           | originator of the payment; duplicates topic 1 for indexers that key by data only  |
| `amount`     | data     | i128              | payment amount in USDC micro-units; invariant `amount > 0`                        |
| `to_pool`    | data     | bool              | `true` â†’ credited to global pool; `false` â†’ credited to an individual developer   |
| `developer`  | data     | Option\<Address\> | `None` when `to_pool = true`; `Some(address)` when `to_pool = false`              |

**Example â€” global pool credit (`to_pool = true`):**

```json
{
  "topics": ["payment_received", "GCALLER..."],
  "data": {
    "from_vault": "GCALLER...",
    "amount": 5000000,
    "to_pool": true,
    "developer": null
  }
}
```

Side effect: `GlobalPool.total_balance += amount` and
`GlobalPool.last_updated = env.ledger().timestamp()`.

**Example â€” developer credit (`to_pool = false`):**

```json
{
  "topics": ["payment_received", "GCALLER..."],
  "data": {
    "from_vault": "GCALLER...",
    "amount": 2500000,
    "to_pool": false,
    "developer": "GDEV..."
  }
}
```

Side effect: developer balance map entry for `GDEV...` is incremented by
`amount`. `GlobalPool.last_updated` is **not** touched on developer credits.

**Indexer guidance.**
- `topic 1` is always the caller; filter on it to isolate payments from a
  specific vault or admin.
- `developer` is the only field that distinguishes pool vs. developer credits
  in the data payload; the `to_pool` boolean is redundant but stable and
  cheaper to filter on.
- A `payment_received` with `to_pool = false` is always paired with exactly
  one `balance_credited` event in the same transaction.

---

### `supported_token_added`

Emitted when the admin registers a token for settlement payments. The event is
also emitted when `set_usdc_token()` or the one-time storage migration
backfills the configured USDC token.

| Index   | Location | Type    | Description |
|---------|----------|---------|-------------|
| topic 0 | topics   | Symbol  | `"supported_token_added"` |
| topic 1 | topics   | Address | Admin that enabled the token |
| topic 2 | topics   | Address | Token contract address |
| data    | data     | Address | Token contract address |

### `supported_token_removed`

Emitted when the admin removes a registered token. Existing developer balances
remain stored, but future single and batch payments in that token are rejected.

| Index   | Location | Type    | Description |
|---------|----------|---------|-------------|
| topic 0 | topics   | Symbol  | `"supported_token_removed"` |
| topic 1 | topics   | Address | Admin that disabled the token |
| topic 2 | topics   | Address | Token contract address |
| data    | data     | Address | Token contract address |

### `balance_credited`

Emitted by `receive_payment()` **only** when `to_pool = false`, immediately
after the matching `payment_received` event.

| Index         | Location | Type    | Description                                                     |
|---------------|----------|---------|-----------------------------------------------------------------|
| topic 0       | topics   | Symbol  | `"balance_credited"`                                            |
| topic 1       | topics   | Address | `developer` â€” address whose balance was updated                 |
| `developer`   | data     | Address | same as topic 1; duplicated for data-only indexers              |
| `amount`      | data     | i128    | amount credited to the developer in USDC micro-units            |
| `new_balance` | data     | i128    | developer's cumulative balance after this credit (post-state)   |

```json
{
  "topics": ["balance_credited", "GDEV..."],
  "data": {
    "developer": "GDEV...",
    "amount": 2500000,
    "new_balance": 7500000
  }
}
```

**Invariants.**
- `new_balance = prior_balance + amount`, checked for `i128` overflow; overflow
  panics and rolls back both events.
- `new_balance` equals `CalloraSettlement::get_developer_balance(developer)`
  immediately after the emitting transaction.
- `amount` in `balance_credited` equals `amount` in the paired
  `payment_received`.

**Indexer guidance.**
- Track developer earnings by subscribing to `balance_credited` â€” it already
  carries the post-credit balance, so no separate read is required.
- Track total protocol inflow by summing `payment_received.amount` across
  both routing modes, or filter `to_pool = true` for pool-only inflow.
- `balance_credited` is **never** emitted when `to_pool = true`; do not wait
  for one on pool credits.

---

### `deposit`

Emitted alongside every `balance_credited` event, once per developer credit in
both `receive_payment()` (`to_pool = false`) and `batch_receive_payment()`.
It provides a compact deposit-centric view for indexers that don't need the
full `PaymentReceivedEvent` context.

| Index       | Location | Type    | Description                                          |
|-------------|----------|---------|------------------------------------------------------|
| topic 0     | topics   | Symbol  | `"deposit"`                                          |
| topic 1     | topics   | Address | `developer` — address receiving the credit           |
| `developer` | data     | Address | same as topic 1; duplicated for data-only indexers   |
| `token`     | data     | Address | token contract address for this deposit              |
| `amount`    | data     | i128    | deposit amount in USDC micro-units; invariant `> 0`  |

```json
{
  "topics": ["deposit", "GDEV..."],
  "data": {
    "developer": "GDEV...",
    "token": "GUSDC...",
    "amount": 2500000
  }
}
```

**Indexer guidance.**
- `deposit` is always emitted **after** `balance_credited` for the same
  developer credit within the same transaction.
- `deposit` is **never** emitted for pool credits (`to_pool = true`).
- For batch credits, `N` items produce exactly `N` `deposit` events.
- The `token` field identifies which asset was deposited; required for
  multi-asset environments.

---

Emitted by `set_vault()` when the admin updates the registered vault address.

| Index   | Location | Type    | Description                        |
|---------|----------|---------|------------------------------------|
| topic 0 | topics   | Symbol  | `"vault_changed"`                |
| topic 1 | topics   | Address | `caller` — admin who performed update |
| data    | data     | (Address, Address) | (old_vault, new_vault)        |

```json
{
  "topics": ["vault_changed", "GADMIN..."],
  "data": ["GOLDVAULT...", "GNEWVAULT..."]
}
```

---

### `developer_withdraw`

Emitted by `withdraw_developer_balance()` when a developer withdraws their balance as USDC.

| Index             | Location | Type    | Description                                                     |
|-------------------|----------|---------|-----------------------------------------------------------------|
| topic 0           | topics   | Symbol  | `"developer_withdraw"`                                          |
| topic 1           | topics   | Address | `developer` — address withdrawing the balance                   |
| `developer`       | data     | Address | same as topic 1; duplicated for data-only indexers              |
| `amount`          | data     | i128    | amount withdrawn in USDC micro-units                            |
| `remaining_balance`| data    | i128    | developer's cumulative balance after this withdrawal (post-state)|
| `to`              | data     | Address | recipient address the funds were sent to (defaults to developer)|

```json
{
    "topics": ["developer_withdraw", "GDEV..."],
    "data": {
        "developer": "GDEV...",
        "amount": 2000000,
        "remaining_balance": 0,
        "to": "GRECIPIENT..."
    }
}
```

**Invariants.**
- `remaining_balance = prior_balance - amount`, checked for underflow.
- `to` cannot be the contract's own address.
- If `to` is not provided (None), it defaults to `developer`.

### `developer_force_credited`

Emitted by `force_credit_developer()` when an admin manually credits a developer balance (escape hatch).

This is an **admin-authorized inflow** — no on-ledger USDC is moved. It is designed for
operational edge cases (off-chain payment reconciliation, dispute resolution).

| Index         | Location | Type    | Description                                                     |
|---------------|----------|---------|-----------------------------------------------------------------|
| topic 0       | topics   | Symbol  | `"developer_force_credited"`                                    |
| topic 1       | topics   | Address | `developer` — address whose balance was updated                  |
| `developer`   | data     | Address | same as topic 1; duplicated for data-only indexers              |
| `amount`      | data     | i128    | amount credited to the developer in USDC micro-units            |
| `reason`      | data     | Symbol  | on-chain reason code for the manual credit                      |
| `new_balance` | data     | i128    | developer's cumulative balance after this credit (post-state)   |

```json
{
    "topics": ["developer_force_credited", "GDEV..."],
    "data": {
        "developer": "GDEV...",
        "amount": 5000000,
        "reason": "offline_settlement",
        "new_balance": 7500000
    }
}
```

**Invariants.**
- `new_balance = prior_balance + amount`, checked for `i128` overflow.
- Only the contract admin may call `force_credit_developer`.
- This is an audit-only path; every credit includes an on-chain `reason` Symbol.

**Indexer guidance.**
- Subscribe to `developer_force_credited` to track admin-initiated manual credits.
- The `reason` field distinguishes different operational scenarios (e.g., `"dispute_resolution"`, `"offline_settlement"`, `"bulk_reconciliation"`).
- This event is **never** paired with a `payment_received` event.
- For full accounting, sum `balance_credited.amount` + `developer_force_credited.amount` to compute total developer inflows.

---

## Contract: `callora-distribute` (v0.1.0)

The distribute contract coordinates batched and single-leg USDC payouts to developers and recipients.
All events emitted by `callora-distribute` follow the structured 3-topic shape `(action, version, subject)`
where topic[1] is the canonical version marker `Symbol("callora_v1")`.

### Lifecycle Overview (`batch_distribute`)

When `batch_distribute` is called with $N$ legs:
1. `batch_distribute_started` is emitted with `(total_amount, N)` as data.
2. For each payment leg $i \in [0, N-1]$ in strict sequence order:
   - `distribute_started` is emitted with recipient topic and `DistributionLifecycleEvent` payload (`mode: Batch`, `batch_index: i`, `batch_size: N`).
   - The token transfer `usdc.transfer(contract, recipient, amount)` is executed.
   - `distribute` is emitted with recipient topic and `amount: i128` data.
   - `distribute_completed` is emitted with recipient topic and `DistributionLifecycleEvent` payload (`mode: Batch`, `batch_index: i`, `batch_size: N`).
3. `batch_distribute_completed` is emitted with `(total_amount, N)` as data, where `total_amount` strictly equals the sum of the $N$ per-leg amounts.

If any leg fails or validation fails, the entire transaction reverts atomically and no events are persisted to the ledger.

### `batch_distribute_started`

Emitted immediately after parameter validation passes, prior to executing any payout legs.

| Index   | Location | Type         | Description                                        |
|---------|----------|--------------|----------------------------------------------------|
| topic 0 | topics   | Symbol       | `"batch_distribute_started"`                       |
| topic 1 | topics   | Symbol       | `"callora_v1"`                                     |
| topic 2 | topics   | Address      | `admin` — caller initiating the batch              |
| data    | data     | (i128, u32)  | `(total_amount, count)` — total stroops and legs   |

```json
{
  "topics": ["batch_distribute_started", "callora_v1", "GADMIN..."],
  "data": [3000000, 3]
}
```

---

### `distribute_started`

Emitted immediately prior to each USDC transfer leg (for both `batch_distribute` and `distribute`).

| Index   | Location | Type                       | Description                                           |
|---------|----------|----------------------------|-------------------------------------------------------|
| topic 0 | topics   | Symbol                     | `"distribute_started"`                                |
| topic 1 | topics   | Symbol                     | `"callora_v1"`                                         |
| topic 2 | topics   | Address                    | `recipient` — developer receiving the leg payout      |
| data    | data     | `DistributionLifecycleEvent` | Structured lifecycle tracking record                  |

```json
{
  "topics": ["distribute_started", "callora_v1", "GRECIPIENT..."],
  "data": {
    "version": 1,
    "amount": 1000000,
    "mode": "Batch",
    "batch_index": 0,
    "batch_size": 3,
    "ledger_sequence": 123456,
    "timestamp": 1775000000
  }
}
```

---

### `distribute`

Emitted immediately following each successful USDC transfer leg. Provides indexers with the recipient and amount for payout reconciliation.

| Index   | Location | Type    | Description                                           |
|---------|----------|---------|-------------------------------------------------------|
| topic 0 | topics   | Symbol  | `"distribute"`                                        |
| topic 1 | topics   | Symbol  | `"callora_v1"`                                         |
| topic 2 | topics   | Address | `recipient` — developer who received the transfer     |
| data    | data     | i128    | `amount` in stroops transferred to `recipient`        |

```json
{
  "topics": ["distribute", "callora_v1", "GRECIPIENT..."],
  "data": 1000000
}
```

---

### `distribute_completed`

Emitted immediately following the successful completion and verification of each transfer leg.

| Index   | Location | Type                       | Description                                           |
|---------|----------|----------------------------|-------------------------------------------------------|
| topic 0 | topics   | Symbol                     | `"distribute_completed"`                              |
| topic 1 | topics   | Symbol                     | `"callora_v1"`                                         |
| topic 2 | topics   | Address                    | `recipient` — developer receiving the leg payout      |
| data    | data     | `DistributionLifecycleEvent` | Structured lifecycle tracking record                  |

```json
{
  "topics": ["distribute_completed", "callora_v1", "GRECIPIENT..."],
  "data": {
    "version": 1,
    "amount": 1000000,
    "mode": "Batch",
    "batch_index": 0,
    "batch_size": 3,
    "ledger_sequence": 123456,
    "timestamp": 1775000000
  }
}
```

---

### `batch_distribute_completed`

Emitted after all payment legs have succeeded.

| Index   | Location | Type         | Description                                        |
|---------|----------|--------------|----------------------------------------------------|
| topic 0 | topics   | Symbol       | `"batch_distribute_completed"`                     |
| topic 1 | topics   | Symbol       | `"callora_v1"`                                     |
| topic 2 | topics   | Address      | `admin` — caller who executed the batch            |
| data    | data     | (i128, u32)  | `(total_amount, count)` — verified sum and count   |

```json
{
  "topics": ["batch_distribute_completed", "callora_v1", "GADMIN..."],
  "data": [3000000, 3]
}
```

---

### `init`

Emitted once when the distribute contract is initialized with admin and USDC token.

| Index   | Location | Type               | Description                                    |
|---------|----------|--------------------|------------------------------------------------|
| topic 0 | topics   | Symbol             | `"init"`                                       |
| topic 1 | topics   | Symbol             | `"callora_v1"`                                 |
| topic 2 | topics   | Address            | `admin`                                        |
| data    | data     | (Address, Address) | `(admin, usdc_token)`                          |

---

### `pause_set`

Emitted when the contract pause status is updated.

| Index   | Location | Type    | Description                                        |
|---------|----------|---------|----------------------------------------------------|
| topic 0 | topics   | Symbol  | `"pause_set"`                                      |
| topic 1 | topics   | Symbol  | `"callora_v1"`                                     |
| topic 2 | topics   | Address | `admin`                                            |
| data    | data     | bool    | `paused` (`true` for paused, `false` for active)   |

---

### `set_max_distribute`

Emitted when the per-leg distribution limit is reconfigured.

| Index   | Location | Type         | Description                                        |
|---------|----------|--------------|----------------------------------------------------|
| topic 0 | topics   | Symbol       | `"set_max_distribute"`                             |
| topic 1 | topics   | Symbol       | `"callora_v1"`                                     |
| topic 2 | topics   | Address      | `admin`                                            |
| data    | data     | (i128, i128) | `(old_max, new_max)`                               |

---

### `admin_changed` / `admin_transfer_started` / `admin_transfer_completed` / `admin_cancelled`

Two-step admin handover events following the standard Callora governance lifecycle.

---

## Contract: `callora-freeze` (v0.0.1)

Every state-changing entrypoint emits exactly one event. Topic[1] is always
`"callora_v1"` for version filtering. The `reason` and `frozen_at` fields from
`freeze_set` are also persisted in storage and readable via `get_freeze_status()`.

### `freeze_initialized`

Emitted once by `init()`.

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

Emitted by `freeze()`. Payload includes the reason label and ledger timestamp.

| Index      | Location | Type     | Description                                 |
|------------|----------|----------|---------------------------------------------|
| topic 0    | topics   | Symbol   | `"freeze_set"`                              |
| topic 1    | topics   | Symbol   | `"callora_v1"` (version marker)             |
| topic 2    | topics   | Address  | `caller` — admin or freeze operator         |
| `reason`   | data     | Symbol   | reason label supplied by the caller         |
| `frozen_at`| data     | u64      | `env.ledger().timestamp()` at freeze time   |

```json
{
  "topics": ["freeze_set", "callora_v1", "GCALLER..."],
  "data": { "reason": "exploit_risk", "frozen_at": 1700000000 }
}
```

---

### `freeze_cleared`

Emitted by `unfreeze()`. Persisted reason and timestamp are cleared atomically.

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

---

### `freeze_operator_set`

Emitted by `set_freeze_operator()` for both set and clear operations.
`new_operator: null` means the role was cleared.

| Index          | Location | Type              | Description                                         |
|----------------|----------|-------------------|-----------------------------------------------------|
| topic 0        | topics   | Symbol            | `"freeze_operator_set"`                             |
| topic 1        | topics   | Symbol            | `"callora_v1"` (version marker)                     |
| topic 2        | topics   | Address           | `caller` — admin who updated the operator           |
| `old_operator` | data     | `Option<Address>` | operator before this call; `null` if none was set   |
| `new_operator` | data     | `Option<Address>` | operator after this call; `null` if role was cleared|

```json
{
  "topics": ["freeze_operator_set", "callora_v1", "GADMIN..."],
  "data": { "old_operator": null, "new_operator": "GOPERATOR..." }
}
```

---

## Indexer quick-reference

| Event                    | Contract        | Trigger                                  |
|--------------------------|-----------------|------------------------------------------|
| `init`                   | vault           | `init()`                                 |
| `deposit`                | vault           | `deposit()`                              |
| `deduct`                 | vault           | `deduct()` / each item in `batch_deduct()`|
| `withdraw`               | vault           | `withdraw()`                             |
| `withdraw_to`            | vault           | `withdraw_to()`                          |
| `vault_paused`           | vault           | `pause()`                                |
| `vault_unpaused`         | vault           | `unpause()`                              |
| `ownership_nominated`    | vault           | `transfer_ownership()`                   |
| `ownership_accepted`     | vault           | `accept_ownership()`                     |
| `admin_nominated`        | vault           | `set_admin()`                            |
| `admin_accepted`         | vault           | `accept_admin()`                         |
| `set_revenue_pool`       | vault           | `set_revenue_pool(Some(addr))`           |
| `clear_revenue_pool`     | vault           | `set_revenue_pool(None)`                 |
| `set_max_deduct`         | vault           | `set_max_deduct()`                       |
| `set_authorized_caller` | vault           | `set_authorized_caller()`                |
| `tl_window_changed`     | vault           | `set_timelock_window()`                  |
| `metadata_set`           | vault           | `set_metadata()`                         |
| `metadata_updated`       | vault           | `update_metadata()`                      |
| `metadata_removed`       | vault           | `remove_metadata()`                      |
| `distribute`             | vault           | `distribute()`                           |
| `swept`                  | vault           | `sweep_idle_balance()`                   |
| `init`                   | revenue-pool    | `init()`                                 |
| `admin_changed`          | revenue-pool    | `accept_admin()` / `claim_admin()`       |
| `admin_transfer_started` | revenue-pool    | `set_admin()`                            |
| `set_max_distribute`     | revenue-pool    | `set_max_distribute()`                   |
| `admin_transfer_completed`| revenue-pool   | `claim_admin()`                          |
| `receive_payment`        | revenue-pool    | `receive_payment()`                      |
| `distribute`             | revenue-pool    | `distribute()`                           |
| `batch_distribute`       | revenue-pool    | each payment in `batch_distribute()`     |
| `pause_set`              | revenue-pool    | `pause()` / `unpause()`                  |
| `admin_cancelled`        | revenue-pool    | `cancel_admin_transfer()`                |
| `upgraded`               | revenue-pool    | `upgrade()`                              |
| `treasury_transfer_started` | revenue-pool | `set_treasury()`                         |
| `treasury_transfer_completed` | revenue-pool | `accept_treasury()`                      |
| `treasury_cancelled`   | revenue-pool    | `cancel_treasury_transfer()`             |
| `yield_deposited`        | revenue-pool    | `deposit_yield()`                        |
| `admin_broadcast`        | revenue-pool    | `broadcast()`                            |
| `emergency_drain_proposed` | revenue-pool  | `propose_emergency_drain()`              |
| `emergency_drain_executed` | revenue-pool  | `execute_emergency_drain()`              |
| `emergency_drain_cancelled` | revenue-pool | `cancel_emergency_drain()`               |
| `payment_received`       | settlement      | `receive_payment()`                      |
| `balance_credited`       | settlement      | `receive_payment()` with `to_pool=false` |
| `vault_changed`          | settlement      | `set_vault()`                            |
| `developer_force_credited`| settlement     | `force_credit_developer()`               |
| `init`                   | distribute      | `init()`                                 |
| `admin_changed`          | distribute      | `accept_admin()` / `claim_admin()`       |
| `admin_transfer_started` | distribute      | `set_admin()`                            |
| `admin_transfer_completed`| distribute     | `accept_admin()` / `claim_admin()`       |
| `admin_cancelled`        | distribute      | `cancel_admin_transfer()`                |
| `pause_set`              | distribute      | `pause()` / `unpause()`                  |
| `set_max_distribute`     | distribute      | `set_max_distribute()`                   |
| `batch_distribute_started` | distribute    | `batch_distribute()` before legs         |
| `batch_distribute_completed`| distribute   | `batch_distribute()` after all legs      |
| `distribute`             | distribute      | each payment leg in `batch_distribute()` and `distribute()` |
| `distribute_started`     | distribute      | before each transfer in `batch_distribute()` and `distribute()` |
| `distribute_completed`   | distribute      | after each transfer in `batch_distribute()` and `distribute()` |
| `upgraded`               | distribute      | `upgrade()`                              |
| `freeze_initialized`     | freeze          | `init()`                                 |
| `freeze_set`             | freeze          | `freeze()`                               |
| `freeze_cleared`         | freeze          | `unfreeze()`                             |
| `freeze_operator_set`    | freeze          | `set_freeze_operator()` (set or clear)   |

---

## Version history

| Version | Contract      | Change                                                       |
|---------|---------------|--------------------------------------------------------------|
| 0.0.1   | vault         | Initial vault events                                         |
| 0.0.1   | vault         | Added `set_authorized_caller` event with old/new value payload (Issue #256) |
| 0.0.1   | vault         | Added `metadata_removed` event on `remove_metadata()` for stale-entry cleanup |
| 0.0.1   | revenue-pool  | Full revenue pool event suite with JSON examples             |
| 0.0.1   | revenue-pool  | Added `admin_changed` event on `set_admin` for explicit old/new admin intent |
| 0.1.0   | settlement    | `payment_received`, `balance_credited`                       |
| 0.1.0   | settlement    | `developer_force_credited` (admin escape hatch)               |
| 0.2.0   | vault         | Added `swept` event on `sweep_idle_balance()` (Issue #415)  |
| 0.2.0   | vault         | Fixed `tl_window_changed` payload: first element is now the **previous** window, second is the new window (Issue #1112). Prior versions emitted `(new, new)`. |
| 0.2.0   | revenue-pool  | Added `emergency_drain_proposed`, `emergency_drain_executed`, `emergency_drain_cancelled` events |
| 0.2.0   | settlement    | Added `developer_min_balance_changed` event on `set_developer_min_balance()` (Issue #633) |
| 0.3.0   | vault         | Added `"callora_v1"` version marker at topic[1] for `withdraw`, `withdraw_to`, `distribute`, `rescue_funds`, `reserve_cap_set`, `request_id_pruned` (Issue #1118) |
| 0.3.0   | vault         | Version symbol renamed `"callora.v1"` → `"callora_v1"` (dot not allowed in Soroban Symbol charset) |
| 0.1.0   | distribute    | Initial distribute contract with batch and per-leg transfer events |
| 0.1.1   | distribute    | Publish per-leg transfer events (`distribute_started`, `distribute`, `distribute_completed`) with recipient topic and `DistributionLifecycleEvent` payload for payout reconciliation |
| 0.0.1   | freeze        | Added `freeze_initialized`, `freeze_set`, `freeze_cleared`, `freeze_operator_set` events; `reason` persisted in storage; `get_freeze_status()` view added (Issue #1217) |
| 0.3.0   | revenue-pool  | Moved `admin_changed` from `set_admin()` to `accept_admin()` so nomination no longer announces a change (Issue #1163) |
