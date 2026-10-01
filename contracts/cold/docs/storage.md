# Callora Cold — Storage Status

The cold-storage partition is not currently exposed by the vault. The planned
`ColdConfig`, `ColdBalances`, and `PendingColdSweep` records therefore have no
active storage keys, and `callora-cold::capabilities()` returns `0` until the
vault entrypoints are implemented.

The following names are retained as historical design references only. They
are not a storage schema and must not be treated as available contract API.

## Planned records

- `ColdConfig` — the future hot/cold ratio, rebalance threshold, signer set,
  and approval threshold.
- `ColdBalances` — the future accounting partition whose `hot + cold` value
  would equal the vault's tracked balance.
- `PendingColdSweep` — the future single pending N-of-M sweep proposal.

When this functionality is implemented, the storage tier, authorization
rules, accounting invariants, and TTL behavior must be documented here before
the corresponding capability bits are advertised.
