# Vault Storage Layout

This document describes the storage layout of the Callora Vault contract, including storage keys, data types, and access control implications.

## TTL Policy Rationale

This section captures the rationale for every storage tier's TTL constants and their relationship to the cross-contract policy defined in [`docs/STORAGE_TTL_DOCTOR.md`](../../docs/STORAGE_TTL_DOCTOR.md).

### Why named constants matter

Magic-number literals (`50000`, etc.) make audits error-prone and prevent the TTL doctor script from validating expected values against live on-chain state. Every `extend_ttl` call in this contract must reference a named constant so the doctor can load the expected values from the policy table.

### Ledger rate assumption

**17 280 ledgers/day** (5-second close time on Stellar mainnet). All TTL values below use this rate.

### Instance storage (long-lived config)

| Constant                  | Value (ledgers)           | Approximate Duration | Rationale |
| ------------------------- | ------------------------- | -------------------- | --------- |
| `INSTANCE_BUMP_THRESHOLD` | `17_280 × 30` = 518 400  | ~30 days             | Bump fires when fewer than 30 days of TTL remain, giving operators a large observation window before archival. |
| `INSTANCE_BUMP_AMOUNT`    | `17_280 × 60` = 1 036 800 | ~60 days             | Each bump doubles the window. Minimises on-chain write frequency while keeping archival risk low. |

All critical vault state (Admin, Balance, Settlement, RevenuePool, MaxDeduct, Paused, Metadata, DepositorList, …) lives in instance storage. To prevent archival on infrequently-used vaults, every **mutating** entrypoint calls `env.storage().instance().extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT)`.

Entrypoints that bump instance TTL: `init`, `deposit`, `deduct`, `batch_deduct`, `withdraw`, `withdraw_to`, `set_allowed_depositor`, `set_authorized_caller`, `set_settlement`, `set_revenue_pool`, `set_max_deduct`, `set_metadata`, `upgrade`, `pause`, `unpause`, `set_reserve_cap`.

Pure view functions (`get_meta`, `balance`, `get_admin`, `get_usdc_token`, `get_settlement`, `get_revenue_pool`, `get_contract_addresses`, `is_paused`, `is_authorized_depositor`, `get_metadata`, `get_max_deduct`, `get_allowed_depositors`, `is_request_processed`, `get_reserve_cap`) do **not** bump the TTL — they are read-only and incur no write cost.

### Persistent storage — request-id idempotency markers

| Constant                    | Value (ledgers)          | Approximate Duration | Rationale |
| --------------------------- | ------------------------ | -------------------- | --------- |
| `REQUEST_ID_BUMP_THRESHOLD` | `17_280 × 7` = 120 960  | ~7 days              | Idempotency markers must outlive the client retry window. 7 days covers typical backend re-submission timeouts. |
| `REQUEST_ID_BUMP_AMOUNT`    | `17_280 × 30` = 518 400 | ~30 days             | 30-day bump is a best-effort deduplication guarantee. After expiry the marker auto-archives and the `request_id` can be reused (callers requiring longer windows must track off-chain). |

`StorageKey::ProcessedRequest(Symbol)` uses **temporary storage** — it auto-archives after TTL expiry; no manual cleanup is required.

### Persistent storage — rate-limit token bucket

| Constant                  | Value (ledgers)          | Approximate Duration | Rationale |
| ------------------------- | ------------------------ | -------------------- | --------- |
| `RATE_LIMIT_BUMP_THRESHOLD` | `17_280 × 7` = 120 960 | ~7 days              | Rate-limit state must persist across refill windows. 7 days exceeds any realistic refill period. |
| `RATE_LIMIT_BUMP_AMOUNT`   | `17_280 × 30` = 518 400 | ~30 days             | Aligns with idempotency-marker policy: short-lived persistent records use a 30-day bump. |

`StorageKey::DeveloperState(Address)` persists the token-bucket state. Constants declared in `rate_limit.rs` as `RATE_LIMIT_BUMP_THRESHOLD` and `RATE_LIMIT_BUMP_AMOUNT`.

### Persistent storage — reserve caps

Reserve cap entries (`StorageKey::ReserveCap(Address)`) are admin configuration. They follow the **instance storage policy** (30-day threshold, 60-day bump) because they are long-lived and critical. The `set_reserve_cap` entrypoint calls `extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT)` on the cap key.

> **Note:** `INSTANCE_BUMP_THRESHOLD` and `INSTANCE_BUMP_AMOUNT` must be declared at the crate root (e.g. in `lib.rs`) for the `limits.rs` module to import them. The current simplified `lib.rs` does not declare these constants — this is a tracked bug (see `docs/STORAGE_TTL_DOCTOR.md` Action Items).

### Cross-contract policy alignment

This vault's TTL constants are the **reference values** for the cross-contract target policy documented in [`docs/STORAGE_TTL_DOCTOR.md`](../../docs/STORAGE_TTL_DOCTOR.md). Specifically:

- `INSTANCE_BUMP_THRESHOLD = 17_280 * 30` and `INSTANCE_BUMP_AMOUNT = 17_280 * 60` are the target for all long-lived instance and persistent entries across Vault, Settlement, and Revenue Pool.
- `RATE_LIMIT_BUMP_THRESHOLD` / `RATE_LIMIT_BUMP_AMOUNT` are the target for short-lived persistent entries (timelocks, migration records).
- The TTL doctor script (`scripts/storage-ttl-doctor.ts`) uses these values as the expected baseline when `--policy` is passed.

---

## Instance Storage TTL

All critical vault state lives in instance storage. To prevent archival on infrequently-used vaults, **every mutating entrypoint** and **every public view function ("hot read path")** calls `env.storage().instance().extend_ttl(threshold, extend_to)`.

Read-path bumps (a.k.a. **buffer #5**) are the critical addition in this revision. A vault that is only queried (no writes for months) must not silently archive. Every `get_*`, `is_*`, and `balance` view call re-ups the instance TTL to the same 60-day target as writes.

| Constant                  | Value                    | Rationale                                                    |
| ------------------------- | ------------------------ | ------------------------------------------------------------ |
| `INSTANCE_BUMP_THRESHOLD` | `17_280 * 30` (~30 days) | Bump is triggered when fewer than 30 days of TTL remain      |
| `INSTANCE_BUMP_AMOUNT`    | `17_280 * 60` (~60 days) | Each bump extends the TTL to 60 days from the current ledger |

Ledger rate assumption: **17 280 ledgers/day** (5-second close time on Stellar mainnet).

**Write entrypoints that bump instance TTL** (at exit, after all writes succeed):
`init`, `deposit`, `deduct`, `batch_deduct`, `withdraw`, `withdraw_to`, `set_authorized_caller`, `pause`, `unpause`, `set_max_deduct`, `set_settlement`, `set_timelock_window`, `set_admin`, `accept_admin`, `propose_pause`, `execute_pause`, `cancel_pause`, `propose_upgrade`, `execute_upgrade`, `cancel_upgrade`, `propose_sweep`, `execute_sweep`, `cancel_sweep`, `prune_processed_requests`, `set_reserve_cap`.

**View entrypoints that bump instance TTL — buffer #5 hot read paths** (at entry, before any reads):
`balance`, `get_owner`, `get_admin`, `get_usdc_token`, `get_max_deduct`, `get_settlement`, `get_revenue_pool`, `get_timelock_window`, `is_paused`, `is_authorized_depositor`, `get_reserve_cap`, `get_pending_pause`, `get_pending_upgrade`, `get_pending_sweep`.

Rationale for buffer #5: production indexers and UIs call `balance()` / `get_admin()` / `is_paused()` many times per day. Without read-path bumps, a vault that only receives reads (no owner-initiated writes for 60 days) would be archived, breaking all reads and requiring an explicit owner touch to revive. Buffer #5 makes "usage = reads OR writes" for TTL accounting.

Intentionally **excluded** from bumps:
- `dry_run_sweep_idle_balance` in `views.rs` — documented as side-effect-free, returns estimated surplus for admin dashboards.

## Persistent Storage TTL

Timelock proposals (`PendingPause`, `PendingUpgrade`, `PendingSweep`), idempotency markers (`ProcessedRequest`), and per-developer rate-limit state live in the **persistent** storage tier. Each entry bumps its own TTL on both write and — for proposal entries — on their corresponding `get_pending_*` view.

| Constant                      | Value                    | Rationale                                                    |
| ----------------------------- | ------------------------ | ------------------------------------------------------------ |
| `PERSISTENT_BUMP_THRESHOLD`   | `17_280 * 30` (~30 days) | Mirrors instance threshold (30 days)                         |
| `PERSISTENT_BUMP_AMOUNT`      | `17_280 * 60` (~60 days) | Mirrors instance amount (60 days)                            |
| `REQUEST_ID_BUMP_THRESHOLD`   | `17_280 * 7` (~7 days)   | Per-key trigger for idempotency markers                      |
| `REQUEST_ID_BUMP_AMOUNT`      | `17_280 * 30` (~30 days) | Per-key target for idempotency markers                       |

**Persistent keys with buffer #5 read-path bumps** (only if the key exists — non-existent keys are skipped):

| Key                            | Written by                           | Bumped on read via             |
| ------------------------------ | ------------------------------------ | ------------------------------ |
| `StorageKey::PendingPause`     | `propose_pause`                      | `get_pending_pause` (if `Some`) |
| `StorageKey::PendingUpgrade`   | `propose_upgrade`                    | `get_pending_upgrade` (if `Some`) |
| `StorageKey::PendingSweep`     | `propose_sweep`                      | `get_pending_sweep` (if `Some`) |

Buffer #5 persistent rationale: an admin may propose a pause/upgrade, then poll `get_pending_*` each day waiting for the timelock to expire. Without read bumps, the proposal itself could archive before its own execute window opens, leaving the admin unable to execute even though they "used" the contract every day.

## Processed-Request Idempotency Storage (Temporary)

Idempotency markers for `deduct` and `batch_deduct` live in **persistent storage**. They do **not** silently archive; an owner MUST explicitly prune them with `prune_processed_requests` to recover state space.

| Constant                    | Value                    | Rationale                                                    |
| --------------------------- | ------------------------ | ------------------------------------------------------------ |
| `REQUEST_ID_BUMP_THRESHOLD` | `17_280 * 7` (~7 days)   | Bump is triggered when fewer than 7 days of TTL remain       |
| `REQUEST_ID_BUMP_AMOUNT`    | `17_280 * 30` (~30 days) | Each bump extends the TTL to 30 days from the current ledger |

### Key: `StorageKey::ProcessedRequest(Symbol)`

- **Storage tier:** Temporary (auto-archived after TTL expires)
- **Value type:** `bool` (`true`); presence of the key is the authoritative signal
- **Written by:** `deduct` and `batch_deduct` on every **successful** deduction where `request_id` is `Some(id)`
- **Read by:** `deduct`, `batch_deduct` (duplicate check), `is_request_processed` (view)
- **TTL:** Set to `REQUEST_ID_BUMP_AMOUNT` (~30 days) on write; bumped on every successful re-use within the threshold window

### Retention Policy

| Scenario                      | Behaviour                                            |
| ----------------------------- | ---------------------------------------------------- |
| First deduct with `Some(id)`  | Marker written; TTL set to ~30 days                  |
| Retry within retention window | `DuplicateRequestId` error returned; no state change |
| Retry after TTL expires       | Marker archived; deduct treated as new (succeeds)    |
| Deduct with `None`            | No marker written; no deduplication                  |
| Failed deduct (any error)     | No marker written; id remains reusable               |

> **Caller guidance:** Backends should treat `VaultError::DuplicateRequestId` as a successful no-op — the original deduction already went through. Do not retry with a new `request_id` for the same logical operation.

> **Retention window:** The 30-day window is a best-effort guarantee. After expiry the marker is archived and the `request_id` can be reused. Callers requiring longer deduplication windows must implement their own off-chain tracking.

## Storage Overview

The Callora Vault contract uses Soroban's instance storage to persist contract state. Data is organized using the `StorageKey` enum, providing type-safe access to contract state.

## Storage Keys

The contract defines the following storage keys:

```rust
#[contracttype]
pub enum StorageKey {
    MetaKey,                       // VaultMeta
    Admin,                         // Address
    UsdcToken,                     // Address
    Settlement,                    // Address
    RevenuePool,                   // Option<Address>
    MaxDeduct,                     // i128
    Paused,                        // bool
    Metadata(String),              // String (offering metadata by offering_id)
    PendingOwner,                  // Address
    PendingAdmin,                  // Address
    AllowedDepositors,             // Vec<Address> — instance storage, deposit allowlist
    ContractVersion,               // BytesN<32>
    ProcessedRequest(Symbol),      // bool — persistent storage, idempotency marker
}
```

### Storage Keys Table

| Key Variant                | Storage Tier  | Value Type        | Description                                            | Access                                                                     |
| -------------------------- | ------------- | ----------------- | ------------------------------------------------------ | -------------------------------------------------------------------------- |
| `MetaKey`                  | Instance      | `VaultMeta`       | Owner, balance, authorized_caller, min_deposit         | `get_meta()`, updated by deposit/deduct/withdraw                           |
| `Admin`                    | Instance      | `Address`         | Administrator address                                  | `get_admin()`, `set_admin()`                                               |
| `UsdcToken`                | Instance      | `Address`         | USDC token contract address                            | Set during `init()`                                                        |
| `Settlement`               | Instance      | `Address`         | Settlement contract; receives USDC on deduct           | `set_settlement()`, `get_settlement()`                                     |
| `RevenuePool`              | Instance      | `Option<Address>` | Revenue pool address (informational)                   | `set_revenue_pool()`, `get_revenue_pool()`                                 |
| `MaxDeduct`                | Instance      | `i128`            | Maximum USDC per single deduct                         | Set during `init()`, read by `deduct()` / `batch_deduct()`                 |
| `Paused`                   | Instance      | `bool`            | Circuit-breaker flag                                   | `pause()`, `unpause()`, `is_paused()`                                      |
| `Metadata(String)`         | Instance      | `String`          | Per-offering metadata (IPFS CID / URI)                 | `set_metadata()`, `get_metadata()`, `update_metadata()`                    |
| `Price(String)`            | Instance      | `String`          | Per-offering price string for the offering ID          | `set_price()`, `get_price()`, `remove_price()`                             |
| `OfferingIndex`            | Instance      | `Vec<String>`     | Ordered list of offering IDs with stored prices        | `set_price()`, `remove_price()`, `list_prices()`                           |
| `PendingOwner`             | Instance      | `Address`         | Two-step ownership transfer nominee                    | `transfer_ownership()`, `accept_ownership()`                               |
| `PendingAdmin`             | Instance      | `Address`         | Two-step admin transfer nominee                        | `set_admin()`, `accept_admin()`                                            |
| `Depositor(Address)` / `DepositorIndex` | — | — | **Removed (#1110)** — never written; the canonical allowlist entry is `AllowedDepositors` below | — |
| `AllowedDepositors`          | Instance      | `Vec<Address>`    | Addresses permitted to deposit (owner bypasses)       | `add_address()`, `clear_all()`, `deposit()`, `is_authorized_depositor()` |
| `ContractVersion`          | Instance      | `BytesN<32>`      | WASM hash set by `upgrade()`                           | `upgrade()`, `version()`                                                   |
| `ProcessedRequest(Symbol)` | **Temporary** | `bool`            | Idempotency marker for a processed deduct `request_id` | Written by `deduct()` / `batch_deduct()`; read by `is_request_processed()` |
| Key Variant | Storage Tier | Value Type | Description | Access |
|-------------|-------------|-----------|-------------|--------|
| `MetaKey` | Instance | `VaultMeta` | Owner, balance, authorized_caller, min_deposit | `get_meta()`, updated by deposit/deduct/withdraw |
| `Admin` | Instance | `Address` | Administrator address | `get_admin()`, `set_admin()` |
| `UsdcToken` | Instance | `Address` | USDC token contract address | Set during `init()` |
| `Settlement` | Instance | `Address` | Settlement contract; receives USDC on deduct | `set_settlement()`, `get_settlement()` |
| `RevenuePool` | Instance | `Option<Address>` | Revenue pool address (informational) | `set_revenue_pool()`, `get_revenue_pool()` |
| `MaxDeduct` | Instance | `i128` | Maximum USDC per single deduct | Set during `init()`, read by `deduct()` / `batch_deduct()` |
| `Paused` | Instance | `bool` | Circuit-breaker flag | `pause()`, `unpause()`, `is_paused()` |
| `Metadata(String)` | Instance | `String` | Per-offering metadata (IPFS CID / URI) | `set_metadata()`, `get_metadata()`, `update_metadata()` |
| `PendingOwner` | Instance | `Address` | Two-step ownership transfer nominee | `transfer_ownership()`, `accept_ownership()` |
| `PendingAdmin` | Instance | `Address` | Two-step admin transfer nominee | `set_admin()`, `accept_admin()` |
| `DepositorList` (renamed `AllowedDepositors`) | Instance | `Vec<Address>` | Allowed depositor addresses | `add_address()`, `clear_all()`, `get_allowlist()` |
| `ContractVersion` | Instance | `BytesN<32>` | WASM hash set by `upgrade()` | `upgrade()`, `version()` |
| `ProcessedRequest(Symbol)` | **Persistent** | `bool`            | Idempotency marker for a processed deduct `request_id` | Written by `deduct()` / `batch_deduct()`; read by `is_request_processed()` |
| `LifetimeDeposit(Address)` | **Persistent** | `i128`            | Cumulative USDC ever deposited by a given address; never decrements | Written by `deposit()`; read by `get_lifetime_deposit()` / `list_lifetime_deposits()` |
| `LifetimeDepositorIndex`   | Instance       | `Vec<Address>`    | Ordered list of all addresses that have deposited at least once; used for paginated listing | Written by `deposit()` on first deposit per address |

## Data Structures

### VaultMeta

```rust
#[contracttype]
#[derive(Clone)]
pub struct VaultMeta {
    pub owner: Address,                    // Vault owner; always permitted to deposit
    pub balance: i128,                     // Current vault balance (USDC units)
    pub authorized_caller: Option<Address>, // Optional address authorized to call deduct/batch_deduct
    pub min_deposit: i128,                 // Minimum amount required per deposit
}
```

**Fields:**

- `owner`: `Address` - The vault owner; immutable except via `transfer_ownership()`; always permitted to deposit; can set allowed depositors and manage metadata
- `balance`: `i128` - Current vault balance in smallest USDC units; incremented by deposits, decremented by deducts/withdrawals
- `authorized_caller`: `Option<Address>` - Optional address permitted to trigger `deduct()` and `batch_deduct()` operations; can be set via `set_authorized_caller()`
- `min_deposit`: `i128` - Minimum required per deposit; configured at initialization; prevents dust deposits and rejects zero or sub-unit transfer requests on the deposit path. The same floor is reused as the per-call minimum for `deduct`/`batch_deduct` items and for `propose_sweep`, so every entrypoint that moves USDC out of or into the vault rejects sub-unit/dust amounts consistently (`propose_sweep` returns `VaultError::BelowMinTransferAmount` when `amount` is below this floor)

### DeductItem

```rust
#[contracttype]
#[derive(Clone)]
pub struct DeductItem {
    pub amount: i128,
    pub request_id: Option<Symbol>,
}
```

Used in `batch_deduct()` to represent individual deduction requests.

## Storage Operations

### Initialization

**Function:** `init()`

Sets up the vault with initial state:

- `StorageKey::Meta` ← `VaultMeta { owner, balance: initial_balance, authorized_caller, min_deposit }`
- `StorageKey::UsdcToken` ← USDC token address
- `StorageKey::Admin` ← owner address (initially)
- `StorageKey::RevenuePool` ← optional revenue pool address
- `StorageKey::MaxDeduct` ← max deduct cap (or `DEFAULT_MAX_DEDUCT` if not specified)

### Core Vault Operations

| Operation                       | Reads                                                          | Writes                                                                         | Authorization              |
| ------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------------ | -------------------------- |
| `deposit(amount)`               | MetaKey, DepositorList                                         | MetaKey (balance += amount)                                                    | Owner or AllowedDepositor  |
| `deduct(amount, request_id)`    | MetaKey, MaxDeduct, Settlement, ProcessedRequest(id)?          | MetaKey (balance -= amount); ProcessedRequest(id) if Some; transfers USDC      | Owner or authorized_caller |
| `batch_deduct(items)`           | MetaKey, MaxDeduct, Settlement, ProcessedRequest(id)? per item | MetaKey (balance -= total); ProcessedRequest(id) per Some item; transfers USDC | Owner or authorized_caller |
| `withdraw(amount)`              | MetaKey, UsdcToken                                             | MetaKey (balance -= amount); transfers USDC to owner                           | Owner only                 |
| `withdraw_to(to, amount)`       | MetaKey, UsdcToken                                             | MetaKey (balance -= amount); transfers USDC to `to`                            | Owner only                 |
| `sweep_idle_balance(to, amount)`| MetaKey, UsdcToken, Settlement or RevenuePool                  | MetaKey (balance -= amount); transfers USDC to destination                     | Owner only                 |
| `balance()`                     | MetaKey                                                        | —                                                                              | Public read                |
| `transfer_ownership(new_owner)` | MetaKey                                                        | PendingOwner                                                                   | Owner only                 |

### Admin Operations

| Operation                | Reads            | Writes                                      | Authorization |
| ------------------------ | ---------------- | ------------------------------------------- | ------------- |
| `distribute(to, amount)` | Admin, UsdcToken | — (USDC transfer only, no balance tracking) | Admin only    |
| `set_admin(new_admin)`   | Admin            | Admin                                       | Admin only    |

### Access Control Operations

| Operation                          | Reads                   | Writes                               | Authorization |
| ---------------------------------- | ----------------------- | ------------------------------------ | ------------- |
| `add_address(depositor)` / `clear_all()` | AllowedDepositors       | AllowedDepositors (append or remove) | Owner only    |
| `set_authorized_caller(caller)` | Meta                    | Meta (authorized_caller field)       | Owner only    |
| `is_authorized_depositor(caller)`  | Owner, AllowedDepositors | —                                    | Public read   |

### Settlement & Routing

| Operation                            | Reads       | Writes      | Authorization                        |
| ------------------------------------ | ----------- | ----------- | ------------------------------------ |
| `set_settlement(settlement_address)` | Admin       | Settlement  | Admin only                           |
| `get_settlement()`                   | Settlement  | —           | Public read (view-only, no mutation) |
| `set_revenue_pool(revenue_pool)`     | Admin       | RevenuePool | Admin only                           |
| `get_revenue_pool()`                 | RevenuePool | —           | Public read (view-only, no mutation) |

**Deduct Routing Logic:**

1. If `StorageKey::Settlement` is set: transfer USDC to settlement
2. Else if `StorageKey::RevenuePool` is set: transfer USDC to revenue pool
3. Else: USDC remains in vault

**View Function Safety:**

- Both `get_settlement()` and `get_revenue_pool()` are read-only operations
- They return only final committed state, never intermediate or pending values
- Safe for external indexers and off-chain queries
- Deterministic: identical state inputs always produce identical outputs
- `get_settlement()` panics if not configured; `get_revenue_pool()` returns `None` gracefully

### Metadata Operations

| Operation                                | Reads                       | Writes                | Authorization |
| ---------------------------------------- | --------------------------- | --------------------- | ------------- |
| `set_metadata(offering_id, metadata)`    | Meta                        | Metadata(offering_id) | Owner only    |
| `get_metadata(offering_id)`              | Metadata(offering_id)       | —                     | Public read   |
| `update_metadata(offering_id, metadata)` | Meta, Metadata(offering_id) | Metadata(offering_id) | Owner only    |

**Metadata Notes:**

- Metadata is stored per offering (keyed by `offering_id`)
- Typical usage: store IPFS CID or HTTPS URI for offering details
- Maximum string length: no hard limit enforced, but should be kept reasonable
- Empty strings are allowed

### Read Operations

```
Instance Storage
├── StorageKey::Meta
│   └── VaultMeta
│       ├── owner: Address
│       ├── balance: i128
│       ├── authorized_caller: Option<Address>
│       └── min_deposit: i128
├── StorageKey::UsdcToken
│   └── Address
├── StorageKey::Admin
│   └── Address
├── StorageKey::AllowedDepositors (optional)
│   └── Vec<Address>
├── StorageKey::Settlement (optional)
│   └── Address
├── StorageKey::RevenuePool (optional)
│   └── Address
├── StorageKey::MaxDeduct
│   └── i128
└── StorageKey::Metadata(offering_id_1..N) (optional, multiple entries)
    └── String
```

## Migration and Upgrade Notes

### Post-Refactor Changes

The following changes were made in the recent refactor:

1. **VaultMeta Structure Expansion**
   - Added `authorized_caller: Option<Address>` field for designated deduct authorization
   - Added `min_deposit: i128` field for deposit minimum enforcement
   - Old deployments must migrate existing `VaultMeta` to include these new fields with appropriate defaults

2. **Storage Key Consolidation**
   - All admin-related keys (Admin, UsdcToken, Settlement, RevenuePool, MaxDeduct) now use the `StorageKey` enum
   - Previously may have used Symbol-based keys
   - Migration: read from old Symbol keys, write to new enum keys

3. **AllowedDepositors Structure Change**
   - Now `Vec<Address>` instead of single optional address
   - Allows multiple authorized depositors
   - Supports add/remove operations without replacing the entire collection

4. **Metadata System**
   - `StorageKey::Metadata(String)` replaces hardcoded offering metadata patterns
   - Enables flexible per-offering metadata storage

### Migration Strategy for Existing Deployments

If upgrading from a pre-refactor version, use the following pattern:

```rust
// 1. Read old VaultMeta (owner, balance only)
let old_meta = env.storage().instance().get(&StorageKey::Meta);

// 2. Create new VaultMeta with migrations
let new_meta = VaultMeta {
    owner: old_meta.owner,
    balance: old_meta.balance,
    authorized_caller: None,  // Set by owner post-upgrade via set_authorized_caller()
    min_deposit: 0,           // Default to 0; can be reset if needed
};

// 3. Write new structure back
env.storage().instance().set(&StorageKey::Meta, &new_meta);

// 4. Migrate other storage keys as needed
// (e.g., from Symbol("usdc") to StorageKey::UsdcToken)
```

## Security Considerations

### Access Control

- **Owner-Only Operations:** `add_address()`, `clear_all()`, `set_authorized_caller()`, `transfer_ownership()`, `withdraw()`, `withdraw_to()`, metadata operations
- **Admin-Only Operations:** `distribute()`, `set_admin()`, `set_settlement()`, `set_revenue_pool()`
- **Public Operations:** `balance()`, `get_meta()`, `get_metadata()`, `is_authorized_depositor()`, `get_settlement()`, `get_revenue_pool()` (all read-only)
- **Depositor Operations:** `deposit()` (owner or allowed depositor); `deduct()` and `batch_deduct()` (owner or authorized_caller)

### Data Integrity

- `VaultMeta` is updated atomically; all fields are modified together for consistency
- Balance operations include assertions to prevent underflow and enforce non-negative constraints
- Storage writes are transactional within Soroban; partial writes are not possible
- Authorization is validated before any state mutations

### Deduct Safety

- Single deduct amount capped by `StorageKey::MaxDeduct` to prevent excessive USDC transfers
- Batch deduct validates all items before applying any deductions (all-or-nothing semantics)
- Balance underflow prevention: all attempted deductions are validated before modifying state

## Testing

### Storage Access Patterns

The test suite validates:

- Initialization sets all required storage keys
- Deposit updates balance correctly
- Deduct routes to settlement/revenue pool as configured
- Batch operations update balance atomically
- Metadata operations (set, get, update) work correctly
- AllowedDepositors Vec operations (add, remove)
- Access control is enforced for owner-only and admin-only operations

### Recommended Additional Tests

- Metadata size limits and edge cases
- Settlement vs. RevenuePool routing priority
- Authorized caller deduction scenarios
- Balance overflow/underflow edge cases (max i128, min i128)
- Storage upgrade/downgrade compatibility
- Gas usage benchmarks for storage operations

## Monitoring and Debugging

### Storage Inspection

Use Soroban CLI to inspect storage:

```bash
soroban contract storage \
  --contract-id <CONTRACT_ID> \
  --key "meta" \
  --output json
```

### Event Monitoring

Monitor storage-related events:

- `init` events for vault creation
- Future events could track significant balance changes

## Version History

| Version | Change                                                                                                                                                                                                                                                                        |
| ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1.0     | Initial `StorageKey` enum with `Meta`, `AllowedDepositors`, `Admin`, `UsdcToken`, `Settlement`, `RevenuePool`, `MaxDeduct`, `Metadata(String)`                                                                                                                                |
| 1.1     | Renamed `StorageKey` → `DataKey`; added doc comments to all variants; removed stale `// Replaced by StorageKey enum variants` comment; updated STORAGE.md                                                                                                                     |
| 1.2     | Added `StorageKey::ProcessedRequest(Symbol)` in **temporary storage** for `request_id` idempotency in `deduct` and `batch_deduct`. Added `VaultError::DuplicateRequestId` (code 28). Added `is_request_processed(request_id)` view. TTL: threshold ~7 days, bump to ~30 days. |
| 1.4     | **Buffer #5 — TTL bump on hot read paths.** Added public TTL constants (`LEDGERS_PER_DAY`, `INSTANCE_BUMP_THRESHOLD/AMOUNT`, `PERSISTENT_BUMP_THRESHOLD/AMOUNT`, `REQUEST_ID_BUMP_THRESHOLD/AMOUNT`). Instance TTL now bumped at **entry** of EVERY public view call (`balance`, `get_*`, `is_*`) so read-only usage keeps vault alive. Persistent `PendingPause/PendingUpgrade/PendingSweep` keys bumped by `get_pending_*` getters when the proposal exists. Write entrypoints continue to bump at exit. Added new `VaultError` codes 44-47 (proposal/timelock errors) and declared `pub mod timelock`. |
| 1.5     | Issue #1110 — removed never-written `DataKey::Depositor(Address)` and `DataKey::AllowedDepositorsList`. `is_authorized_depositor()` now reads `Owner` + `StorageKey::AllowedDepositors` through the same private helper as `deposit()`, so the view and the deposit gate can no longer diverge (owner included; documented). |
| Version | Change |
|---------|--------|
| 1.0 | Initial `StorageKey` enum with `Meta`, `AllowedDepositors`, `Admin`, `UsdcToken`, `Settlement`, `RevenuePool`, `MaxDeduct`, `Metadata(String)` |
| 1.1 | Renamed `StorageKey` → `DataKey`; added doc comments to all variants; removed stale `// Replaced by StorageKey enum variants` comment; updated STORAGE.md |
| 1.2 | Added `StorageKey::ProcessedRequest(Symbol)` in **persistent storage** for `request_id` idempotency in `deduct` and `batch_deduct`. Added `VaultError::DuplicateRequestId` (code 28). Added `is_request_processed(request_id)` view. TTL: threshold ~7 days, bump to ~30 days. |
| 1.3 | Added `StorageKey::LifetimeDeposit(Address)` (persistent, TTL ~1 year) and `StorageKey::LifetimeDepositorIndex` (instance) for per-depositor cumulative deposit tracking. Added `get_lifetime_deposit(addr)` and `list_lifetime_deposits(cursor, limit)` view functions. Page cap: 100. Overflow-protected via `checked_add`. |
| 1.4 | **Buffer #5 — TTL bump on hot read paths.** Instance TTL bumped at entry of EVERY public view call (`balance`, `get_*`, `is_*`, `get_pending_*`) so read-only usage keeps vault alive. Persistent timelock proposal keys also bumped by getters when present. All mutating entrypoints continue to bump instance TTL at exit. Added TTL constants (all `pub`). Declared `pub mod timelock`; added `VaultError` codes 44 (ProposalNotFound), 45 (TimelockNotExpired), 46 (TimelockOverflow), 47 (InvalidTimelockWindow). |

## Canonical Storage Keys

All storage is accessed via `StorageKey` enum.

### Keys

| Key                        | Storage Tier  | Description                                                     |
| -------------------------- | ------------- | --------------------------------------------------------------- |
| `MetaKey`                  | Instance      | Vault metadata (owner, balance, authorized_caller, min_deposit) |
| `DepositorList`            | Instance      | Authorized depositors                                           |
| `Admin`                    | Instance      | Admin address                                                   |
| `UsdcToken`                | Instance      | Token contract                                                  |
| `Settlement`               | Instance      | Settlement contract                                             |
| `RevenuePool`              | Instance      | Revenue pool                                                    |
| `MaxDeduct`                | Instance      | Deduct cap                                                      |
| `Paused`                   | Instance      | Circuit breaker                                                 |
| `Metadata(String)`         | Instance      | Offering metadata                                               |
| `PendingOwner`             | Instance      | Ownership transfer nominee                                      |
| `PendingAdmin`             | Instance      | Admin transfer nominee                                          |
| `ContractVersion`          | Instance      | WASM hash (set by `upgrade()`)                                  |
| `ProcessedRequest(Symbol)` | **Temporary** | Idempotency marker; auto-expires after ~30 days                 |
| Key | Storage Tier | Description |
|-----|-------------|------------|
| `MetaKey` | Instance | Vault metadata (owner, balance, authorized_caller, min_deposit) |
| `DepositorList` | Instance | Authorized depositors |
| `Admin` | Instance | Admin address |
| `UsdcToken` | Instance | Token contract |
| `Settlement` | Instance | Settlement contract |
| `RevenuePool` | Instance | Revenue pool |
| `MaxDeduct` | Instance | Deduct cap |
| `Paused` | Instance | Circuit breaker |
| `Metadata(String)` | Instance | Offering metadata |
| `PendingOwner` | Instance | Ownership transfer nominee |
| `PendingAdmin` | Instance | Admin transfer nominee |
| `ContractVersion` | Instance | WASM hash (set by `upgrade()`) |
| `ProcessedRequest(Symbol)` | **Persistent** | Idempotency marker; manually pruned |

### Migration

- Removes deprecated `AllowedDepositors`
- Ensures Admin fallback from Meta.owner
- `ProcessedRequest` uses temporary storage — no manual cleanup required; markers expire automatically
- `ProcessedRequest` uses persistent storage — markers must be explicitly pruned using `prune_processed_requests` to avoid state bloat
