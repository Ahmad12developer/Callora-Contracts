# Vault Operation Gas / Cost Notes

Approximate resource usage for Callora Vault operations to guide integration and capacity planning. Soroban uses resource metering (CPU instructions, ledger reads/writes, events). Exact numbers depend on network fee configuration and should be validated on testnet or via `soroban contract invoke` simulation.

> **Disclaimer**: The numbers provided below are estimates and should be used contextually. Exact costs diverge between testnet and mainnet depending on real-time network conditions.

## Methodology

Cost estimations are derived by running transaction simulations through `soroban contract invoke --simulate` on recent testnet deployments. Simulated operations log CPU/instruction costs, ledger entry reads/writes, event size, and network fee parameters.

## Relative Cost (typical order)

| Operation | Relative cost | Notes | Estimated CPU Instructions (Testnet)* |
|---|---|---|---|
| `balance()` | Lowest | Single instance read, no writes, no event. | < 500k |
| `get_meta()` | Low | Same as balance (reads full meta). | < 500k |
| `deposit` | Medium | One read, one write, one event. Cross-contract call to USDC. | ~ 2.5M |
| `deduct` | Medium | One read, one write, one event. May cross-call Settlement pool. | ~ 2.8M |
| `withdraw` | Medium | One read, one write, one event. Cross-contract call to USDC. | ~ 2.5M |
| `withdraw_to` | Medium | One read, one write, one event. Cross-contract call to USDC. | ~ 2.5M |
| `distribute`| Medium | One read, one write, one event. Cross-contract call to USDC. | ~ 2.5M |
| `receive_payment`| Low | One event emission. Validates caller and emits an event. | ~ 1.0M |
| `batch_deduct` | Medium–High | One read, one write, N events (one per item). Bulk process. | ~ 3.5M + (100k per item) |
| `init` | Highest | First write (create instance), one event; requires auth. | ~ 4.5M |

## Metadata Validation Note

`callora-vault::set_metadata` and `update_metadata` now run a bounded O(n)
visible-ASCII validation pass before storage. The input is already capped at
256 bytes, so the incremental cost is a single linear scan plus a fixed buffer
copy. This keeps the impact small while rejecting zero-width, bidi-override,
and confusable metadata strings before they reach state.

Release WASM size comparison for `callora-vault` using
`cargo build --target wasm32-unknown-unknown --release -p callora-vault`:
baseline `upstream/main` was 69,465 bytes; this change builds to 69,505 bytes
(+40 bytes). The branch therefore has a minor size impact, though the baseline
artifact is already above the repository's nominal 65,536-byte size target.

*\*These are purely structural estimates. Actual costs fluctuate and must be simulated per deployment.*

## Obtaining Exact Numbers

- **Testnet**: Deploy the vault and invoke each operation; inspect transaction meta for instructions and fee.
- **CLI**: Use `soroban contract invoke` with `--simulate` (or equivalent) and check returned resource/fee info.
- **Test env**: Run the optional benchmark test: `cargo test --ignored vault_operation_costs -- --nocapture`. This logs CPU/instruction and fee estimates per operation when invocation cost metering is enabled in the test environment.

## Fee Configuration

Soroban fees are configured per network (e.g. Pubnet). They are applied to:

- CPU instructions (per increment)
- Ledger entry reads and writes
- Event size
- Transaction size
- Rent for persistent/temporary storage

See [Stellar documentation on Fees and Resource Limits](https://developers.stellar.org/docs/encyclopedia/fees-and-resource-limits) for current fee parameters and a detailed breakdown of metering operations.

# Hot Contract Criterion Benchmarks

The `callora-hot` contract includes `criterion` benchmarks for its hot entrypoints to track performance over time. Benchmarks are located at `contracts/hot/benches/main.rs`.

## Running

```bash
cargo bench -p callora-hot
```

This runs all registered benchmarks and prints per-entrypoint timing statistics.

## Benchmark Targets

| Target | Description |
|---|---|
| `hot/is_paused` | Read paused flag from instance storage |
| `hot/get_admin` | Read current admin address |
| `hot/get_signer` | Read current hot signer address |
| `hot/get_cooldown` | Read configured cool-off window |
| `hot/get_pending_admin` | Read pending admin (two-step rotation) |
| `hot/cooldown_remaining` | Compute remaining cooldown for an action |
| `hot/is_ready` | Check whether an action may run now |
| `hot/pause` | Critical action: set paused flag (cooldown-guarded) |
| `hot/unpause` | Critical action: clear paused flag (cooldown-guarded) |
| `hot/rotate_signer` | Critical action: rotate hot signer (cooldown-guarded) |
| `hot/set_cooldown` | Update global cool-off window |
| `hot/set_admin` | Nominate new admin (two-step rotation) |
| `hot/accept_admin` | Accept pending admin transfer |

Cooldown-guarded critical actions (`pause`, `unpause`, `rotate_signer`) advance the ledger timestamp by `COOLDOWN_SECS + 1` between iterations so each invocation is accepted.

## Baseline

Run `cargo bench -p callora-hot -- --save-baseline main` to capture a baseline. Future runs can be compared with:

```bash
cargo bench -p callora-hot -- --baseline main
```

## Dev-Dependency

`criterion = "0.5"` is added as a dev-dependency in `contracts/hot/Cargo.toml`. This does not affect the production WASM artifact.

# Whitelist Criterion Benchmarks

The `callora-whitelist` package benchmarks the contract's public whitelist
entrypoints with Criterion. The harness is located at
`contracts/whitelist/benches/main.rs`.

## Running

```bash
cargo bench -p callora-whitelist
```

## Benchmark Targets

| Target | Description |
|---|---|
| `whitelist/is_whitelisted/member/32` | Check a member at the end of a 32-address whitelist |
| `whitelist/is_whitelisted/miss/32` | Check a missing address against a 32-address whitelist |
| `whitelist/get_whitelist/32` | Return a 32-address whitelist |
| `whitelist/add_address/empty` | Add the first address |
| `whitelist/add_address/32` | Add an address after 32 existing entries |
| `whitelist/remove_address/32` | Remove the final address from 32 entries |
| `whitelist/clear_all/32` | Clear a 32-address whitelist |

Each state-changing sample uses a newly initialized fixture. Initialization,
list population, and cooldown advancement stay outside the measured operation,
while the measured invocation still executes the entrypoint's authorization
and cooldown checks.

## Baseline

Save a comparison baseline with:

```bash
cargo bench -p callora-whitelist -- --save-baseline main
```

Compare a later run with:

```bash
cargo bench -p callora-whitelist -- --baseline main
```

`criterion = "0.5"` is a development-only dependency and does not affect the
production contract WASM.

# Gas Regression Baseline

The repository maintains a measured CPU/memory baseline at
`contracts/.gas-baseline.json`. Pull requests are gated on this baseline by the
`gas-regression` job in the CI workflow (`.github/workflows/ci.yml`),
which runs `scripts/gas-regression.sh` and fails when any entrypoint grows by
more than 5% in CPU or memory unless the PR carries the `gas-override` label.

## What the check does

1. Builds the workspace and runs the gas measurement tests for `callora-vault`,
`callora-allowlist`, `callora-limits`, and `callora-cold`.
2. Compares each measured entrypoint against `contracts/.gas-baseline.json` using a
5% threshold.
3. Writes a markdown report to `target/gas-report.md` and uploads it as the
`gas-report` artifact on every pull request run.

## Refreshing the baseline

When a regression is intentional (for example, a deliberate storage layout change):

1. Run the measurements and write the new baseline:

   ```bash
   ./scripts/gas-regression.sh --update-baseline
   ```

2. Review the diff in `contracts/.gas-baseline.json` and commit it as part of the PR.
3. Document the rationale in the PR description. Merging the refreshed baseline
is equivalent to accepting the new cost profile.

### Override label

If a regression is known and accepted but the baseline has not yet been
refreshed, add the `gas-override` label to the pull request. The workflow will
still run and upload the report, but the gate will not fail. Remove the label
and refresh the baseline as soon as possible to keep the gate meaningful.

### Running locally

```bash
./scripts/gas-regression.sh
```

The script requires `jq`, `cargo`, and `python3`. Pass `--threshold <percent>`
to override the default 5% threshold for a one-off investigation.
