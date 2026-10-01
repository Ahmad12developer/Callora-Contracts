#![no_std]
//! Callora cold-storage capability surface.
//!
//! The vault does not currently expose the planned hot/cold accounting
//! partition. This crate exposes a **read-only** [`views::capabilities`]
//! bitmap so clients can detect that no cold features are currently
//! supported without parsing version strings.
//!
//! # Quick-start
//! ```ignore
//! let caps = client.capabilities();
//! assert_eq!(caps, 0);
//! ```

mod views;

pub use views::{
    capabilities, ALL_CAPABILITIES, CAP_AUTO_REBALANCE, CAP_COLD_BALANCE_VIEW,
    CAP_COLD_MULTISIG_SWEEP, CAP_HOT_COLD_SPLIT, CAP_PENDING_COLD_SWEEP_VIEW, CAP_SET_COLD_SIGNERS,
    CAP_SET_HOT_COLD_RATIO,
};

use soroban_sdk::{contract, contractimpl, Env};

/// Thin contract facade that exposes cold capability discovery on-chain.
///
/// This entrypoint remains stable while the cold-storage functionality is
/// unshipped; it currently reports an empty capability mask.
#[contract]
pub struct CalloraCold;

#[contractimpl]
impl CalloraCold {
    /// Return the cold-feature capability bitmap for this deployment.
    ///
    /// Pure view: no auth, no storage writes, no TTL bump.
    pub fn capabilities(env: Env) -> u64 {
        views::capabilities(&env)
    }
}
