# Contract Address Configuration Guide for Backend Operators

This guide explains how to deploy and wire the three Callora contracts so that
USDC deducted from a vault is correctly forwarded to a settlement contract and
developers can withdraw their earnings.

The contracts have a **circular dependency at init time**: the vault needs the
settlement address and the settlement needs the vault address. The ordering
below breaks that cycle safely.

---

## Background

When a backend operator calls `deduct` or `batch_deduct`, the vault reduces the
caller's on-chain balance **and** transfers the corresponding USDC to the
configured settlement contract. The destination is stored in the vault at init
time:

| Address slot   | Storage key   | Purpose                                                              |
|----------------|---------------|----------------------------------------------------------------------|
| `settlement`   | `Settlement`  | `callora-settlement` contract; tracks per-developer balances         |
| `revenue_pool` | `RevenuePool` | `callora-revenue-pool` contract; simple admin-controlled distribution|
| `usdc_token`   | `UsdcToken`   | USDC token contract; set at init and never changed                   |

**Priority rule**: when `settlement` is configured it takes exclusive priority;
`revenue_pool` is not used in the same deduct call.

---

## Addresses at a glance

```
callora-vault
├── usdc_token    ← set at vault init; never changes
├── settlement    ← set at vault init (or later via set_settlement)
└── revenue_pool  ← set at vault init (or later via set_revenue_pool)

callora-settlement
├── vault         ← set at settlement init (or later via propose_vault/accept_vault)
└── usdc_token    ← set after settlement init via set_usdc_token
```

---

## Circular-dependency init order

The vault and settlement each store the other's address. The cycle is broken by
**deploying all three contracts first, then initializing them in sequence**:

```
1.  Deploy USDC token contract (or locate testnet/mainnet address)
2.  Deploy vault contract         → VAULT_CONTRACT_ID
3.  Deploy settlement contract    → SETTLEMENT_CONTRACT_ID
4.  Deploy revenue pool contract  → REVENUE_POOL_CONTRACT_ID
5.  vault.init(...)               ← pass SETTLEMENT_CONTRACT_ID here
6.  settlement.init(admin, VAULT_CONTRACT_ID)
7.  revenue_pool.init(admin, USDC_TOKEN_ID)
8.  settlement.set_usdc_token(admin, USDC_TOKEN_ID)   ← required for withdrawals
```

This matches the wiring in `scripts/e2e_setup.rs`.

---

## Step-by-step deployment runbook

### Step 1 — Locate the USDC token contract

On **Stellar testnet**, USDC is available at:

```bash
stellar contract id asset \
    --asset USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5 \
    --network testnet
# → USDC_TOKEN_ID
```

On **mainnet**, use the canonical Circle USDC issuer:

```
Issuer:  GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN
Asset:   USDC
```

Record the resulting contract ID as `USDC_TOKEN_ID`.

---

### Step 2 — Deploy the vault contract

```bash
stellar contract deploy \
    --wasm target/wasm32-unknown-unknown/release/callora_vault.wasm \
    --source <OPERATOR_KEY> \
    --network testnet
# → VAULT_CONTRACT_ID
```

Do **not** call `init` yet — you need the settlement address first.

---

### Step 3 — Deploy the settlement contract

```bash
stellar contract deploy \
    --wasm target/wasm32-unknown-unknown/release/callora_settlement.wasm \
    --source <OPERATOR_KEY> \
    --network testnet
# → SETTLEMENT_CONTRACT_ID
```

---

### Step 4 — Deploy the revenue pool (optional)

```bash
stellar contract deploy \
    --wasm target/wasm32-unknown-unknown/release/callora_revenue_pool.wasm \
    --source <OPERATOR_KEY> \
    --network testnet
# → REVENUE_POOL_CONTRACT_ID
```

Skip this step if you are routing all deductions to settlement only. You can
pass `None` for `revenue_pool` in vault `init`.

---

### Step 5 — Initialize the vault

All addresses are wired in a single `init` call. `settlement` is passed here
directly — there is no need for a separate `set_settlement` call.

```bash
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <OPERATOR_KEY> \
    --network testnet \
    -- init \
    --owner <OWNER_ADDRESS> \
    --usdc_token <USDC_TOKEN_ID> \
    --initial_balance 0 \
    --authorized_caller <BACKEND_ADDRESS> \
    --min_deposit 1000000 \
    --revenue_pool <REVENUE_POOL_CONTRACT_ID> \
    --max_deduct 9223372036854775807 \
    --settlement <SETTLEMENT_CONTRACT_ID>
```

> **`min_deposit` must be > 0** — passing `0` or omitting the flag panics
> with `MinDepositNotPositive`. The example above uses `1000000` stroops
> (0.1 USDC). Adjust to your operational minimum.

> **`settlement` is required for `deduct`** — if you omit this flag, any
> subsequent `deduct` call will panic with `"Settlement not set"`.

---

### Step 6 — Initialize the settlement contract

```bash
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <OPERATOR_KEY> \
    --network testnet \
    -- init \
    --admin <ADMIN_ADDRESS> \
    --vault_address <VAULT_CONTRACT_ID>
```

This registers the vault as the only non-admin address permitted to call
`receive_payment` and `record_deduction`.

---

### Step 7 — Initialize the revenue pool (if deployed)

```bash
stellar contract invoke \
    --id <REVENUE_POOL_CONTRACT_ID> \
    --source <OPERATOR_KEY> \
    --network testnet \
    -- init \
    --admin <ADMIN_ADDRESS> \
    --usdc_token <USDC_TOKEN_ID>
```

---

### Step 8 — Configure USDC on the settlement contract

`settlement.init` does **not** set the USDC token address. You must call
`set_usdc_token` before developers can withdraw their balances:

```bash
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <ADMIN_KEY> \
    --network testnet \
    -- set_usdc_token \
    --caller <ADMIN_ADDRESS> \
    --usdc_address <USDC_TOKEN_ID>
```

> **This step is required for `withdraw_developer_balance` to succeed.** If
> omitted, any withdrawal attempt will fail with `UsdcTokenNotConfigured`.

---

## Post-deploy verification checklist

Run these view functions after completing all eight steps to confirm the
deployment is correctly wired. None require authentication.

### Vault

```bash
# 1. Confirm settlement address
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_settlement
# Expected: "C<SETTLEMENT_CONTRACT_ID>"

# 2. Confirm revenue pool address (if configured)
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_revenue_pool
# Expected: "C<REVENUE_POOL_CONTRACT_ID>" or null

# 3. Confirm USDC token
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_usdc_token
# Expected: "C<USDC_TOKEN_ID>"

# 4. Confirm admin
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_admin
# Expected: owner address (admin defaults to owner at init)

# 5. Confirm tracked balance
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- balance
# Expected: 0 (no deposits yet)

# 6. Confirm pause state
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- is_paused
# Expected: false
```

### Settlement

```bash
# 7. Confirm vault address registered in settlement
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_vault
# Expected: "C<VAULT_CONTRACT_ID>"

# 8. Confirm admin
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_admin
# Expected: admin address passed to settlement init

# 9. Confirm global pool is zero
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <ANY_KEY> \
    --network testnet \
    -- get_global_pool
# Expected: { "total_balance": 0, "last_updated": <timestamp> }
```

**Checklist summary — deployment is ready when all nine checks pass.**

---

## Updating addresses after deployment

### Changing the settlement address (vault, owner-only)

`set_settlement` is restricted to the **vault owner** (not just any admin).

```bash
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <OWNER_KEY> \
    --network testnet \
    -- set_settlement \
    --caller <OWNER_ADDRESS> \
    --settlement <NEW_SETTLEMENT_CONTRACT_ID>
```

> ⚠️ Address changes take effect immediately on the next `deduct` call.
> Coordinate with your monitoring stack before switching in production.

### Rotating the vault address in settlement (two-step, admin-only)

The settlement contract uses a propose/accept pattern to rotate the vault
address. This prevents a typo from locking out the settlement contract.

**Step 1 — propose (current admin)**:

```bash
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <ADMIN_KEY> \
    --network testnet \
    -- propose_vault \
    --caller <ADMIN_ADDRESS> \
    --new_vault <NEW_VAULT_CONTRACT_ID>
```

**Step 2 — accept (proposed vault or admin)**:

```bash
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    --source <NEW_VAULT_KEY_OR_ADMIN_KEY> \
    --network testnet \
    -- accept_vault \
    --caller <NEW_VAULT_ADDRESS_OR_ADMIN_ADDRESS>
```

Verify the rotation completed:

```bash
stellar contract invoke \
    --id <SETTLEMENT_CONTRACT_ID> \
    -- get_vault
# Expected: "C<NEW_VAULT_CONTRACT_ID>"
```

### Changing the revenue pool address (vault, owner-only)

```bash
# Set a new revenue pool
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <OWNER_KEY> \
    --network testnet \
    -- set_revenue_pool \
    --caller <OWNER_ADDRESS> \
    --revenue_pool <NEW_REVENUE_POOL_CONTRACT_ID>

# Remove revenue pool routing entirely (pass None)
stellar contract invoke \
    --id <VAULT_CONTRACT_ID> \
    --source <OWNER_KEY> \
    --network testnet \
    -- set_revenue_pool \
    --caller <OWNER_ADDRESS> \
    --revenue_pool null
```

---

## Rollback steps

If any init call succeeds but a subsequent call fails, the contracts are in a
partially-wired state. The safest recovery is to redeploy and reinitialize from
scratch using the correct order above — Soroban contracts cannot be
re-initialized once `init` succeeds.

**Partial wiring scenarios and remediation:**

| Scenario | Problem | Fix |
|----------|---------|-----|
| Vault init succeeded, settlement init failed | Vault points to uninitialized settlement; deduct will route funds to an uninitialized contract | Redeploy settlement, re-init, then call `vault.set_settlement(owner, new_settlement_id)` |
| Settlement init succeeded, `set_usdc_token` not called | Developers cannot withdraw | Call `settlement.set_usdc_token(admin, usdc_id)` — no redeployment needed |
| Vault init succeeded without `settlement` arg | Deduct will panic | Redeploy vault or call `vault.set_settlement(owner, settlement_id)` after the fact |
| Wrong USDC token address in vault | Deposits and deductions use wrong token | Cannot be corrected; redeploy vault with correct `usdc_token` |

---

## TypeScript / stellar-sdk example

Use individual view functions instead of the removed `get_contract_addresses()`:

```ts
import { Contract, SorobanRpc, scValToNative } from "@stellar/stellar-sdk";

const server = new SorobanRpc.Server("https://soroban-testnet.stellar.org");
const vault  = new Contract(VAULT_CONTRACT_ID);

async function verifyWiring() {
    const settlement = scValToNative(
        (await server.simulateTransaction(
            buildTransaction(vault.call("get_settlement"))
        )).result.retval
    );

    const revenuePool = scValToNative(
        (await server.simulateTransaction(
            buildTransaction(vault.call("get_revenue_pool"))
        )).result.retval
    );

    const usdcToken = scValToNative(
        (await server.simulateTransaction(
            buildTransaction(vault.call("get_usdc_token"))
        )).result.retval
    );

    console.log({ usdcToken, settlement, revenuePool });

    if (!settlement) {
        console.warn("settlement address not configured — deduct will panic");
    }
}
```

---

## Security considerations

- **Owner key**: `set_settlement` and `set_revenue_pool` check the vault
  **owner** (not just any admin). Use a hardware wallet or multisig for the
  owner key.
- **Settlement admin key**: `set_usdc_token`, `propose_vault`, and other
  settlement admin functions check the settlement **admin**. Keep this key
  separate from the vault owner if you need independent access control.
- **Address validation**: The vault does not verify that configured addresses
  are valid, initialized contracts. Always confirm the settlement contract is
  deployed and initialized before passing its address to vault `init`.
- **Atomicity**: Each address change is a single storage write; no partial
  update is observable by concurrent callers.
- **Testnet vs mainnet**: USDC token IDs differ across networks. Verify with
  `get_usdc_token` after deployment.
- **`set_usdc_token` is irreversible** in the sense that it overwrites the
  prior value silently — double-check the address before calling it.

---

## See also

- [`docs/ACCESS_CONTROL.md`](ACCESS_CONTROL.md) — role matrix for all privileged functions
- [`SECURITY.md`](../SECURITY.md) — security checklist and threat model
- [`EVENT_SCHEMA.md`](../EVENT_SCHEMA.md) — events emitted by `set_settlement` / `set_revenue_pool`
- [`UPGRADE.md`](../UPGRADE.md) — upgrade and migration playbook
- [`scripts/e2e_setup.rs`](../scripts/e2e_setup.rs) — authoritative wiring reference used by the E2E test suite
