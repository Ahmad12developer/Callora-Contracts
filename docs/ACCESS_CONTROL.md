# Access Control

## 1. Vault Access Control

### Overview
The Callora Vault implements role-based access control for deposit operations to ensure only authorized parties can increase the vault balance.

### Roles
- **Owner**: Set during contract initialization. Exclusive authority to manage allowed depositors, withdraw funds, and propose revenue pool changes.
- **Allowed Depositor**: Addresses approved by the owner to handle automated deposits.
- **Authorized Caller**: Optional address permitted to trigger `deduct` operations.
- **Pending Owner**: Nominee awaiting acceptance of the owner role.
- **Pending Admin**: Nominee awaiting acceptance of the admin role.
- **Admin / Multisig Admin**: Current admin address. If this address is a Stellar multisig account, native thresholds and signer weights are enforced by `require_auth`.
- **Pending Revenue Pool**: Proposed revenue pool address awaiting acceptance.

### Authorization Matrix

| Function | Owner | Allowed Depositor | Authorized Caller | Pending Owner | Others |
|----------|-------|-------------------|-------------------|---------------|--------|
| `deposit` | ✅ | ✅ | ❌ | ❌ | ❌ |
| `withdraw` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `withdraw_to` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `deduct` | ❌ | ❌ | ✅ | ❌ | ❌ |
| `batch_deduct` | ❌ | ❌ | ✅ | ❌ | ❌ |
| `set_allowed_depositor` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `clear_allowed_depositors` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `set_authorized_caller` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `transfer_ownership` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `accept_ownership` | ❌ | ❌ | ❌ | ✅ | ❌ |
| `cancel_ownership_transfer` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `set_admin` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `accept_admin` | ❌ | ❌ | ❌ | ❌ | ✅ |
| `cancel_admin_transfer` | ❌ | ❌ | ❌ | ❌ | ✅ |
| `propose_revenue_pool` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `accept_revenue_pool` | ❌ | ❌ | ❌ | ❌ | ✅ |
| `cancel_revenue_pool` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `pause` | ✅ | ❌ | ❌ | ❌ | ❌ |
| `nuclear_pause` | ❌ | ❌ | ❌ | ❌ | Admin only |
| `unpause` | ✅ | ❌ | ❌ | ❌ | ❌ |

### Security Model
- **Two-Step Owner Rotation**: Prevents accidental loss of control by requiring the nominee to explicitly accept the role.
- **Two-Step Admin Rotation**: Prevents accidental loss of control by requiring the nominee to explicitly accept the role.
- **Cancellation Safety**: Provides `cancel_ownership_transfer` and `cancel_admin_transfer` functions to abort mistaken nominations before acceptance.
- **Restricted Depositors**: Only owner and explicitly allowed depositors can increase vault balance.
- **Multisig-Guarded Nuclear Pause**: `nuclear_pause(caller)` accepts only the current admin address and calls `caller.require_auth()`, so deployments that set admin to a Stellar multisig account get native threshold enforcement for the emergency pause.
- **Nonce-Bound Authorized-Caller Rotation**: `set_authorized_caller` requires the caller to supply the current monotonic nonce (see below), preventing a leaked owner signature from being replayed to reinstate a stale `authorized_caller`.

### Authorized-Caller Replay Protection

`set_authorized_caller` maintains a monotonic `u64` nonce stored under
`StorageKey::AuthorizedCallerNonce` in instance storage.

| Step | Who | Action |
|------|-----|--------|
| 1 | Integrator | Call `get_authorized_caller_nonce()` to read the current nonce (defaults to `0`). |
| 2 | Owner | Call `set_authorized_caller(new_caller, expected_nonce)` with the value from step 1. |
| 3 | Contract | Verifies `expected_nonce == stored_nonce`; rejects with `VaultError::StaleNonce` if not. |
| 4 | Contract | Increments the stored nonce (`wrapping_add(1)`) and emits it in the event payload. |

**Replay resistance**: a captured owner signature contains a fixed `expected_nonce`.
After one successful rotation the stored nonce advances, so the captured signature is
permanently invalid.

**Event payload**: the `set_authorized_caller` event now carries
`(old_caller, new_caller, consumed_nonce)` as data, allowing off-chain indexers to
detect nonce gaps.

**Nonce wrap**: the nonce wraps to `0` after `u64::MAX` rotations (2^64 calls) — a
practical impossibility, but handled safely by `wrapping_add`.

### Cancellation Functions

#### cancel_ownership_transfer
Allows the current owner to cancel a pending ownership transfer before the nominee accepts it. This provides a safety mechanism to abort mistaken nominations.

**Access Control**: Only the current owner can call this function.
**Behavior**: 
- Removes the `PendingOwner` from storage
- Emits `ownership_cancelled` event with current owner and cancelled nominee
- Panics with "no ownership transfer pending" if no transfer is pending

#### cancel_admin_transfer
Allows the current admin to cancel a pending admin transfer before the nominee accepts it. This provides a safety mechanism to abort mistaken nominations.

**Access Control**: Only the current admin can call this function.
**Behavior**: 
- Removes the `PendingAdmin` from storage
- Emits `admin_cancelled` event with current admin and cancelled nominee
- Panics with "no admin transfer pending" if no transfer is pending

---

## 2. Settlement Access Control

### Overview
The Callora Settlement contract tracks individual developer balances and global protocol revenue. It enforces strict access control for incoming payments and administrative updates.

Developer address compliance recoveries use `propose_balance_migration` and
`execute_balance_migration`. Both calls require authorization by the current
admin address, including its native Stellar multisig thresholds, and execution
is delayed by 24 hours. See [Admin developer balance migration](ADMIN_BALANCE_MIGRATION.md).

### Roles
- **Admin**: Primary authority over contract configuration and sensitive data.
- **Vault**: The registered vault contract authorized to send payments.
- **Pending Admin**: Nominee awaiting acceptance of the admin role.
- **Pending Vault**: Proposed vault awaiting acceptance.

### Caller Matrix

| Entrypoint | Allowed Callers | Rationale |
|------------|-----------------|-----------|
| `init` | Admin | Initial contract setup and role assignment |
| `record_deduction` | Vault | Accounting-only deduction logging from the vault |
| `receive_payment` | Vault + Admin | Forwarding settlement payment from vault, or manual reconciliation by admin |
| `batch_receive_payment` | Vault + Admin | Forwarding batch settlement payments from vault, or manual reconciliation by admin |
| `set_developer_min_balance` (and alias `set_minimum_balance`) | Admin | Setting minimum balance floor per developer |
| `propose_balance_migration` | Admin | Proposing developer balance migration |
| `execute_balance_migration` | Admin | Executing matured developer balance migration |
| `set_usdc_token` | Admin | Configuring settlement USDC token contract address |
| `withdraw_developer_balance` | Developer | Developer claiming/withdrawing earned credits as USDC |
| `set_developer_claim_window` | Admin | Restricting developer withdrawal time window |
| `clear_developer_claim_window` | Admin | Removing developer withdrawal claim window |
| `set_daily_withdraw_cap` | Admin | Configuring developer daily withdrawal limit |
| `force_credit_developer` | Admin | Direct admin developer credit without moving tokens (manual reconciliation / dispute resolution) |
| `get_all_developer_balances` | Admin | Restricted batch query of developer balances |
| `get_developer_balances_page` | Admin | Restricted paginated query of developer balances |
| `get_developer_balances_cursor` | Admin | Restricted cursor-paginated query of developer balances |
| `set_admin` | Admin | Nominating new admin address (step 1 of two-step transfer) |
| `accept_admin` | Pending Admin | Accepting admin nomination (step 2 of two-step transfer) |
| `cancel_admin_transfer` | Admin | Cancelling pending admin nomination |
| `propose_vault` (and alias `set_vault`) | Admin | Proposing new vault address (step 1 of two-step rotation) |
| `accept_vault` | Proposed Vault + Admin | Finalizing pending vault rotation (step 2 of two-step rotation) |
| `broadcast` | Admin | Broadcasting operator/emergency message |
| `upgrade` | Admin | Upgrading contract code WASM |
| `migrate_developer_balance` (and alias `migrate_single_dev_v2`) | Admin | Storage schema migration for developer balance |
| `migrate_v1_to_v2` / `migrate_v1_to_v2_page` | Admin | Storage schema migration |
| `batch_withdraw_balance_cursor` | Anyone | Compatibility placeholder for cursor batch withdrawals |
| `batch_settle` | Anyone | Delegates to settlement batch helper |
| `freeze_developer` | Admin | Freezing developer withdrawals |
| `unfreeze_developer` | Admin | Unfreezing developer withdrawals |
| `set_price` / `remove_price` | Admin | Price registry management |

> [!NOTE]
> Read-only view entrypoints (`get_admin`, `get_vault`, `get_global_pool`, `get_total_received`, `get_developer_balance`, `get_developer_min_balance`, `get_minimum_balance`, `get_developer_claim_window`, `get_daily_withdraw_cap`, `get_withdrawal_today`, `get_pending_admin`, `get_balance_migration`, `get_usdc_token`, `get_version`, `version`, `migration_storage_version`, `is_developer_frozen`, `get_price`, `simulate_claim`) do not require authentication and are accessible by any caller.

### Dual-Caller Rule & Admin Security Model

> [!IMPORTANT]
> **Dual-Caller Security Risk**:
> The dual-caller authorization model for `receive_payment` and `batch_receive_payment` allows both the registered `Vault` and the contract `Admin` to credit developer balances. Because the `Admin` can call these entrypoints directly (in addition to `force_credit_developer`), the admin key has the technical capability to mint developer credits directly.
>
> **Operator Guidance**:
> Operators sizing admin key protections must recognize that a compromised or misconfigured admin key can credit developer balances directly. Admin keys should be protected using strong security configurations (e.g., Stellar native multisig thresholds, multi-party signing policies, hardware security modules, and strict key management controls).

### Security Model
- **Two-Step Admin Rotation**: Prevents accidental loss of control by requiring the nominee to explicitly accept the role.
- **Two-Step Vault Rotation**: Prevents accidentally misrouting settlement credits by requiring the proposed vault to accept (or the admin to finalize).
- **Per-Developer Claim Windows**: Admins may configure inclusive ledger timestamp windows that restrict when each developer can claim accrued settlement balance. Developers without a configured window remain unrestricted.
- **Restricted Views**: Sensitive batch queries like `get_all_developer_balances` are restricted to the admin to prevent unnecessary exposure of the full ledger via the contract interface.
- **Cancellation Safety**: The admin can invoke `cancel_admin_transfer` to clear a mistaken nomination.

---

## 3. Revenue Pool Access Control

### Overview
The Callora Revenue Pool contract processes USDC distribution to developer wallets. Like Settlement and Vault, it implements standard administrative roles and rotation procedures.

### Roles
- **Admin**: Handles revenue distributions and nominates administrative successions.
- **Pending Admin**: A nominated account that has to explicitly accept the role to become the Admin.
- **Pause Guardian**: Optional emergency role that may pause the revenue pool without receiving any distribution, unpause, upgrade, or admin-management authority.

### Authorization Matrix

> **Issue #730 audit** (GrantFox FWC26 Stellar Wave, 2026-07-28): every
> state-changing entrypoint below was verified to call `require_auth` on its
> acting `Address` before touching storage or emitting events.  No gaps were
> found.  Focused per-entrypoint tests are maintained in
> `contracts/tests/src/grant_fox_fwc26_auth_matrix.rs` (`revenue_pool_audit`
> module) and `contracts/revenue_pool/tests/auth_snap.rs`.

| Function | `require_auth` on | Admin | Pending Admin | Pause Guardian | Others |
|----------|-------------------|-------|---------------|----------------|--------|
| `set_admin` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `accept_admin` | `caller` | ❌ | ✅ | ❌ | ❌ |
| `claim_admin` (alias of `accept_admin`) | `caller` | ❌ | ✅ | ❌ | ❌ |
| `cancel_admin_transfer` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `set_pause_guardian` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `clear_pause_guardian` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `pause` | `caller` | ✅ | ❌ | ✅ | ❌ |
| `unpause` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `receive_payment` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `deposit_yield` | `treasury` | ✅ (treasury == admin) | ❌ | ❌ | ❌ |
| `set_max_distribute` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `distribute` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `batch_distribute` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `upgrade` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `broadcast` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `propose_emergency_drain` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `execute_emergency_drain` | `caller` | ✅ | ❌ | ❌ | ❌ |
| `cancel_emergency_drain` | `caller` | ✅ | ❌ | ❌ | ❌ |

**Read-only entrypoints** (`get_admin`, `get_usdc_token`, `get_pending_admin`,
`get_pause_guardian`, `is_paused`, `get_cumulative_yield_deposited`,
`get_max_distribute`, `balance`, `get_version`, `version`, `get_ttl_policy`,
`get_pending_emergency_drain`, `chunk_iter`) do **not** require auth and are
callable by anyone.

### Cancellation Safety
The current admin can call `cancel_admin_transfer` to abort a pending admin nomination.

### Pause Guardian Safety
The current admin can call `set_pause_guardian` to delegate emergency pause authority to a narrow role, and `clear_pause_guardian` to remove that role. The pause guardian can only call `pause`; it cannot unpause, distribute funds, rotate admin, change caps, clear or replace itself, or upgrade the contract.

### Auth Audit History

| Date | Issue | Scope | Outcome |
|------|-------|-------|---------|
| 2026-07-28 | [#730](https://github.com/CalloraOrg/Callora-Contracts/issues/730) | All 18 state-changing entrypoints on `callora-revenue-pool` | ✅ All entrypoints call `require_auth`; no gaps found. Focused tests added to `revenue_pool_audit` module in `grant_fox_fwc26_auth_matrix.rs`. |

---

## Test Coverage
The implementation includes comprehensive tests covering:
- ✅ **Issue #730 — Revenue Pool auth audit (FWC26 Stellar Wave)**: every state-changing
  entrypoint audited; focused three-variant tests (`_no_auth`, `_wrong_role`, `_authorized`)
  added to `contracts/tests/src/grant_fox_fwc26_auth_matrix.rs` (`revenue_pool_audit` module).
  Existing per-entrypoint coverage in `contracts/revenue_pool/tests/auth_snap.rs` and
  `contracts/revenue_pool/tests/emergency_auth_snap.rs` confirmed complete.
- ✅ `set_authorized_caller` default nonce is `0` before first rotation
- ✅ First rotation with nonce `0` succeeds and advances stored nonce to `1`
- ✅ Replaying a consumed nonce is rejected with `VaultError::StaleNonce`
- ✅ Supplying a future nonce is rejected with `VaultError::StaleNonce`
- ✅ Three sequential rotations each advance the nonce correctly
- ✅ Nonce wraps at `u64::MAX` via `wrapping_add`
- ✅ Failed rotations do not advance the stored nonce
- ✅ Successful rotation emits `(old, new, consumed_nonce)` in the event payload
- ✅ Vault self-address is rejected as `new_caller`
- ✅ Admin and Vault can call `receive_payment`
- ✅ Unauthorized callers are rejected from `receive_payment`
- ✅ Only Admin can call `set_admin` and `propose_vault` (and the `set_vault` alias)
- ✅ Only Admin or Pending Vault can call `accept_vault`
- ✅ Only Pending Admin can call `accept_admin`
- ✅ Only Admin can call `get_all_developer_balances`
- ✅ All rotation and update logic preserves state integrity
- ✅ Only current owner can call `cancel_ownership_transfer`
- ✅ Only current admin can call `cancel_admin_transfer` in Vault, Settlement, and Revenue Pool
- ✅ Cancel functions clear pending state and emit events
- ✅ Cancel functions fail when no transfer is pending
- ✅ Cancel functions fail for unauthorized callers
- ✅ After cancellation, new nominations can be made

Run tests with:
```bash
cargo build --workspace --release --target=wasm32-unknown-unknown
```
-| `set_allowed_depositor` | ✅ | ❌ | ❌ | ❌ | ❌ |
-| `clear_allowed_depositors` | ✅ | ❌ | ❌ | ❌ | ❌ |
+| `add_address` | ✅ | ❌ | ❌ | ❌ | ❌ |
+| `clear_all` | ✅ | ❌ | ❌ | ❌ | ❌ |
+| `get_allowlist` (view) | ✅ | ✅ | ✅ | ✅ | ✅ |
+| `is_authorized_depositor` (view) | ✅ | ✅ | ✅ | ✅ | ✅ |