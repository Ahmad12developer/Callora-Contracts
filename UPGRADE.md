# Multi-Contract Upgrade and Migration Playbook

This document describes how the Callora smart contracts are deployed, how state is stored, and how to upgrade or migrate each contract. Soroban contract upgradeability is limited: you cannot replace the code of an existing contract instance in place. Migration is done by deploying a new contract and migrating state and traffic.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Contract Storage Layouts](#contract-storage-layouts)
3. [Upgrade Choreography](#upgrade-choreography)
4. [Backend Coordination](#backend-coordination)
5. [Admin Key Handling](#admin-key-handling)
6. [Rollback Stance](#rollback-stance)
7. [Stellar Network Procedures](#stellar-network-procedures)
8. [Verification Checklist](#verification-checklist)

---

## Architecture Overview

The Callora workspace consists of three independently deployed Soroban smart contracts:

| Contract | Crate | Purpose |
|----------|-------|---------|
| **callora-vault** | `contracts/vault` | USDC vault for prepaid API calls; tracks per-user balance, min deposits, and authorized callers |
| **callora-revenue-pool** | `contracts/revenue_pool` | Receives USDC from vault deducts and distributes to developers |
| **callora-settlement** | `contracts/settlement` | Developer balance tracking and global pool settlement |

### Deploy Model

- **One WASM per contract type**: Each contract is built as a separate Soroban WASM module.
- **One instance per logical entity**: Each vault/revenue-pool/settlement is a separate contract instance identified by its contract address.
- **No in-place upgrades**: Soroban does not support replacing contract code in place. To change behavior, you must deploy a new WASM and migrate state.

### Dependency Flow

```
User deposits USDC
        │
        ▼
  ┌─────────┐      deduct       ┌──────────────┐      distribute      ┌─────────────┐
  │  Vault  │ ───────────────► │ Revenue Pool │ ─────────────────► │ Developer   │
  │         │   (if revenue_    │              │                    │ Wallet      │
  └─────────┘    pool set)      └──────────────┘                    └─────────────┘

  ┌─────────┐      deduct       ┌────────────┐
  │  Vault  │ ───────────────► │ Settlement │ (alternative to revenue pool)
  └─────────┘                  └────────────┘
```

---

## Contract Storage Layouts

### Vault (`contracts/vault/src/lib.rs`)

The vault uses **instance storage** with the following keys:

| StorageKey | Type | Description |
|------------|------|-------------|
| `MetaKey` | `VaultMeta` | Owner address, tracked balance, authorized caller, min deposit |
| `AllowedDepositors` | `Vec<Address>` | List of addresses permitted to deposit |
| `Admin` | `Address` | Admin address (defaults to owner at init) |
| `UsdcToken` | `Address` | USDC token contract address |
| `Settlement` | `Option<Address>` | Optional settlement contract for deduct flow |
| `RevenuePool` | `Option<Address>` | Optional revenue pool address for deduct flow |
| `MaxDeduct` | `i128` | Maximum amount per single deduct (default `i128::MAX`) |
| `Metadata(String)` | `String` | Per-offering metadata (IPFS CID or URI) |
| `ContractVersion` | `BytesN<32>` | WASM hash set by `upgrade` function |

**VaultMeta structure** (defined in `lib.rs:46-51`):

```rust
pub struct VaultMeta {
    pub owner: Address,
    pub balance: i128,
    pub authorized_caller: Option<Address>,
    pub min_deposit: i128,
}
```

**Init signature** (`lib.rs:93-131`):

```rust
pub fn init(
    env: Env,
    owner: Address,
    usdc_token: Address,
    initial_balance: Option<i128>,
    authorized_caller: Option<Address>,
    min_deposit: Option<i128>,
    revenue_pool: Option<Address>,
    max_deduct: Option<i128>,
) -> VaultMeta
```

#### Upgradeability

The vault supports in-place upgrades via an admin-gated `upgrade` function. 
This method calls the host deployer to update the contract WASM code while 
preserving existing instance storage.

```bash
# Build new WASM
cargo build --target wasm32-unknown-unknown --release -p callora-vault

# Compute WASM hash and call upgrade via RPC or tooling
soroban contract invoke --contract-id <VAULT_ID> -- upgrade \
   --caller <ADMIN> --new_wasm_hash <32-byte-hex>
```

The `version()` view returns the stored WASM hash; the contract emits an `upgraded`
event with the admin as a topic and the new version as data.

**Upgrade Function Signature:**

```rust
pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>)
```

**Version Function Signature:**

```rust
pub fn get_version(env: Env) -> Option<BytesN<32>>
```

**Behavior note (pause semantics):**

- `pause()` is a circuit breaker for **deposit-like** flows.
- While paused, `deposit()` is rejected, but **`deduct()` and `batch_deduct()` still execute and still emit `deduct` events**.
- If you rely on “pause stops all balance movement”, you must update your operational assumptions and monitoring.

### Revenue Pool (`contracts/revenue_pool/src/lib.rs`)

| Key | Type | Description |
|-----|------|-------------|
| `admin` | `Address` | Admin address; may call `distribute` and `set_admin` |
| `usdc` | `Address` | USDC token contract address |
| `ContractVersion` | `BytesN<32>` | WASM hash set by `upgrade` function |

#### Upgradeability

The revenue pool now supports in-place upgrades via an admin-gated `upgrade` function. 
This method calls the host deployer to update the contract WASM code while 
preserving existing instance storage.

```bash
# Build new WASM
cargo build --target wasm32-unknown-unknown --release -p callora-revenue-pool

# Compute WASM hash (example helper) and call upgrade via RPC or tooling
soroban contract invoke --contract-id <REVENUE_POOL_ID> -- upgrade \
   --caller <ADMIN> --new_wasm_hash <32-byte-hex>
```

The `get_version()` view returns the stored WASM hash; the contract emits an `upgraded`
event with the admin as a topic and the new version as data.

**Init signature** (`lib.rs:28-39`):

```rust
pub fn init(env: Env, admin: Address, usdc_token: Address)
```

### Settlement (`contracts/settlement/src/lib.rs`)

| Key | Type | Description |
|-----|------|-------------|
| `admin` | `Address` | Admin address; may call `set_admin`, `set_vault` |
| `vault` | `Address` | Registered vault address |
| `developer_balances` | `Map<Address, i128>` | Per-developer balance tracking |
| `global_pool` | `GlobalPool` | Total balance and last updated timestamp |
| `ContractVersion` | `BytesN<32>` | WASM hash set by `upgrade` function |

#### Upgradeability

The settlement contract supports in-place upgrades via an admin-gated `upgrade` function. 
This method calls the host deployer to update the contract WASM code while 
preserving existing instance storage.

```bash
# Build new WASM
cargo build --target wasm32-unknown-unknown --release -p callora-settlement

# Compute WASM hash and call upgrade via RPC or tooling
soroban contract invoke --contract-id <SETTLEMENT_ID> -- upgrade \
   --caller <ADMIN> --new_wasm_hash <32-byte-hex>
```

The `get_version()` view returns the stored WASM hash (as an `Option<BytesN<32>>`); the contract emits an `upgraded`
event with the admin as a topic and the new version as data.

**GlobalPool structure** (`lib.rs:16-19`):

```rust
pub struct GlobalPool {
    pub total_balance: i128,
    pub last_updated: u64,
}
```

**Init signature** (`lib.rs:51-65`):

```rust
pub fn init(env: Env, admin: Address, vault_address: Address)
```

---

## Upgrade Choreography

Because contracts reference each other by address, upgrades must be sequenced carefully to maintain consistency.

### Recommended Upgrade Order

```
1. Settlement (if changing)
       │
       ▼
2. Revenue Pool (if changing)
       │
       ▼
3. Vault (always upgrade last if updating references)
```

**Rationale**: The vault can reference either a settlement contract or a revenue pool. Update the target first, then update the vault's reference.

### Per-Contract Upgrade Steps

#### A. Upgrading Vault

**Note:** As of version 1.1.0, the Vault contract supports in-place WASM upgrades via the `upgrade` function, eliminating the need for full redeployment and state migration in most cases.

##### Option 1: In-Place Upgrade (Recommended)

The vault now supports admin-gated in-place upgrades that preserve all existing state:

1. **Build new vault WASM**
   ```bash
   cargo build --target wasm32-unknown-unknown --release -p callora-vault
   ```

2. **Compute WASM hash**
   ```bash
   # Using soroban CLI or custom tooling
   soroban contract install --wasm target/wasm32-unknown-unknown/release/callora_vault.wasm
   # Returns: <NEW_WASM_HASH>
   ```

3. **Call upgrade function** (admin only)
   ```bash
   soroban contract invoke --contract-id <VAULT_ID> -- upgrade \
     --caller <ADMIN> \
     --new_wasm_hash <NEW_WASM_HASH>
   ```

4. **Verify upgrade**
   ```bash
   # Check version marker
   soroban contract invoke --contract-id <VAULT_ID> -- get_version
   # Should return <NEW_WASM_HASH>
   
   # Verify state preserved
   soroban contract invoke --contract-id <VAULT_ID> -- get_meta
   soroban contract invoke --contract-id <VAULT_ID> -- balance
   ```

5. **Run post-upgrade migration** (if needed)
   ```bash
   // If the new WASM includes a migrate function for schema changes
   soroban contract invoke --contract-id <VAULT_ID> -- migrate \
     --caller <ADMIN>
   ```

**Migration of `request_id` Deduplication (v1.2+):**
The vault now uses `persistent` storage for `ProcessedRequest` idempotency markers rather than `temporary` storage.
During the upgrade, no explicit state migration script is needed. The `require_not_duplicate` checks have been updated to query *both* temporary (legacy) and persistent (new) storage. Temporary markers will naturally expire over the next ~30 days. Going forward, the owner should periodically invoke `prune_processed_requests` to garbage-collect old persistent markers and recover storage deposits.

6. **Monitor and verify**
   - Test a small transaction
   - Verify all view functions return expected values
   - Check event emissions
   - Monitor error rates

**Upgrade Event:**
The `upgrade` function emits an `upgraded` event with:
- Topic 0: `Symbol("upgraded")`
- Topic 1: Admin address
- Data: New WASM hash (BytesN<32>)

**Version Tracking:**
- Call `get_version()` to retrieve the current WASM hash
- Returns `None` for contracts deployed before upgrade functionality
- Returns `Some(BytesN<32>)` after first upgrade

##### Option 2: Full Redeployment (Legacy/Fallback)

Use this approach only when in-place upgrade is not possible (e.g., breaking storage changes):

1. **Export state**
   ```bash
   # Read current state via RPC or CLI
   soroban contract invoke --contract-id <VAULT_ID> -- get_meta
   soroban contract invoke --contract-id <VAULT_ID> -- get_admin
   soroban contract invoke --contract-id <VAULT_ID> -- get_settlement  # if set
   ```

2. **Deploy new vault WASM**
   ```bash
   cargo build --target wasm32-unknown-unknown --release -p callora-vault
   soroban contract deploy --wasm target/wasm32-unknown-unknown/release/callora_vault.wasm --source <OWNER_ACCOUNT>
   ```

3. **Initialize new vault** (same owner, same USDC token, migrate balance)
   ```bash
   soroban contract invoke --contract-id <NEW_VAULT_ID> -- init \
     --owner <OWNER> \
     --usdc_token <USDC_TOKEN> \
     --initial_balance <CURRENT_BALANCE> \
     --authorized_caller <AUTH_CALLER> \
     --min_deposit <MIN_DEPOSIT> \
     --revenue_pool <REVENUE_POOL_OR_NONE> \
     --max_deduct <MAX_DEDUCT>
   ```

4. **Transfer actual USDC** (if balance was real USDC)
   ```bash
   # From old vault owner, withdraw to self, then deposit to new vault
   soroban contract invoke --contract-id <OLD_VAULT_ID> -- withdraw --amount <BALANCE>
   # Then deposit from owner to new vault
   soroban contract invoke --contract-id <NEW_VAULT_ID> -- deposit --caller <OWNER> --amount <BALANCE>
   ```

5. **Update backend config** (see Backend Coordination below)

6. **Decommission old vault** (stop using; do not delete)

#### B. Upgrading Revenue Pool

1. **Export state**
   ```bash
   soroban contract invoke --contract-id <RP_ID> -- get_admin
   soroban contract invoke --contract-id <RP_ID> -- balance
   ```

2. **Deploy new revenue pool WASM**
   ```bash
   cargo build --target wasm32-unknown-unknown --release -p callora-revenue-pool
   soroban contract deploy --wasm target/wasm32-unknown-unknown/release/callora_revenue_pool.wasm --source <ADMIN_ACCOUNT>
   ```

3. **Initialize new revenue pool**
   ```bash
   soroban contract invoke --contract-id <NEW_RP_ID> -- init \
     --admin <ADMIN> \
     --usdc_token <USDC_TOKEN>
   ```

4. **Transfer USDC balance** (if applicable)
   - Revenue pool holds actual USDC tokens
   - Transfer from old contract to new via token `transfer`

5. **Update vault references** (if vault points to this revenue pool)

6. **Decommission old revenue pool**

#### C. Upgrading Settlement

> **Note:** As of the timelocked-upgrade release, the settlement contract enforces a mandatory
> **48-hour delay** between proposing and executing an upgrade. The instant `upgrade` function
> is retained as a deprecated alias for `propose_upgrade` — it no longer applies the WASM
> immediately. Call `execute_upgrade` after the delay to finalize.

##### In-Place Upgrade (Recommended)

The settlement contract supports admin-gated, timelocked in-place WASM upgrades that preserve
all developer balances and contract state.

**Step 1 — Build new WASM**

```bash
cargo build --target wasm32-unknown-unknown --release -p callora-settlement
```

**Step 2 — Upload WASM and obtain hash**

```bash
soroban contract install \
  --wasm target/wasm32-unknown-unknown/release/callora_settlement.wasm
# Returns: <NEW_WASM_HASH>
```

**Step 3 — Propose the upgrade (admin only)**

```bash
soroban contract invoke --contract-id <SETTLEMENT_ID> -- propose_upgrade \
  --caller <ADMIN> \
  --new_wasm_hash <NEW_WASM_HASH>
```

This records a [`PendingUpgrade`] snapshot and emits an `upgrade_proposed` event containing:

| Field | Value |
|---|---|
| `wasm_hash` | `<NEW_WASM_HASH>` |
| `proposed_at` | ledger timestamp at proposal |
| `execute_after` | `proposed_at + 172800` (48 h) |

**Step 4 — Verify the proposal is visible**

```bash
soroban contract invoke --contract-id <SETTLEMENT_ID> -- get_pending_upgrade
# Returns the PendingUpgrade struct, or null if no proposal exists.
```

**Step 5 — Wait for the 48-hour window to elapse**

Off-chain watchers and security teams should monitor `upgrade_proposed` events.
If anything looks wrong, the admin can abort immediately — see Step 6.

**Step 6 (optional) — Cancel**

```bash
soroban contract invoke --contract-id <SETTLEMENT_ID> -- cancel_upgrade \
  --caller <ADMIN>
# Emits upgrade_cancelled and clears the proposal.
```

Cancellation may be called at any time before execution, including before the delay has
expired. After cancellation, `get_pending_upgrade` returns `null`.

**Step 7 — Execute the upgrade (after delay)**

```bash
soroban contract invoke --contract-id <SETTLEMENT_ID> -- execute_upgrade \
  --caller <ADMIN>
# Applies the WASM, clears the proposal, emits `upgraded`.
```

Calling before `execute_after` will return `UpgradeTimelockNotExpired (45)`.

**Step 8 — Verify**

```bash
soroban contract invoke --contract-id <SETTLEMENT_ID> -- get_version
# Should return <NEW_WASM_HASH>

soroban contract invoke --contract-id <SETTLEMENT_ID> -- get_pending_upgrade
# Should return null (proposal cleared)

soroban contract invoke --contract-id <SETTLEMENT_ID> -- get_admin
soroban contract invoke --contract-id <SETTLEMENT_ID> -- get_global_pool
```

##### Upgrade Events

| Event topic | Emitted by | Key data |
|---|---|---|
| `upgrade_proposed` | `propose_upgrade` | `wasm_hash`, `proposed_at`, `execute_after` |
| `upgrade_cancelled` | `cancel_upgrade` | `wasm_hash`, `cancelled_at` |
| `upgraded` | `execute_upgrade` | `new_wasm_hash` |

##### Error Codes

| Code | Variant | Meaning |
|---|---|---|
| 43 | `NoUpgradePending` | `execute_upgrade` / `cancel_upgrade` called with no active proposal |
| 44 | `ZeroWasmHash` | All-zero `new_wasm_hash` rejected by `propose_upgrade` |
| 45 | `UpgradeTimelockNotExpired` | `execute_upgrade` called before `execute_after` |

##### Full Redeployment (Legacy / Fallback)

Use only when an in-place upgrade is not feasible (e.g., breaking storage schema changes).

---

## Backend Coordination

When any contract is upgraded, the backend must update its configuration:

### Configuration Changes Required

| Upgrade Type | Backend Config Update |
|-------------|----------------------|
| New vault instance | Update `vault_contract_id` per user/API |
| New revenue pool | Update `revenue_pool_contract_id` in vault (via `set_settlement`) |
| New settlement | Update `settlement_contract_id` in vault (via `set_settlement`) |
| Vault points to new revenue pool | Call `vault.set_revenue_pool(new_address)` |
| Vault points to new settlement | Call `vault.set_settlement(new_address)` |

### Backend Update Sequence

```
1. Deploy new contract(s)
2. Initialize with migrated state
3. Update backend configuration (new contract addresses)
4. Verify backend can reach new contracts
5. Point traffic to new contract (gradual or atomic switchover)
6. Monitor for 24-48 hours
7. Decommission old contract (stop calls, archive address)
```

### Health Checks After Upgrade

- Verify `get_meta()` / `get_admin()` / `balance()` return expected values
- Run a small test transaction before full traffic switchover
- Monitor error rates and revert if anomalies detected

---

## Admin Key Handling

### Key Management Expectations

Admin keys for all three contracts should be managed with care:

| Contract | Admin Role | Key Type Recommendation |
|----------|-----------|------------------------|
| Vault | Sets distribution recipients, authorized callers, min deposits | Hardware wallet or multisig |
| Revenue Pool | Calls `distribute`, `batch_distribute`, `set_admin` | Hardware wallet or multisig |
| Settlement | Calls `set_admin`, `set_vault`, receives payments | Hardware wallet or multisig |

### Key Rotation Procedure

To rotate an admin key:

1. **Ensure new admin key is accessible** (test in non-production first)
2. **Call `set_admin`** on each affected contract:
   ```bash
   soroban contract invoke --contract-id <CONTRACT_ID> -- set_admin \
     --caller <OLD_ADMIN> \
     --new_admin <NEW_ADMIN>
   ```
3. **Verify** by calling `get_admin()` and confirming new address
4. **Update backend** to use new admin key for signing transactions
5. **Archive old admin key** (do not delete; retain for audit purposes)

### Multisig Considerations

- If using a Stellar multisig account (e.g., 2-of-3), all admin operations require sufficient signers
- Coordinate multisig transactions carefully to avoid being locked out
- Test multisig threshold changes on testnet before mainnet

---

## Rollback Stance

**Rollback is not supported as a first-class operation.** Due to Soroban's immutability design, there is no mechanism to revert a contract instance to previous code. Instead, rollback is achieved through redeployment.

### Rollback Procedure

If an upgrade causes issues:

1. **Do not attempt to modify the upgraded contract** — it cannot be changed
2. **Deploy the previous WASM as a new instance**:
   ```bash
   # Get previous WASM (from git history or artifact store)
   git checkout <PREVIOUS_COMMIT>
   cargo build --target wasm32-unknown-unknown --release -p callora-vault
   soroban contract deploy --wasm target/wasm32-unknown-unknown/release/callora_vault.wasm
   ```
3. **Migrate state back** (export from current, import to previous)
4. **Update backend** to point to the previous contract instance
5. **Investigate** the issue in the new contract separately (do not delete the new contract yet)

### Rollback Decision Matrix

| Scenario | Rollback Recommended? | Alternative |
|----------|----------------------|-------------|
| Critical bug affecting funds | Yes | Deploy hotfix and migrate |
| Non-critical bug | No | Deploy fix in next release cycle |
| Performance regression | No | Optimize and redeploy |
| Feature removal | No | Communicate to users; deprecate |

### Prevention

- Always test upgrades on testnet first
- Run full test suite (`cargo test`) and coverage (`./scripts/coverage.sh`) before any upgrade
- Use gradual traffic switchover (e.g., 5% → 25% → 100%) to catch issues early

---

## Stellar Network Procedures

### Soroban Upgrade Constraints

Soroban smart contracts on Stellar have the following upgrade characteristics:

1. **No in-place code replacement**: Once a contract is deployed, its WASM code cannot be changed.
2. **Contract addresses are deterministic**: The address is derived from the deployer's public key and sequence number, not from the WASM code hash.
3. **Storage persists independently**: Contract storage exists separately from the WASM code and travels with the contract address.

### WASM Size Limits

Soroban enforces a **64 KB WASM size limit**. The Callora contracts are optimized to stay under this limit:

```bash
# Check WASM size
./scripts/check-wasm-size.sh

# Build optimized WASM
cargo build --target wasm32-unknown-unknown --release -p callora-vault
```

Current optimized sizes should be approximately:
- `callora-vault`: ~17-18 KB
- `callora-revenue-pool`: ~15-16 KB
- `callora-settlement`: ~16-17 KB

### Deployment on Stellar

1. **Build the WASM**
   ```bash
   cargo build --target wasm32-unknown-unknown --release -p <crate-name>
   ```

2. **Deploy using Soroban CLI or Stellar Laboratory**
   ```bash
   soroban contract deploy \
     --wasm target/wasm32-unknown-unknown/release/<contract>.wasm \
     --source <DEPLOYER_ACCOUNT>
   ```

3. **Initialize the contract**
   ```bash
   soroban contract invoke --contract-id <NEW_ID> -- init <args>
   ```

4. **Verify on-chain**
   ```bash
   soroban contract invoke --contract-id <NEW_ID> -- get_meta  # or other view function
   ```

### Network Selection

| Network | Use For |
|---------|---------|
| **Testnet** | Development, testing upgrades, integration testing |
| **Mainnet** | Production deployment |

**Never deploy experimental code directly to mainnet.**

---

## Verification Checklist

Before and after any upgrade, verify the following:

### Pre-Upgrade Verification

- [ ] All tests pass: `cargo test`
- [ ] Coverage above 95%: `./scripts/coverage.sh`
- [ ] Clippy clean: `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] Format clean: `cargo fmt -- --check`
- [ ] WASM size under limit: `./scripts/check-wasm-size.sh`
- [ ] State export from old contracts completed
- [ ] Backend configuration backup taken
- [ ] Rollback plan documented and tested (if critical)

### Post-Upgrade Verification

- [ ] `get_meta()` / `get_admin()` / `balance()` return expected values
- [ ] Test transaction executed successfully
- [ ] Backend can communicate with new contracts
- [ ] Error rates nominal (compare to pre-upgrade baseline)
- [ ] Event emissions correct (verify emitted events match expected)
- [ ] Monitoring dashboards updated (if applicable)
- [ ] Old contract marked as decommissioned (no new traffic)

### Test and Commit Requirements

Per the contribution guidelines:

1. Run `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` from workspace root
2. For WASM builds: `cargo build --target wasm32-unknown-unknown --release -p callora-vault` (adjust `-p` as needed)
3. Run `./scripts/coverage.sh` (or `cargo tarpaulin` per `tarpaulin.toml`)
4. Include summarized test output in PR description

---

## Summary

| Aspect | Recommendation |
|--------|----------------|
| **Upgrade approach** | In-place timelocked upgrade (settlement); deploy new contract for full rewrites |
| **Settlement upgrade flow** | `propose_upgrade` → wait 48 h → `execute_upgrade` (cancel with `cancel_upgrade`) |
| **Upgrade order** | Settlement → Revenue Pool → Vault |
| **Rollback** | Not supported; deploy previous WASM as new instance |
| **Admin keys** | Hardware wallet or multisig; rotate via `set_admin` |
| **Testing** | Testnet first, then gradual mainnet rollout |
| **Verification** | Run full test suite and coverage before any upgrade |

For detailed storage layouts, see:
- [Vault Storage Layout](contracts/vault/STORAGE.md)
- [Vault Access Control](contracts/vault/ACCESS_CONTROL.md)
- [Core Contracts README](contracts/README.md)

### Upgrade Cooldown
As of the GrantFox FWC26 campaign, a cool-off window is enforced between admin actions on upgrade to prevent rapid abuse.
This is implemented in the `contracts/upgrade/src/admin.rs` module.
Admins must wait at least 24 hours (configurable) between consecutive upgrades.
