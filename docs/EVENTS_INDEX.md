# Callora Contracts — Structured Events Index

This document is the canonical index of every event emitted by the Callora
smart contracts (`settlement`, `revenue_pool`, `vault`, `whitelist`, `distribute`). It defines the
target topic structure for off-chain consumers, lists every currently
emitted event with its actual topic shape, and defines the backwards-compat
ladder for migrating existing events toward the target shape without
breaking indexers that already depend on them.

## Target Topic Structure

Every event SHOULD eventually be published with exactly three topic
elements, in this order:

| Position | Name       | Type     | Description                                                          |
|----------|------------|----------|------------------------------------------------------------------------|
| Topic1   | `contract` | `Symbol` | Identifies which contract emitted the event: `settlement`, `revenue_pool`, or `vault`. |
| Topic2   | `action`   | `Symbol` | The event name itself, e.g. `payment_received`, `deposit`, `upgraded`. |
| Topic3   | `subject`  | `Address`| The primary address the event concerns (caller, developer, owner, etc.). |

The event **data** (the second argument to `env.events().publish((topics), data)`)
carries the remaining structured fields (amounts, balances, flags) as a
`#[contracttype]` struct or tuple — this document does not change that.

## Current State (as of this document)

**None of the currently emitted events use the full 3-topic target shape
yet.** Every existing call site publishes exactly **2 topics**:
`(action, subject)`, with no explicit `contract` topic. This is because
Soroban event subscriptions are already scoped per-contract by the
contract address at the protocol level, so a redundant `contract` topic
was historically omitted.

This document formalizes the target shape going forward and defines the
migration ladder below, rather than retroactively rewriting all 39
existing call sites in one change (see "Why not migrate everything now?").

### Why not migrate everything now?

- Changing topic shape is a **breaking change** for any off-chain indexer
  already subscribed to these events by position.
- Rewriting 39 call sites across three contracts in a single change is a
  large, high-risk diff that is difficult to review safely and increases
  the chance of an unintended functional regression in unrelated logic.
- The repo's CI gate (`scripts/check-event-shape.sh`) enforces something
  immediately actionable and safe today: **every event topic must come
  from a centralized `events::event_*()` constructor**, never an inline
  `Symbol::new(...)` literal at the call site. This closes the most common
  source of drift (typos, accidental renames) without touching the wire
  format of any event.

## Backwards-Compatibility Ladder

Events migrate to the target 3-topic shape in stages, never all at once:

1. **Stage 0 — Current (today).** All events use `(action, subject)`,
   2 topics. Centralized via `events::event_*()` constructors (enforced
   by CI gate). This document is the source of truth for what exists.
2. **Stage 1 — Dual-publish (opt-in, per event).** When a specific event
   needs indexer-side disambiguation between contracts (e.g. if an
   off-chain consumer subscribes to events from multiple Callora contracts
   at once without filtering by contract address), that event MAY be
   migrated to publish **both** the old 2-topic shape and a new 3-topic
   shape as two separate `publish()` calls, for one full deprecation
   window (minimum 1 minor version). This avoids breaking existing
   consumers while giving new consumers the structured shape.
3. **Stage 2 — Cutover.** After the deprecation window, the old 2-topic
   publish call is removed for that event, leaving only the 3-topic
   shape. This is a breaking change and MUST be called out explicitly in
   the changelog/release notes for that version.
4. **New events.** Any event introduced *after* this document is adopted
   SHOULD be published directly in the target 3-topic shape from the
   start — there is no need to stage a brand-new event through 2-topic
   first.

No event is migrated through these stages as part of this PR. This PR
establishes the document, the ladder, and the CI gate; future PRs migrate
individual events through the ladder as indexer needs arise.

## Event Index

### `settlement` contract

| Event topic (action) | Constructor | Subject (topic2 today) | Data payload |
|---|---|---|---|
| `payment_received` | `event_payment_received` | vault/admin caller | `PaymentReceivedEvent { from_vault, amount, to_pool, developer }` |
| `supported_token_added` | `event_supported_token_added` | caller, token | token address |
| `supported_token_removed` | `event_supported_token_removed` | caller, token | token address |
| `balance_credited` | `event_balance_credited` | developer address | `BalanceCreditedEvent { developer, amount, new_balance }` |
| `developer_withdraw` | `event_developer_withdraw` | developer address | `DeveloperWithdrawEvent { developer, amount, to }` |
| `daily_withdraw_cap_changed` | `event_daily_withdraw_cap_changed` | caller | `DailyWithdrawCapChanged { developer, new_cap }` |
| `claim_window_changed` | `event_developer_claim_window_changed` | developer address | `DeveloperClaimWindowChanged { developer, start_ts, end_ts, enabled }` |
| `developer_force_credited` | `event_developer_force_credited` | developer address | `DeveloperForceCreditedEvent { developer, amount, reason, new_balance }` |
| `admin_nominated` | `event_admin_nominated` | current admin | new admin address |
| `admin_accepted` | `event_admin_accepted` | pending admin | — |
| `admin_cancelled` | `event_admin_cancelled` | current admin, pending admin | — |
| `vault_proposed` | `event_vault_proposed` | caller | `VaultProposedEvent { current_vault, proposed_vault }` |
| `vault_accepted` | `event_vault_accepted` | caller | `VaultAcceptedEvent { old_vault, new_vault }` |
| `admin_broadcast` | `event_admin_broadcast` | caller | `AdminBroadcast { severity, message }` |
| `upgraded` | `event_upgraded` | admin | new WASM hash |

### `revenue_pool` contract

| Event topic (action) | Constructor | Subject (topic2 today) | Data payload |
|---|---|---|---|
| `init` | `event_init` | — | admin, USDC token address |
| `admin_changed` | `event_admin_changed` | previous admin | `(previous_admin, new_admin)` — emitted by `accept_admin()`, never by `set_admin()` |
| `admin_transfer_started` | `event_admin_transfer_started` | current admin | new admin address |
| `admin_transfer_completed` | `event_admin_transfer_completed` | — | — |
| `admin_cancelled` | `event_admin_cancelled` | — | — |
| `pause_set` | `event_pause_set` | — | `true` (paused) / `false` (unpaused) |
| `receive_payment` | `event_receive_payment` | caller | `(amount, from_vault)` |
| `yield_deposited` | `event_yield_deposited` | treasury | `(amount, source, new_total)` |
| `set_max_distribute` | `event_set_max_distribute` | admin | `(old_max, max_distribute)` |
| `distribute` | `event_distribute` | — | — |
| `batch_distribute` | `event_batch_distribute` | — | — |
| `distribute_started` | `event_distribute_started` | caller, recipient | `DistributionLifecycleEvent` |
| `distribute_completed` | `event_distribute_completed` | caller, recipient | `DistributionLifecycleEvent` |
| `upgraded` | `event_upgraded` | — | new WASM hash |
| `admin_broadcast` | `event_admin_broadcast` | caller | `AdminBroadcast { severity, message }` |

### `vault` contract

| Event topic (action) | Constructor | Subject (topic2 today) | Data payload |
|---|---|---|---|
| `init` | `event_init` | — | owner, initial balance |
| `admin_nominated` | `event_admin_nominated` | owner | — |
| `admin_accepted` | `event_admin_accepted` | — | — |
| `admin_cancelled` | `event_admin_cancelled` | — | — |
| `set_authorized_caller` | `event_set_authorized_caller` | owner | — |
| `set_max_deduct` | `event_set_max_deduct` | owner | `(old, max_deduct)` |
| `vault_paused` | `event_vault_paused` | — | — |
| `vault_unpaused` | `event_vault_unpaused` | — | — |
| `deposit` | `event_deposit` | caller | `(amount, balance)` |
| `deduct` | `event_deduct` | caller, request ID | `(amount, balance)` |
| `ownership_nominated` | `event_ownership_nominated` | owner | — |
| `ownership_accepted` | `event_ownership_accepted` | old owner, new owner | — |
| `withdraw` | `event_withdraw` | owner | `(amount, balance)` |
| `withdraw_to` | `event_withdraw_to` | owner, recipient | `(amount, balance)` |
| `distribute` | `event_distribute` | — | — |
| `set_revenue_pool` | `event_set_revenue_pool` | owner | new pool address |
| `clear_revenue_pool` | `event_clear_revenue_pool` | owner | — |
| `set_settlement` | `event_set_settlement` | admin | settlement address |
| `metadata_set` | `event_metadata_set` | admin | — |
| `price_set` | `event_price_set` | admin | — |
| `price_removed` | `event_price_removed` | admin | — |
| `metadata_updated` | `event_metadata_updated` | admin | — |
| `metadata_removed` | `event_metadata_removed` | admin | — |
| `upgraded` | `event_upgraded` | — | new WASM hash |
| `allowlist_add` | `event_allowlist_add` | owner | — |
| `allowlist_clear` | `event_allowlist_clear` | owner | — |
| `revenue_pool_proposed` | `event_revenue_pool_proposed` | owner, new pool | — |
| `revenue_pool_accepted` | `event_revenue_pool_accepted` | old, new | — |
| `revenue_pool_cancelled` | `event_revenue_pool_cancelled` | — | — |
| `request_id_pruned` | `event_request_id_pruned` | caller | pruned request ID |
| `admin_broadcast` | `event_admin_broadcast` | caller | `AdminBroadcast { severity, message }` |

### `whitelist` contract

Whitelist access-list mutations are prime audit targets, so every mutating
entrypoint emits exactly one event. All whitelist events are published in the
versioned shape `(action, version, subject[, affected])`:

| Topic position | Value |
|---|---|
| Topic 0 | action symbol (see table below) |
| Topic 1 | `callora_v1` (`event_version_v1`) — schema version marker |
| Topic 2 | the authorized caller (for `accept_admin`, the nominee) |
| Topic 3 | the affected address, when there is exactly one |

| Event topic (action) | Constructor | Trigger | Topics (beyond version) | Data payload |
|---|---|---|---|---|
| `init` | `event_init` | `init` registers the owner/admin | admin | owner/admin address |
| `address_added` | `event_address_added` | `add_address` appends an address | caller, address | `()` |
| `address_removed` | `event_address_removed` | `remove_address` removes an address | caller, address | `()` |
| `whitelist_cleared` | `event_whitelist_cleared` | `clear_all` empties the whitelist | caller | number of entries cleared (`u32`) |
| `admin_nominated` | `event_admin_nominated` | `set_admin` nominates a successor | caller, new admin | new admin address |
| `admin_accepted` | `event_admin_accepted` | `accept_admin` completes the transfer | new admin | new admin address |
| `admin_cooldown_set` | `event_admin_cooldown_set` | `set_admin_cooldown` updates the window | caller | new window in seconds (`u64`) |

**Failure-mode note:** `add_address` on an address that is already present
returns `WhitelistError::AddressAlreadyInWhitelist`
([`contracts/whitelist/src/errors.rs`](../contracts/whitelist/src/errors.rs))
*before* any event is published, so duplicate add attempts emit nothing.
Similarly, a cool-off rejection or authorization failure aborts before the
publish call, so no spurious events are emitted for rejected mutations.
### `distribute` contract

| Event topic (action) | Constructor | Subject (topic2) | Data payload |
|---|---|---|---|
| `init` | `event_init` | admin | `(admin, usdc_token)` |
| `admin_changed` | `event_admin_changed` | previous admin | `(previous_admin, new_admin)` (emitted on accept) |
| `admin_transfer_started` | `event_admin_transfer_started` | current admin | `pending_admin` |
| `admin_transfer_completed` | `event_admin_transfer_completed` | new admin | — |
| `admin_cancelled` | `event_admin_cancelled` | current admin | — |
| `pause_set` | `event_pause_set` | admin | `true` (paused) / `false` (unpaused) |
| `set_max_distribute` | `event_set_max_distribute` | admin | `(old_max, max_distribute)` |
| `batch_distribute_started` | `event_batch_distribute_started` | admin | `(total_amount, count)` |
| `batch_distribute_completed` | `event_batch_distribute_completed` | admin | `(total_amount, count)` |
| `distribute_started` | `event_distribute_started` | recipient | `DistributionLifecycleEvent` |
| `distribute` | `event_distribute` | recipient | `amount` (i128) |
| `distribute_completed` | `event_distribute_completed` | recipient | `DistributionLifecycleEvent` |
| `upgraded` | `event_upgraded` | admin | new WASM hash |

## CI Gate

`scripts/check-event-shape.sh` enforces the **one rule that is safe to
enforce today without changing wire format**: every `env.events().publish()`
call site across `contracts/*/src/lib.rs` must use a centralized
`events::event_*()` constructor for its action topic — never a raw inline
`Symbol::new(&env, "...")` literal.

Run it locally:

```bash
./scripts/check-event-shape.sh
```

Exit code `0` means every call site is compliant. Exit code `1` means one
or more raw inline topic literals were found, with the file and line
number printed for each violation.

This script is intended to be wired into CI (e.g. as a step in the
existing GitHub Actions workflow) so that any future PR introducing a new
event via inline `Symbol::new(...)` fails the build automatically, with a
pointer back to this document.

## Adding a New Event — Checklist

1. Add a `pub fn event_<name>(env: &Env) -> Symbol` constructor to the
   relevant contract's `events.rs`, with a rustdoc comment explaining when
   it fires.
2. Add a snapshot test in `events.rs`'s `#[cfg(test)] mod tests` asserting
   the constructor returns the exact expected `Symbol` bytes.
3. Publish the event from `lib.rs` using `events::event_<name>(&env)` —
   never an inline `Symbol::new(...)` literal.
4. Add a row to the relevant contract's table in this document.
5. Prefer publishing new events directly in the target 3-topic shape
   (`contract`, `action`, `subject`) per the ladder above, rather than the
   legacy 2-topic shape.
6. Run `./scripts/check-event-shape.sh` locally before opening the PR.
