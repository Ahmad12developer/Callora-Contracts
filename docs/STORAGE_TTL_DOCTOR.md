# Storage TTL Doctor Utility

The Storage TTL Doctor is a CLI utility that reports the remaining Time-To-Live (TTL) of the Callora smart contracts' storage entries so operators can trigger extensions (bumps) before data is archived.

In Soroban, storage entries (such as instance config or developer balances in persistent storage) are automatically archived once their TTL expires.

---

## Where live TTLs come from

**Live TTLs are read from Soroban RPC `getLedgerEntries`, not from a contract view.**

Each `LedgerEntry` returned by `getLedgerEntries` carries:

* `liveUntilLedgerSeq` — the future ledger number at which the entry expires;
* and the enclosing response carries `latestLedger`.

Remaining TTL is therefore `liveUntilLedgerSeq - latestLedger`.

This split exists because **contract code cannot observe the remaining TTL of a ledger entry**. A contract view that claims to report one can only return a constant, so an operator cannot distinguish a healthy entry from one about to be archived. The `scripts/storage-ttl-doctor.ts` revenue-pool path reads the policy from the contract and the live TTL from `getLedgerEntries`.

`docs/interfaces/revenue_pool.json` and the contract source are the source of truth for the view signatures below.

---

## Unified TTL Policy Table

> **Ledger rate assumption:** 17 280 ledgers/day (5-second close time on Stellar mainnet).
> All threshold/bump values below use this rate unless otherwise noted.

| Contract | Storage Class | Key / Category | Threshold (ledgers) | Bump Amount (ledgers) | Approx. Threshold | Approx. Bump | Rationale | Status |
|---|---|---|---|---|---|---|---|---|
| Vault | Instance | All `DataKey::*` instance entries (Owner, Balance, Settlement, …) | 518 400 (`17_280 × 30`) | 1 036 800 (`17_280 × 60`) | ~30 days | ~60 days | Vault config is long-lived; a 30-day warning window gives operators ample time to top up. Each bump doubles the window to 60 days to minimise on-chain calls. | ✅ Defined (STORAGE.md / test.rs ref; missing in current lib.rs — **FLAG: undefined in source**) |
| Vault | Persistent | `DataKey::ProcessedRequest(Symbol)` — request-id idempotency marker | 120 960 (`17_280 × 7`) | 518 400 (`17_280 × 30`) | ~7 days | ~30 days | Temporary idempotency markers need only survive the retry window. Auto-archived after expiry; no manual cleanup required. | ✅ Defined in STORAGE.md |
| Vault | Persistent | `StorageKey::DeveloperState(Address)` — rate-limit token bucket | 120 960 (`17_280 × 7`) (`RATE_LIMIT_BUMP_THRESHOLD`) | 518 400 (`17_280 × 30`) (`RATE_LIMIT_BUMP_AMOUNT`) | ~7 days | ~30 days | Rate-limit state must persist across the refill window. Constants named and declared in `rate_limit.rs`. | ✅ Defined in `rate_limit.rs` |
| Vault | Instance | `StorageKey::ReserveCap(Address)` — per-token reserve cap | 518 400 (`17_280 × 30`) (`INSTANCE_BUMP_THRESHOLD`) | 1 036 800 (`17_280 × 60`) (`INSTANCE_BUMP_AMOUNT`) | ~30 days | ~60 days | Reserve caps are admin config; bump follows instance policy. Constants imported from crate root but **not defined there** — **FLAG: undefined constant bug**. | ⚠️ Imported but not declared |
| Vault | — | No-bump instance (base `lib.rs`) | — | — | — | — | The simplified `lib.rs` never calls `extend_ttl` on instance storage. Any `DataKey::*` entry will expire when the Soroban ledger-assigned TTL lapses. | ❌ **NO TTL bump — archival risk** |
| Vault | — | `helpers` crate | — | — | — | — | The helpers crate contains no storage access. No TTL handling required. | ℹ️ N/A (no storage) |
| Settlement | Persistent | `StorageKey::DeveloperBalance(Address, Address)` — per-token developer balance | 50 000 (~2.9 days) | 50 000 (~2.9 days) | ~2.9 days | ~2.9 days | **Mismatch vs. policy.** Literal 50 000 used everywhere (receive_payment, batch_receive_payment, admin migration). Very short window — **archival risk for infrequently-touched developer balances**. Target: raise to `17_280 × 30` / `17_280 × 60`. | ⚠️ Literal — too short |
| Settlement | Persistent | `StorageKey::DeveloperMinBalance(Address)` — per-developer minimum balance floor | 50 000 (~2.9 days) | 50 000 (~2.9 days) | ~2.9 days | ~2.9 days | Same literal pattern as `DeveloperBalance`. **Archival risk.** Target: raise to `17_280 × 30` / `17_280 × 60`. | ⚠️ Literal — too short |
| Settlement | Persistent | `StorageKey::PendingDeveloperMigration(Address)` — timelock'd admin migration record | 50 000 (~2.9 days) | 50 000 (~2.9 days) | ~2.9 days | ~2.9 days | Migration records must survive the 24-hour timelock (`DEVELOPER_MIGRATION_TIMELOCK_SECONDS = 86_400`). 2.9 days is sufficient but inconsistent. Target: align to `17_280 × 7` / `17_280 × 30`. | ⚠️ Literal — inconsistent |
| Settlement | Temporary | `DataKey::ArchivedEvent(Address, u64)` — FIFO event archive | 17 280 (`MIN_TTL_LEDGERS`, ~1 day) | 3 110 400 (`ARCHIVE_TTL_LEDGERS`, ~6 months) | ~1 day | ~6 months | Archived events are long-retention read artefacts. 6-month TTL accommodates audit and analytics pipelines. Temporary storage auto-expires — intentional. Constants declared in `archive.rs`. | ✅ Defined, intentional |
| Settlement | Persistent | `DataKey::Cursor(Address)` — FIFO archive cursor | 17 280 (~1 day) | 3 110 400 (~6 months) | ~1 day | ~6 months | Cursor must persist as long as any associated archived event. Same constants as `ArchivedEvent`. | ✅ Defined in `archive.rs` |
| Settlement | Instance | All `StorageKey::*` instance entries (Admin, Vault, GlobalPool, …) | — | — | — | — | Settlement instance storage has **no `extend_ttl` call** in any path. Instance entries will expire per Soroban default. **FLAG: archival risk for core config**. | ❌ **NO TTL bump — archival risk** |
| Revenue Pool | Instance | All `DataKey::*` instance entries | `LIFETIME_THRESHOLD` (undefined) | `BUMP_AMOUNT` (undefined) | — | — | `extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT)` is called in `propose_emergency_drain`, `execute_emergency_drain`, and `cancel_emergency_drain` — but **neither constant is declared anywhere in the crate**. This will fail to compile. **FLAG: undefined constants — critical bug**. | ❌ **Undefined constants — compilation bug** |
| Helpers | — | (no storage) | — | — | — | — | `contracts/helpers` contains only shared utility functions (`snapshot_diff.rs`). No storage keys or TTL handling needed. | ℹ️ N/A (no storage) |

### Status Key

| Symbol | Meaning |
|--------|---------|
| ✅ | Defined, named constants, consistent with target policy |
| ⚠️ | Present but uses magic literals or mismatches target policy |
| ❌ | Missing, undefined, or creates archival risk |
| ℹ️ | Not applicable |

---

## Target TTL Policy

The following policy should be adopted uniformly across all crates. All values use the 17 280 ledgers/day rate.

| Storage Class | Key Lifetime | Threshold Constant | Threshold Value | Bump Constant | Bump Value |
|---|---|---|---|---|---|
| Instance (config / critical state) | Long-lived | `INSTANCE_BUMP_THRESHOLD` | `17_280 * 30` (518 400) | `INSTANCE_BUMP_AMOUNT` | `17_280 * 60` (1 036 800) |
| Persistent (developer balances, caps) | Medium-lived | `PERSISTENT_BUMP_THRESHOLD` | `17_280 * 30` (518 400) | `PERSISTENT_BUMP_AMOUNT` | `17_280 * 60` (1 036 800) |
| Persistent (short-lived records: rate-limit state, migration timelocks) | Short-lived | `SHORT_BUMP_THRESHOLD` | `17_280 * 7` (120 960) | `SHORT_BUMP_AMOUNT` | `17_280 * 30` (518 400) |
| Temporary (idempotency markers, archived events) | Auto-expires | `TEMP_BUMP_THRESHOLD` | `17_280 * 7` (120 960) | `TEMP_BUMP_AMOUNT` | `17_280 * 30` (518 400) |

### Policy Rationale

1. **Instance storage (30-day threshold → 60-day bump):** Critical contract config (Admin, Vault, Settlement, pool addresses) must never be archived. A 30-day warning window gives ample time for operators to intervene. Each bump extends to 60 days, halving on-chain extension costs.

2. **Persistent developer balances (30-day threshold → 60-day bump):** Developer balances are the primary financial state. The current 50 000 literal (~2.9 days) is dangerously short for infrequently-accessed accounts. Raising to 30-day threshold matches instance policy and drastically reduces archival risk.

3. **Short-lived persistent records (7-day threshold → 30-day bump):** Rate-limit token buckets and migration timelocks are ephemeral but must outlive their operational window. A 7-day threshold exceeds the 24-hour timelock and the retry windows used in the system.

4. **Temporary storage (7-day threshold → 30-day bump):** Auto-expires by design. Threshold ensures the entry remains readable for retry deduplication and archive-pipeline ingestion windows.

5. **Named constants over magic literals:** Every `extend_ttl` call must reference a named constant. Magic number literals such as `50000` make audits error-prone and prevent the TTL doctor from validating expected values.

### Action Items (non-goals for this PR — tracked separately)

| Crate | Action | Priority |
|---|---|---|
| `vault/src/lib.rs` | Add `extend_ttl` calls on every mutating entrypoint (instance storage) | High |
| `revenue_pool/src/lib.rs` | Declare `LIFETIME_THRESHOLD` and `BUMP_AMOUNT` constants; fix compilation bug | Critical |
| `settlement/src/lib.rs` | Replace `50000` literals with `PERSISTENT_BUMP_THRESHOLD` / `PERSISTENT_BUMP_AMOUNT` constants | High |
| `settlement/src/limits.rs` | Same as above | High |
| `settlement/src/timelock.rs` | Replace `50_000` literals with `SHORT_BUMP_THRESHOLD` / `SHORT_BUMP_AMOUNT` | Medium |
| `settlement/src/lib.rs` | Add `extend_ttl` calls for instance storage | High |

---

## View Endpoints in Smart Contracts

### Revenue Pool — policy only

`get_ttl_policy() -> Vec<TtlPolicy>`

`TtlPolicy` carries `category`, `key_desc`, `storage_type`, `threshold`, and `bump_amount`. It has **no `ttl` field**: the revenue pool previously exposed `get_storage_ttl()`, whose `ttl` field was the live instance TTL under `cfg(test)` but the constant `BUMP_AMOUNT` in production builds — a fabricated measurement that made the tool useless (and misleading) in exactly the deployments it was meant to check. For the deployed revenue pool, the doctor reads `liveUntilLedgerSeq` for the contract instance entry over RPC.

### Vault and Settlement — entries with in-band TTL

* **Vault**: `get_storage_ttl(request_ids: Vec<Symbol>) -> Vec<StorageEntryTtl>`
* **Settlement**: `get_storage_ttl(developer_addresses: Vec<Address>) -> Vec<StorageEntryTtl>`

These views exist to *enumerate* which storage entries belong to a category (the doctor cannot discover persistent keys on its own). Their `ttl` fields carry the same caveat as the removed revenue-pool field and should be migrated to the same RPC-based approach; until then, treat the reported `ttl` as advisory and cross-check with `getLedgerEntries`.

---

## TTL Thresholds

The Revenue Pool instance TTL uses the following ledger constants (assuming a 5-second ledger close time):

| Constant | Ledgers | Approx duration |
|---------|---------|------------------|
| `LEDGERS_PER_DAY` | 17,280 | 1 day |
| `LIFETIME_THRESHOLD` | 17,280 (`LEDGERS_PER_DAY`) | ~1 day |
| `BUMP_AMOUNT` | 518,400 (`LEDGERS_PER_DAY * 30`) | 30 days |

The Revenue Pool bumps its instance on every entrypoint, including the read-only `get_pending_emergency_drain` view, so that a quiet period or an incident response never lets the instance archive.

---

## How to Run Locally

### 1. Install Node.js Dependencies

```bash
npm install
```

### 2. Run the Doctor Script

```bash
npx ts-node scripts/storage-ttl-doctor.ts \
  --vault-id "C..." \
  --settlement-id "C..." \
  --revenue-pool-id "C..." \
  --threshold 100000
```

### 3. Validate Against the Policy Table

Pass `--policy` to load expected threshold/bump values from the policy table embedded in this script and flag any contract whose live values diverge:

```bash
npx ts-node scripts/storage-ttl-doctor.ts \
  --vault-id "C..." \
  --settlement-id "C..." \
  --revenue-pool-id "C..." \
  --policy
```

---

## CLI Options

| Option | Description | Default |
|--------|-------------|---------|
| `--threshold <number>` | Min remaining TTL (in ledgers) below which the script exits with code 1 | Uses contract default thresholds |
| `--rpc-url <string>` | Soroban RPC server endpoint | `https://soroban-testnet.stellar.org` |
| `--vault-id <string>` | Contract ID of the deployed Callora Vault | `null` |
| `--settlement-id <string>` | Contract ID of the deployed Callora Settlement | `null` |
| `--revenue-pool-id <string>` | Contract ID of the deployed Callora Revenue Pool | `null` |
| `--request-ids <list>` | Comma-separated list of transaction request IDs to query processed status TTL | `[]` |
| `--developer-addresses <list>` | Comma-separated list of developer addresses to check persistent balance TTL | `[]` (falls back to index) |
| `--policy` | Validate live TTL values against the policy table; exits 1 if any entry deviates | `false` |

---

## JSON Schema

The tool outputs a machine-readable JSON report to stdout:

```json
{
  "timestamp": "2026-06-28T00:10:00.000Z",
  "threshold": 100000,
  "summary": {
    "total_categories": 5,
    "categories_below_threshold": 0,
    "status": "OK"
  },
  "categories": {
    "Instance": {
      "storage_type": "Instance",
      "remaining_ttl": 518400,
      "threshold": 518400,
      "bump_amount": 1036800,
      "status": "OK",
      "entries": [
        {
          "contract": "Vault",
          "contract_id": "CDVAULT...",
          "key_desc": "Instance",
          "ttl": 518400,
          "threshold": 518400,
          "bump_amount": 1036800
        }
      ]
    },
    "ProcessedRequest": {
      "storage_type": "Persistent",
      "remaining_ttl": null,
      "threshold": null,
      "bump_amount": null,
      "status": "EMPTY",
      "entries": []
    }
  },
  "policy_violations": [],
  "errors": []
}
```

When `--policy` is passed the report gains a `policy_violations` array:

```json
{
  "policy_violations": [
    {
      "contract": "Settlement",
      "category": "DeveloperBalance",
      "field": "threshold",
      "expected": 518400,
      "actual": 50000,
      "message": "DeveloperBalance threshold 50000 is below policy minimum 518400"
    }
  ]
}
```
`remaining_ttl` for a revenue-pool category is derived from `liveUntilLedgerSeq - latestLedger` as returned by `getLedgerEntries`.

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | All active categories are above threshold; no policy violations; no errors |
| `1` | One or more categories are below threshold, or a policy violation was found, or an RPC error occurred |

---

## Nightly Workflow

The Storage TTL Doctor is configured to run on a nightly cron schedule in `.github/workflows/ttl-doctor.yml`. It runs at 2:00 AM UTC every night, queries the deployed contract addresses configured in GitHub Secrets, and outputs the status report to the action logs.
