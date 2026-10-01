# Security

This document outlines security best practices and checklist items for Callora vault contracts to improve audit readiness and reviewer confidence.

## 🔐 Vault Security Checklist

### Access Control

- [ ] All privileged functions protected by `require_auth()` or `require_auth_for_args()` via `Address`
- [ ] Admin state stored securely (e.g., using `env.storage().instance()`)
- [ ] Admin rotation/transfer tested and documented

### Arithmetic Safety

- [x] No integer overflow/underflow possible
- [x] For Soroban/Rust: `checked_add` / `checked_sub` used for all balance mutations
- [x] `overflow-checks` enabled in both dev and release profiles

> All balance mutations in `callora-vault` (`deposit`, `deduct`, `batch_deduct`, `withdraw`, `withdraw_to`) and `callora-revenue-pool` (`batch_distribute`) use `checked_add` / `checked_sub` and panic with a descriptive message on overflow. `callora-settlement` (`receive_payment`) does the same. The workspace `Cargo.toml` sets `overflow-checks = true` for both `dev` and `release` profiles, so even plain arithmetic would trap in debug builds â€” the explicit checked calls make the intent clear and guarantee the same behaviour in all build configurations.

Additional hardening note:
- Removed a duplicated `get_max_deduct` entrypoint declaration in `callora-vault` to avoid ambiguous review surfaces and keep ABI-facing code paths singular. The function is retained as a private internal helper called by `deduct` and `batch_deduct`.

### Initialization / Re-initialization

- [x] `initialize` function protected against multiple calls (e.g., checking if admin key exists in `instance()` storage)
- [ ] Contract upgrades (`env.deployer().update_current_contract_wasm()`) protected by `require_auth()` — tracked in [#264](https://github.com/CalloraOrg/Callora-Contracts/issues/264)
- [ ] No unprotected re-init functions — `distribute::init` in `callora-revenue-pool` is currently callable without authentication; tracked in [#265](https://github.com/CalloraOrg/Callora-Contracts/issues/265)
- [ ] Unrestricted upgrade cooldown setter reviewed — `set_upgrade_cooldown` currently has no auth gate; tracked in [#266](https://github.com/CalloraOrg/Callora-Contracts/issues/266)
- [x] `initialize` validates all input parameters

### Pause / Circuit Breaker

- [x] Emergency pause mechanism implemented via state flag in `instance()` storage
- [x] Paused state blocks fund movement (e.g., reverting via `panic_with_error!`)
- [x] Pause/unpause flows tested
- [x] `is_paused()` view function exposed for off-chain monitoring
- [x] View function is read-only, deterministic, and non-panicking
- [x] Safe default state (returns `false` when unset)

### Admin Transfer

- [x] Ownership transfer is two-step (optional but recommended)
- [ ] Ownership transfer emits events — tracked in [#267](https://github.com/CalloraOrg/Callora-Contracts/issues/267)
- [ ] Renounce ownership reviewed and justified — tracked in [#268](https://github.com/CalloraOrg/Callora-Contracts/issues/268)

### Authorized Caller Role Management

The vault exposes a dedicated `authorized_caller` role (stored in `VaultMeta`
and settable via `set_authorized_caller`) that is permitted to invoke
balance-mutating operations such as `deduct` and `batch_deduct`. This role is
distinct from `owner` and `admin`, and reviewers should confirm the following
controls are in place:

- [x] `authorized_caller` is stored in `VaultMeta` under the `Meta` instance
  storage key and is not duplicated in any other location
- [x] Only the current `owner` can set or rotate `authorized_caller` via
  `set_authorized_caller` (enforced by `meta.owner.require_auth()`)
- [x] `set_authorized_caller` emits a `set_authorized_caller` event with the
  owner as topic and `(old_authorized_caller, new_authorized_caller)` as data,
  enabling off-chain monitoring of role changes and clear audit diffs during
  rotation
- [x] `deduct` and `batch_deduct` reject callers that are not the currently
  configured `authorized_caller` (panic: `unauthorized: caller is not the authorized caller`)
- [x] When `authorized_caller` is `None`, deduct-class operations fall back to
  owner-only execution; non-owner callers remain rejected
- [ ] Rotation flow (set → use → rotate → old caller rejected) covered by
  unit tests in `contracts/vault/src/test.rs` — tracked in [#269](https://github.com/CalloraOrg/Callora-Contracts/issues/269)
- [ ] Role changes are reviewed as part of the operational runbook; the new
  caller address is verified off-chain (e.g. multisig or governance) before
  the owner signs `set_authorized_caller` — tracked in [#270](https://github.com/CalloraOrg/Callora-Contracts/issues/270)
- [ ] `authorized_caller` is scoped strictly to deduct-class operations and
  does **not** grant the ability to withdraw, distribute, pause, or upgrade
  the contract — tracked in [#271](https://github.com/CalloraOrg/Callora-Contracts/issues/271)

> **Security note:** `authorized_caller` is intentionally a narrow-privilege
> role meant for the off-chain billing/settlement driver. It can spend vault
> balance via `deduct` / `batch_deduct` within the configured `max_deduct`
> limit, so the owning key should rotate it immediately if the off-chain
> driver's signing key is suspected of compromise. Because rotation is a
> single-call owner-only operation with an emitted event, recovery is
> observable and atomic.

### Request ID Idempotency (Issue #249)

- [x] `deduct` and `batch_deduct` treat `request_id` as a single-use
  idempotency key when it is provided
- [x] Duplicate `request_id` values are rejected before any balance mutation,
  transfer, or event emission
- [x] Batch validation rejects both replayed request ids from prior calls and
  duplicate ids repeated inside the same batch
- [x] Unit tests cover duplicate single-call replay and duplicate-in-batch
  rejection with atomic balance assertions

### External Calls

- [ ] Token transfers strictly rely on `soroban_sdk::token::Client` — tracked in [#272](https://github.com/CalloraOrg/Callora-Contracts/issues/272)
- [ ] Cross-contract calls handle potential errors/panics gracefully — tracked in [#273](https://github.com/CalloraOrg/Callora-Contracts/issues/273)
- [ ] State changes are persisted before making cross-contract calls to mitigate subtle state-caching issues — tracked in [#274](https://github.com/CalloraOrg/Callora-Contracts/issues/274)
- [ ] Checks-effects-interactions pattern followed — tracked in [#275](https://github.com/CalloraOrg/Callora-Contracts/issues/275)

### Revenue Routing External Transfers (Issue #110)

The vault performs USDC transfers to configurable counterpart addresses on every
`deduct` and `batch_deduct` call. These external transfers are justified as follows:

- **settlement address**: set and updated exclusively by the on-chain admin via
  `set_settlement`. This function emits a `set_settlement` event to provide a
  clear audit trail for address rotation. Transfers to this address implement
  the documented `Vault → Settlement` revenue flow described in
  `SETTLEMENT_IMPLEMENTATION.md`.
- **revenue_pool address**: retained as an informational configuration slot via
  `set_revenue_pool` / `get_revenue_pool`. It is **no longer consulted during
  deducts** â€” `deduct` and `batch_deduct` always route to the settlement address.
- **CRITICAL â€” Settlement Required (Issue #263)**: `deduct` and `batch_deduct`
  panic with `"settlement address not set"` when `set_settlement` has not been
  called. The panic occurs before any balance mutation or event emission, so
  the transaction reverts atomically with no observable state change. This
  closes the silent-loss-of-accounting window where the internal `balance`
  could previously decrement without a corresponding on-ledger USDC transfer.
- **Address Validation**: Both `set_settlement()` and `set_revenue_pool()` validate
  that the provided address is NOT the vault's own address, preventing
  self-referential routing loops.
- **Atomic Updates**: Each address is updated atomically in a single storage write,
  ensuring no partial update is observable by other callers.
- **Audit Trail**: All routing configuration changes emit events:
  - `set_settlement(admin) → address` when setting settlement
  - `set_revenue_pool(admin) → address` when setting revenue pool
  - `clear_revenue_pool(admin) → ()` when clearing revenue pool

### Vault-Specific Risks

- [ ] Deposit/withdraw invariants tested — tracked in [#276](https://github.com/CalloraOrg/Callora-Contracts/issues/276)
- [ ] Vault balance accounting verified — tracked in [#277](https://github.com/CalloraOrg/Callora-Contracts/issues/277)
- [ ] Funds cannot be locked permanently — tracked in [#278](https://github.com/CalloraOrg/Callora-Contracts/issues/278)
- [ ] Minimum deposit requirements enforced — tracked in [#279](https://github.com/CalloraOrg/Callora-Contracts/issues/279)
- [x] Maximum deduction limits enforced (`get_max_deduct` / `set_max_deduct`) with explicit positive-value validation and dedicated unit tests.
- [x] Revenue pool transfers validated
- [x] Settlement developer address required when routing to specific developer.
- [x] Settlement developer address must be None when routing to global pool.
- [ ] Batch operations respect individual limits — tracked in [#280](https://github.com/CalloraOrg/Callora-Contracts/issues/280)

### Revenue Pool Security Assumptions

The Revenue Pool contract (`contracts/revenue_pool`) operates under the following security assumptions and threat models:

- **Malicious Admin:** The `admin` role has the authority to distribute funds and replace the admin address. A compromised or malicious admin could drain the pool's USDC balance.
  - *Mitigation:* The `admin` should always be a heavily guarded multisig account or a rigorously audited governance contract.

- **Wrong USDC Token Initialization:** The `usdc_token` address is set once during `init`. If initialized with a malicious or incorrect token address, the pool will process the wrong asset.
  - *Mitigation:* The deployment process must verify the official Stellar USDC (or appropriate wrapped USDC) contract address before initialization. The `init` function guards against re-initialization.

- **Operational Griefing (Balances):** Anyone can effectively transfer USDC to the revenue pool. If an attacker sends unsolicited funds, it increases the `balance()` but does not disrupt the `distribute` logic, as distribution is explicitly controlled by the admin.
  - *Mitigation:* The pool does not rely on strict balance equality invariants for its core operations, mitigating balance-based operational griefing. The `receive_payment` entrypoint is admin-only and event-only (no token movement), so indexers should reconcile `receive_payment` logs with actual token transfers.

- **Resource Exhaustion via Unbounded Batch:** `batch_distribute` accepts a `Vec<(Address, i128)>`. Without a cap, a compromised admin key could submit thousands of entries, exhausting Soroban's per-transaction CPU/memory budget and causing unpredictable mid-execution failures.
  - *Mitigation:* `batch_distribute` enforces `1 <= payments.len() <= MAX_BATCH_SIZE` (currently **50**), matching the vault's `batch_deduct` cap. Empty vectors and oversized vectors are rejected before any iteration or USDC transfer occurs. The cap keeps resource consumption well within Soroban network limits.

- **Excessive Single-Leg Distribution:** A compromised admin could still try to distribute a huge amount in a single `distribute()` or individual `batch_distribute` leg, increasing the blast radius for a compromised admin key.
  - *Mitigation:* `callora-revenue-pool` now exposes a configurable `max_distribute` cap. Every `distribute` and every individual `batch_distribute` payment leg is validated against this cap. The cap is admin-gated, must be positive, and defaults to `i128::MAX` until configured.

- **Emergency Pause Delegation:** The admin can configure a `pause_guardian` for operational emergencies where the pool needs to be stopped quickly without sharing full admin power.
  - *Mitigation:* `pause_guardian` is scoped to `pause` only. It cannot unpause, distribute funds, rotate admin, update caps, clear or replace itself, or upgrade the contract. `set_pause_guardian` and `clear_pause_guardian` are admin-only and emit dedicated events for monitoring.

- **Emergency Drain (Multisig + Timelock):** The admin can move the entire pool balance to a treasury address as a last-resort evacuation. Without controls, this could be exploited by a compromised admin key.
  - *Mitigation:* `propose_emergency_drain` stores a `PendingEmergencyDrain` snapshot. `execute_emergency_drain` is only callable after `EMERGENCY_DRAIN_TIMELOCK_SECONDS` (86 400 s = 24 h) have elapsed, giving operators a window to call `cancel_emergency_drain` before funds move. A new proposal replaces any existing one, resetting the clock. When the admin is a Stellar multisig account, `require_auth` enforces the native multi-signature threshold automatically — no additional multi-sig logic is required in the contract. Every state change emits an audit event (`emergency_drain_proposed`, `emergency_drain_executed`, `emergency_drain_cancelled`).

### Input Validation

- [ ] All amounts validated to be > 0 — tracked in [#281](https://github.com/CalloraOrg/Callora-Contracts/issues/281)
- [ ] Address/parameter validation on all public functions — tracked in [#282](https://github.com/CalloraOrg/Callora-Contracts/issues/282)
- [ ] Boundary conditions tested (max values, zero values) — tracked in [#283](https://github.com/CalloraOrg/Callora-Contracts/issues/283)
- [ ] Error messages provide clear context for debugging — tracked in [#284](https://github.com/CalloraOrg/Callora-Contracts/issues/284)
- `callora-vault::init` enforces `min_deposit > 0`; omitted values default to `1`.

### Event Logging

- [ ] All state changes emit appropriate events — tracked in [#285](https://github.com/CalloraOrg/Callora-Contracts/issues/285)
- [ ] Event schema documented and indexed — tracked in [#286](https://github.com/CalloraOrg/Callora-Contracts/issues/286)
- [ ] Critical operations (deposit, withdraw, deduct) logged with full context — tracked in [#287](https://github.com/CalloraOrg/Callora-Contracts/issues/287)
- [x] Unit tests assert `deposit` and `deduct` event topics/data (caller, request_id semantics, and resulting balance).
- [x] `callora-revenue-pool` emits `admin_changed` carrying `(old_admin, new_admin)` only from `accept_admin()` / `claim_admin()`. Nomination (`set_admin`) emits `admin_transfer_started` alone and cancellation emits `admin_cancelled` alone, so no `admin_changed` event exists unless the transfer completed; unit tests pin topics/data for all three cases.

### Testing Coverage

- [x] Unit tests cover all public functions
- [x] Edge cases and boundary conditions tested
- [x] Panic scenarios tested with `#[should_panic]`
- [ ] Integration tests for complete user flows — tracked in [#288](https://github.com/CalloraOrg/Callora-Contracts/issues/288)
- [x] Minimum 95% test coverage maintained (enforced via `cargo tarpaulin` with `fail-under = 95.0`)

## External Audit Recommendation

Before any mainnet deployment:

- **Engage an independent third-party security auditor**
  - Choose auditors with experience in Soroban/Stellar smart contracts
  - Ensure auditor understands vault-specific risk patterns

- **Perform a full smart contract audit**
  - Review all contract code for security vulnerabilities
  - Analyze upgrade patterns and migration paths
  - Validate mathematical correctness of balance operations

- **Address all high and medium severity findings**
  - Create tracking system for audit findings
  - Implement fixes for all H/M severity issues
  - Document rationale for any low severity findings that won't be fixed

- **Publish audit report for transparency**
  - Make audit report publicly available
  - Include summary of findings and remediation steps
  - Provide evidence of test coverage and validation

## Additional Security Considerations

### Soroban-Specific Security

- [ ] WASM compilation verified and reproducible (`stellar contract build` / `cargo build --target wasm32-unknown-unknown --release`) — tracked in [#289](https://github.com/CalloraOrg/Callora-Contracts/issues/289)
- [ ] Storage lifespan (`extend_ttl`) implemented to prevent state archiving for critical data — tracked in [#290](https://github.com/CalloraOrg/Callora-Contracts/issues/290)
- [ ] Stellar network parameters validated (budget, CPU/RAM limits) — tracked in [#291](https://github.com/CalloraOrg/Callora-Contracts/issues/291)
- [ ] Cross-contract call security and generic type usage (`Val`) reviewed — tracked in [#292](https://github.com/CalloraOrg/Callora-Contracts/issues/292)
- [ ] Storage patterns optimized and secure (e.g., correct usage of `persistent` vs `instance` vs `temporary` keys) — tracked in [#293](https://github.com/CalloraOrg/Callora-Contracts/issues/293)

### Economic Security

- [ ] Fee structures reviewed for economic attacks — tracked in [#294](https://github.com/CalloraOrg/Callora-Contracts/issues/294)
- [ ] Revenue pool distribution validated — tracked in [#295](https://github.com/CalloraOrg/Callora-Contracts/issues/295)
- [ ] Maximum loss scenarios analyzed — tracked in [#296](https://github.com/CalloraOrg/Callora-Contracts/issues/296)
- [ ] Slippage and market impact considered — tracked in [#297](https://github.com/CalloraOrg/Callora-Contracts/issues/297)

### Operational Security

- [ ] Deployment process documented and automated — tracked in [#298](https://github.com/CalloraOrg/Callora-Contracts/issues/298)
- [ ] Key management procedures established — tracked in [#299](https://github.com/CalloraOrg/Callora-Contracts/issues/299)
- [ ] Monitoring and alerting configured — tracked in [#300](https://github.com/CalloraOrg/Callora-Contracts/issues/300)
- [ ] Incident response plan prepared — tracked in [#301](https://github.com/CalloraOrg/Callora-Contracts/issues/301)

## Security Resources

- [Stellar Security Best Practices](https://developers.stellar.org/docs/security/)
- [Soroban Documentation](https://developers.stellar.org/docs/smart-contracts/)
- [Smart Contract Weakness Classification Registry](https://swcregistry.io/)

---

**Note**: This checklist should be reviewed and updated regularly as new security patterns emerge and the codebase evolves.

## Vulnerability Disclosure

We take the security of Callora contracts seriously. If you believe you have
found a vulnerability, please report it responsibly.

- **Contact:** security@callora.org (PGP key fingerprint published in
  `docs/AUDIT_BUNDLE.md`).
- **Do not** open a public GitHub issue for undisclosed vulnerabilities.
- **Include:** affected contract and entrypoint, a minimal reproduction
  (test or transaction sequence), impact assessment, and any suggested fix.
- **Acknowledgement:** we aim to acknowledge reports within 72 hours and
  provide a remediation timeline within 7 days.
- **Coordinated disclosure:** please allow up to 90 days before public
  disclosure so a fix can be deployed and audited.
- **Safe harbour:** good-faith research that respects user funds and privacy
  will not be pursued legally.

See `docs/AUDIT_BUNDLE.md` for the current audit scope, artifacts, and
disclosure key material.

## require_auth() Audit (Issue #160)

All privileged entrypoints across `vault`, `revenue_pool`, and `settlement` contracts
have been audited for `require_auth()` coverage as part of Issue #160.

### Findings
- All privileged functions call `require_auth()` on the caller before executing. ✅
- Negative tests added to each crate's `test.rs` confirming unauthenticated calls are rejected.

### Intentional Exceptions
| Contract   | Function         | Reason |
|------------|------------------|--------|
| settlement | `init()`         | One-time initializer guarded by already-initialized panic; no auth required by design. |
| vault      | `require_owner()`| Internal helper using `assert!` for address equality. All public callers invoke `caller.require_auth()` before calling this helper, so host-level auth is enforced transitively. Documented gap: `require_owner` itself does not call `require_auth()`. |
| revenue_pool | `distribute::init()` | Currently callable without authentication; tracked in [#265](https://github.com/CalloraOrg/Callora-Contracts/issues/265). |
| revenue_pool | `set_upgrade_cooldown()` | Currently has no auth gate; tracked in [#266](https://github.com/CalloraOrg/Callora-Contracts/issues/266). |

### Cross-reference
- Audit branch: `test/require-auth-sweep`
- Tests: `contracts/vault/src/test.rs`, `contracts/revenue_pool/src/test.rs`, `contracts/settlement/src/test.rs`

## Authorization Matrix Update (Settlement)

As part of the authorization matrix hardening for the `callora-settlement` contract:
- `get_all_developer_balances` now requires `admin` authorization via `require_auth()`. This prevents bulk data scraping while allowing administrative oversight.
- Comprehensive negative tests have been added to `contracts/settlement/src/test.rs` covering `receive_payment`, `set_admin`, `set_vault`, and `get_all_developer_balances`.
- Overflow regression tests now assert `receive_payment` panics with `"pool balance overflow"` and `"developer balance overflow"` when credits would exceed `i128::MAX`.
- Admin rotation (two-step) has been verified to correctly gate access during the transition period.
