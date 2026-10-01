# Revenue Pool Yield Deposits

`deposit_yield(treasury, amount, source)` lets the current revenue-pool admin
deposit accumulated protocol earnings into the pool through one audited
entrypoint.

## Behavior

- `treasury` must be the current admin and must authorize the call.
- `amount` must be positive and is transferred from `treasury` to the revenue
  pool contract using the configured USDC token contract.
- `source` is a short Soroban `Symbol` label for indexers, such as `fees` or
  `yield`.
- `get_cumulative_yield_deposited()` returns the total amount deposited through
  this entrypoint.

## Overflow handling

The cumulative counter is updated with `checked_add`. If the new total would
exceed `int128_MAX`, the call fails with an `Overflow` error. The overflow
 check runs before the token transfer and before any state is persisted, so a
failed deposit leaves both the token balances and the cumulative counter
unchanged.

## Event

Each successful deposit emits:

```text
topics: ["yield_deposited", treasury]
data:   (amount, source, cumulative_yield_deposited)
```

The token transfer runs before the cumulative metric update and event emission.
All three effects are part of the same Soroban transaction: if the USDC callee
reverts or panics, the metric and event are not committed.

Cross-contract call safety for this path is covered by
`contracts/yield/tests/xcontract.rc`.

## Tests

`contracts/revenue_pool/src/lib.rs` covers the following acceptance
criteria under the `yield` test group:

- Overflowing deposit fails with `Overflow`.
- Token balances are unchanged after the failure.
- The cumulative counter is unchanged after the failure.
- A non-admin treasury calling `deposit_yield` fails with `Unauthorized`.

Run them:

```
cargo test -p callora-revenue-pool yield
```
