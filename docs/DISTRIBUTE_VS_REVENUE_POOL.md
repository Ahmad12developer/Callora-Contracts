# Distribute vs Revenue Pool: Roles and Responsibilities

## Purpose

This note clarifies the intended role of `callora-distribute` and
`callora-revenue-pool`, which currently share nearly identical admin-gated
`distribute` / `batch_distribute`, pause, max-cap, and upgrade mechanisms.
It exists to prevent operator confusion about which contract should hold
funds and to avoid fixes landing in one copy but not the other.

## Canonical contract for payouts

`callora-distribute` is the **canonical contract for payouts**. All new
distribution flows, integrations, and operational runbooks should target
`contracts/distribute/src/lib.rs`.

`callora-revenue-pool` is retained as a legacy / compatibility surface. It is
expected to be deprecated in favor of `callora-distribute` once all existing
deployments and integrations have migrated. New features should not be added
to the revenue pool unless they are required to maintain backward
compatibility for existing deployments.

## Behavioural differences

The two contracts implement the same broad capabilities but differ in
observability and validation details:

| Aspect                   | `callora-distribute`                          | `callora-revenue-pool`                        |
| ------------------------- | ------------------------------------------ | ----------------------------------------- |
| Event shape               | Emits distinct events for single and batch distribution | Emits events with a different field layout / naming |
| Error handling           | Returns structured errors with explicit variants  | Returns errors with different variant names / messages |
| Duplicate recipient check | Enforced in the canonical implementation      | May be missing or inconsistent in the legacy copy |
| Admin gating             | Present                                         | Present                                         |
| Pause / max cap / upgrade | Present                                      | Present                                         |

When a fix applies to shared behaviour (e.g. duplicate recipient checks), it
MUST be applied to `callora-distribute` first. Applying it only to the
revenue pool is not sufficient and will be treated as an incomplete fix.

## Relationship to `callora-freeze`

`callora-freeze` is meant to protect the **canonical payout contract**,
i.e. `callora-distribute`. Freezing should halt new distributions through
the canonical contract and prevent funds from being moved while an incident is
under investigation. The legacy revenue pool is secondary and is only expected
to be frozen if it still holds funds or remains active in a given deployment.

## Consolidation proposal

The long-term plan is to consolidate distribution logic into
`callora-distribute` and retire `callora-revenue-pool` once all known
deployments have migrated. Until then, both contracts must remain buildable
and tested, and any shared behavioural fix must be applied to both copies with
`callora-distribute treated as the source of truth.

## Summary

- **Canonical payout contract:** `callora-distribute`
- **Legacy / compatibility contract:** `callora-revenue-pool`
- **Freeze target:** `callora-distribute` (with the revenue pool frozen only if still active)
- **New features:** go into `callora-distribute` only
