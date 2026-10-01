# Callora Cold — Capability Bitmap

The cold surface exposes a `capabilities()` view that returns a `u64` bitmask.
Each set bit indicates a cold-storage feature supported by this deployment so
clients can detect capability deltas across upgrades without parsing versions.

Cold accounting is not currently exposed by the vault. The historical bit
identifiers remain reserved for compatibility, but this deployment advertises
no cold-storage capabilities until the vault entrypoints ship.

## Querying capabilities

```typescript
const caps = await coldClient.capabilities();
// The current deployment returns 0; no cold flow is available yet.
if (caps === 0) {
  // Do not attempt cold-storage operations.
}
```

```rust
let caps: u64 = CalloraColdClient::new(&env, &cold).capabilities();
assert_eq!(caps, 0);
```

## Detecting deltas

```typescript
const before = await oldClient.capabilities();
const after = await newClient.capabilities();
const added = after & ~before;
const removed = before & ~after;
```

## Bit registry

| Bit | Hex | Constant | Feature | Introduced |
|-----|-----|----------|---------|------------|
| 0–6 | `0x01`–`0x40` | Historical `CAP_*` identifiers | Reserved; not currently exposed by the vault | — |
| 7–63 | — | *(reserved)* | Always zero | — |

## Stability guarantee

- A bit position is assigned once and never reused for a different feature.
- Removed features keep their bit **cleared**; the position stays reserved.
- New features may occupy an available bit only when the corresponding vault functionality ships.
- Reserved bits (7–63) are always `0` in the current version.
