//! Typed error codes for the Callora Yield per-account limits surface.
//!
//! The numeric discriminants in this enum are part of the contract interface
//! and must remain stable over time. Off-chain indexers and SDK integrators
//! branch on these `u32` codes instead of parsing panic strings.
//!
//! | Code | Variant               | Meaning                                                       |
//! |------|-----------------------|---------------------------------------------------------------|
//! | 1    | NotInitialized        | Contract has not been initialized yet                         |
//! | 2    | AlreadyInitialized    | `init` was called more than once                              |
//! | 3    | Unauthorized          | Caller is not authorized for the requested operation          |
//! | 4    | InvalidLimit          | One or more per-account caps are negative or otherwise invalid|
//! | 5    | BetsAtCap             | Account's open bet count is already at the configured cap     |
//! | 6    | PositionsAtCap        | Account's open position count is already at the configured cap|
//! | 7    | SubscriptionsAtCap    | Account's active subscription count is at the configured cap |
//! | 8    | CounterUnderflow      | `clear_*` was called when the corresponding counter was zero  |
//! | 9    | Overflow              | `checked_add` overflow detected on a counter increment        |
//! | 10   | UpgradeRejected       | Storage-migration validator rejected the pending upgrade      |
//!
//! All variants implement [`Copy`] + [`PartialEq`] so they can be returned by
//! value or pinned in arrays without allocation.

use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the per-account yield limits
/// surface.
#[contracterror]
#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum YieldLimitError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// Contract has already been initialized (code 2).
    AlreadyInitialized = 2,
    /// Caller is not authorized for the requested operation (code 3).
    Unauthorized = 3,
    /// One or more per-account caps are negative or otherwise invalid (code 4).
    ///
    /// Returned by [`crate::limits::set_account_limits`] and
    /// [`crate::limits::set_default_limits`] when any of `max_bets`,
    /// `max_positions`, or `max_subscriptions` is negative, or when an
    /// `AccountLimits` struct is otherwise invalid for storage.
    InvalidLimit = 4,
    /// Account's open bet count is already at the configured cap (code 5).
    ///
    /// Returned by [`crate::limits::place_bet`] when an increment would push
    /// the account's open bet counter above `max_bets`.
    BetsAtCap = 5,
    /// Account's open position count is already at the configured cap (code 6).
    ///
    /// Returned by [`crate::limits::open_position`] when an increment would
    /// push the account's open position counter above `max_positions`.
    PositionsAtCap = 6,
    /// Account's active subscription count is already at the configured cap (code 7).
    ///
    /// Returned by [`crate::limits::subscribe`] when an increment would push
    /// the active subscription counter above `max_subscriptions`.
    SubscriptionsAtCap = 7,
    /// `clear_*` was called when the corresponding counter was already zero (code 8).
    CounterUnderflow = 8,
    /// `checked_add` overflow detected on a counter increment (code 9).
    ///
    /// Returned in the (effectively unreachable) case that the per-account
    /// counter arithmetic saturates `u32::MAX` — surfaces a stable error
    /// rather than panicking so callers cannot rely on undefined behaviour.
    Overflow = 9,
    /// The storage-migration validator rejected the pending upgrade (code 10).
    ///
    /// Returned by [`crate::limits::CalloraYieldLimits::upgrade`] when the
    /// pre-upgrade guard rejects an all-zero WASM hash, a version skip, an
    /// unsanctioned rollback, or a silent storage-layout change.
    UpgradeRejected = 10,
}
