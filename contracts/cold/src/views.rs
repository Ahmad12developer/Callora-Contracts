//! Read-only capability views for Callora cold storage.
//!
//! Each bit in the `u64` returned by [`capabilities`] represents a distinct,
//! stable cold-storage feature. Bits are assigned once and never reassigned,
//! so clients can detect capability deltas across upgrades with simple
//! bitwise comparison:
//!
//! ```ignore
//! let before = old_client.capabilities();
//! let after = new_client.capabilities();
//! let added = after & !before;
//! let removed = before & !after;
//! ```
//!
//! # Cold-storage status
//!
//! The vault does not currently expose the cold-storage entrypoints described
//! by these historical bit identifiers. They remain exported so clients can
//! continue to compile against the stable capability registry, but no cold
//! capability is advertised until the vault implementation ships.

use soroban_sdk::Env;

/// Historical bit 0 — hot/cold split. Reserved until the vault exposes the
/// corresponding entrypoints.
pub const CAP_HOT_COLD_SPLIT: u64 = 1 << 0;

/// Historical bit 1 — automatic hot-to-cold rebalance. Reserved until the
/// vault exposes the corresponding entrypoints.
pub const CAP_AUTO_REBALANCE: u64 = 1 << 1;

/// Historical bit 2 — N-of-M cold sweep. Reserved until the vault exposes the
/// corresponding entrypoints.
pub const CAP_COLD_MULTISIG_SWEEP: u64 = 1 << 2;

/// Historical bit 3 — hot/cold ratio update. Reserved until the vault exposes
/// the corresponding entrypoints.
pub const CAP_SET_HOT_COLD_RATIO: u64 = 1 << 3;

/// Historical bit 4 — cold signer set update. Reserved until the vault exposes
/// the corresponding entrypoints.
pub const CAP_SET_COLD_SIGNERS: u64 = 1 << 4;

/// Historical bit 5 — cold balance view. Reserved until the vault exposes the
/// corresponding entrypoints.
pub const CAP_COLD_BALANCE_VIEW: u64 = 1 << 5;

/// Historical bit 6 — pending cold-sweep view. Reserved until the vault
/// exposes the corresponding entrypoints.
pub const CAP_PENDING_COLD_SWEEP_VIEW: u64 = 1 << 6;

// Bits 7–63 are reserved for future cold capabilities and are always zero.

/// Bitmask of cold capabilities exposed by this version.
///
/// Cold storage is not exposed by the vault yet, so the advertised mask is
/// intentionally empty. The historical bit constants above remain reserved
/// and are not reused for another meaning.
pub const ALL_CAPABILITIES: u64 = 0;

/// Return the cold capability bitmap.
///
/// Pure view: ignores `_env` (no storage reads). Authentication is not
/// required. Reserved bits (7–63) are always clear.
pub fn capabilities(_env: &Env) -> u64 {
    ALL_CAPABILITIES
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn capabilities_equals_all_mask() {
        let env = Env::default();
        assert_eq!(capabilities(&env), ALL_CAPABILITIES);
    }

    #[test]
    fn reserved_bits_are_clear() {
        let env = Env::default();
        let caps = capabilities(&env);
        assert_eq!(caps & !((1u64 << 7) - 1), 0);
    }
}
