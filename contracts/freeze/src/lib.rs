//! Freeze (circuit-breaker) harness for Callora contracts.
//!
//! This crate provides the [`CalloraFreeze`] contract that wraps the revenue-pool
//! **pause circuit-breaker** (`freeze` / `unfreeze` / `is_frozen`), which blocks
//! `distribute` and `batch_distribute` while active. Every state-changing
//! entrypoint calls `require_auth` on the acting `Address`.
//!
//! # Auth Model
//!
//! | Entrypoint            | Authorized by                         |
//! |-----------------------|---------------------------------------|
//! | `init`                | `admin.require_auth()`               |
//! | `freeze`              | `caller.require_auth()` — admin or freeze operator |
//! | `unfreeze`            | `caller.require_auth()` — admin only |
//! | `set_freeze_operator` | `caller.require_auth()` — admin only |
//!
//! # Events
//!
//! Every state-changing entrypoint emits exactly one event. Topic[1] is always
//! `"callora_v1"` for version filtering.
//!
//! | Event topic            | Entrypoint            | Data                                         |
//! |------------------------|-----------------------|----------------------------------------------|
//! | `freeze_initialized`   | `init`                | `()`                                         |
//! | `freeze_set`           | `freeze`              | `FreezeSetEvent { reason, frozen_at }`       |
//! | `freeze_cleared`       | `unfreeze`            | `()`                                         |
//! | `freeze_operator_set`  | `set_freeze_operator` | `FreezeOperatorSetEvent { old, new }`        |
//!
//! # Fuzzing
//!
//! See `fuzz/targets/main.rs` — a `cargo-fuzz` target that feeds malformed
//! operation sequences into freeze/unfreeze and asserts safety invariants.
//!
//! See also `tests/malformed_freeze.rs` for manual freeze-scenario tests
//! against the underlying `RevenuePool` contract.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

pub mod errors;
pub mod events;

pub use errors::FreezeError;
pub use events::{FreezeOperatorSetEvent, FreezeSetEvent};

/// One step in a freeze/unfreeze fuzz sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeOp {
    /// Attempt pause as admin.
    FreezeAsAdmin,
    /// Attempt pause as configured guardian.
    FreezeAsGuardian,
    /// Attempt pause as an unauthorized outsider.
    FreezeAsOutsider,
    /// Attempt unpause as admin.
    UnfreezeAsAdmin,
    /// Attempt unfreeze as outsider (must fail).
    UnfreezeAsOutsider,
    /// Attempt distribute while possibly frozen (malformed amount allowed).
    Distribute { amount: i128 },
    /// Toggle / clear guardian mid-sequence.
    SetGuardian,
    /// Clear guardian.
    ClearGuardian,
}

impl FreezeOp {
    /// Decode a raw fuzzer byte into an operation (covers all variants).
    pub fn from_byte(b: u8, amount_lo: u8, amount_hi: u8) -> Self {
        let amount = i128::from(u16::from_be_bytes([amount_lo, amount_hi]));
        match b % 8 {
            0 => Self::FreezeAsAdmin,
            1 => Self::FreezeAsGuardian,
            2 => Self::FreezeAsOutsider,
            3 => Self::UnfreezeAsAdmin,
            4 => Self::UnfreezeAsOutsider,
            5 => Self::Distribute { amount },
            6 => Self::SetGuardian,
            _ => Self::ClearGuardian,
        }
    }

    /// Decode a byte slice into a bounded operation list.
    pub fn decode_sequence(data: &[u8], max_ops: usize) -> Vec<Self> {
        let mut ops = Vec::new();
        let mut i = 0;
        while i < data.len() && ops.len() < max_ops {
            let b = data[i];
            let lo = data.get(i + 1).copied().unwrap_or(0);
            let hi = data.get(i + 2).copied().unwrap_or(0);
            ops.push(Self::from_byte(b, lo, hi));
            i = i.saturating_add(3);
            if i == 0 {
                break;
            }
        }
        ops
    }
}

/// Maximum operations executed per fuzz / unit-test invocation.
pub const MAX_FREEZE_OPS: usize = 64;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Frozen,
    FreezeOperator,
    /// The `Symbol` reason supplied to the last successful `freeze()` call.
    FreezeReason,
    /// The ledger timestamp (`env.ledger().timestamp()`) of the last successful
    /// `freeze()` call.
    FreezeTimestamp,
}

// ---------------------------------------------------------------------------
// View types
// ---------------------------------------------------------------------------

/// Snapshot returned by [`CalloraFreeze::get_freeze_status`].
///
/// Combines the current frozen flag, the persisted reason, and the timestamp
/// into a single call, avoiding multiple round-trips for off-chain monitors.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeStatus {
    /// `true` when the circuit-breaker is active.
    pub frozen: bool,
    /// The reason label from the last `freeze()` call, or `None` if the
    /// contract was never frozen or has been unfrozen and the field cleared.
    pub reason: Option<Symbol>,
    /// The ledger timestamp of the last `freeze()` call, or `None` if the
    /// contract was never frozen or has been unfrozen and the field cleared.
    pub frozen_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Callora circuit-breaker contract.
///
/// Wraps a pause/unpause surface with `require_auth` on every state-changing
/// entrypoint.
#[contract]
pub struct CalloraFreeze;

#[contractimpl]
impl CalloraFreeze {
    /// Initialise the contract with an `admin` address.
    ///
    /// Emits a `freeze_initialized` event on success.
    ///
    /// # Arguments
    /// * `admin` — Address authorised to freeze, unfreeze, and set the freeze
    ///   operator.
    ///
    /// # Errors
    /// * [`FreezeError::AlreadyInitialized`] — admin already set.
    pub fn init(env: Env, admin: Address) -> Result<(), FreezeError> {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(FreezeError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Frozen, &false);
        events::emit_freeze_initialized(&env, &admin);
        Ok(())
    }

    /// Return the stored admin address.
    ///
    /// # Errors
    /// * [`FreezeError::NotInitialized`] — `init` has not been called.
    pub fn get_admin(env: Env) -> Result<Address, FreezeError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FreezeError::NotInitialized)
    }

    /// Return `true` if the contract is currently frozen.
    pub fn is_frozen(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Frozen)
            .unwrap_or(false)
    }

    /// Return a snapshot of the current freeze state, last reason, and
    /// timestamp in one call.
    ///
    /// `reason` and `frozen_at` are populated while the contract is frozen and
    /// cleared on `unfreeze`. They are both `None` before the first `freeze`
    /// and after any subsequent `unfreeze`.
    ///
    /// # Errors
    /// * [`FreezeError::NotInitialized`] — `init` has not been called.
    pub fn get_freeze_status(env: Env) -> Result<FreezeStatus, FreezeError> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(FreezeError::NotInitialized);
        }
        let frozen = env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::Frozen)
            .unwrap_or(false);
        let reason: Option<Symbol> = env.storage().instance().get(&DataKey::FreezeReason);
        let frozen_at: Option<u64> = env.storage().instance().get(&DataKey::FreezeTimestamp);
        Ok(FreezeStatus {
            frozen,
            reason,
            frozen_at,
        })
    }

    /// Activate the circuit-breaker.
    ///
    /// Persists `reason` and the current ledger timestamp, then emits a
    /// `freeze_set` event.
    ///
    /// The admin or the configured freeze operator may call.
    ///
    /// # Arguments
    /// * `caller` — Must be the admin or freeze operator; must authorise.
    /// * `reason` — Opaque label persisted in storage and emitted in the event
    ///   for off-chain indexers and the `get_freeze_status` view.
    ///
    /// # Errors
    /// * [`FreezeError::Unauthorized`] — caller is neither admin nor operator.
    /// * [`FreezeError::AlreadyFrozen`] — contract is already frozen.
    pub fn freeze(env: Env, caller: Address, reason: Symbol) -> Result<(), FreezeError> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone())?;
        let operator: Option<Address> = env.storage().instance().get(&DataKey::FreezeOperator);

        let is_authorized = caller == admin || operator.map_or(false, |op| caller == op);
        if !is_authorized {
            return Err(FreezeError::Unauthorized);
        }
        if Self::is_frozen(env.clone()) {
            return Err(FreezeError::AlreadyFrozen);
        }

        let frozen_at = env.ledger().timestamp();
        env.storage().instance().set(&DataKey::Frozen, &true);
        env.storage()
            .instance()
            .set(&DataKey::FreezeReason, &reason);
        env.storage()
            .instance()
            .set(&DataKey::FreezeTimestamp, &frozen_at);

        events::emit_freeze_set(&env, &caller, reason, frozen_at);
        Ok(())
    }

    /// Deactivate the circuit-breaker. Only the admin may call.
    ///
    /// Clears the persisted reason and timestamp, then emits a `freeze_cleared`
    /// event.
    ///
    /// # Errors
    /// * [`FreezeError::Unauthorized`] — caller is not the admin.
    /// * [`FreezeError::NotFrozen`] — contract is not currently frozen.
    pub fn unfreeze(env: Env, caller: Address) -> Result<(), FreezeError> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone())?;
        if caller != admin {
            return Err(FreezeError::Unauthorized);
        }
        if !Self::is_frozen(env.clone()) {
            return Err(FreezeError::NotFrozen);
        }
        env.storage().instance().set(&DataKey::Frozen, &false);
        env.storage().instance().remove(&DataKey::FreezeReason);
        env.storage().instance().remove(&DataKey::FreezeTimestamp);

        events::emit_freeze_cleared(&env, &caller);
        Ok(())
    }

    /// Set or replace the freeze operator.
    ///
    /// The operator may call `freeze` but has no authority to `unfreeze`,
    /// set the operator, or exercise any other admin-only power.
    ///
    /// Pass `None` to revoke the operator role.
    ///
    /// Emits a `freeze_operator_set` event with the previous and new operator
    /// addresses (either may be `None`).
    ///
    /// # Errors
    /// * [`FreezeError::Unauthorized`] — caller is not the admin.
    pub fn set_freeze_operator(
        env: Env,
        caller: Address,
        operator: Option<Address>,
    ) -> Result<(), FreezeError> {
        caller.require_auth();
        let admin = Self::get_admin(env.clone())?;
        if caller != admin {
            return Err(FreezeError::Unauthorized);
        }

        let old_operator: Option<Address> = env.storage().instance().get(&DataKey::FreezeOperator);

        match operator.clone() {
            Some(ref op) => env.storage().instance().set(&DataKey::FreezeOperator, op),
            None => env.storage().instance().remove(&DataKey::FreezeOperator),
        }

        events::emit_freeze_operator_set(&env, &caller, old_operator, operator);
        Ok(())
    }

    /// Return the configured freeze operator, or `None` if unset.
    pub fn get_freeze_operator(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::FreezeOperator)
    }
}

#[cfg(test)]
mod test;
