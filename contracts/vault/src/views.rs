//! # Read-only views for the Callora Vault contract.
//!
//! This module hosts the [`CalloraVault::simulate_deduct`] pre-flight view,
//! which shares [`crate::CalloraVault::validate_deduct`] with
//! [`crate::CalloraVault::deduct`] to guarantee that their validation
//! pipelines are identical.
//!
//! ## Parameters
//! `simulate_deduct(env, caller, amount, request_id)` — mirrors the public
//! signature of `deduct` exactly:
//! - `env` — the contract environment.
//! - `caller` — the address that would call `deduct`; checked against the
//!   authorized-caller role (same check `deduct` performs, but without
//!   `require_auth`).
//! - `amount` — deduct amount in USDC stroops.
//! - `request_id` — `u64` idempotency key, same type as `deduct`.
//!
//! ## Guarantees
//! - **Read-only.** `simulate_deduct` does not write to instance, persistent,
//!   or temporary storage; does not transfer tokens; does not call into the
//!   settlement contract; and does not emit events.
//! - **Auth-free.** It does not call `require_auth`. The `caller` parameter
//!   is checked against the authorized-caller role, but no on-chain
//!   authorization signature is required.
//! - **Parity.** `simulate_deduct` calls `validate_deduct`, which is the same
//!   function `deduct` calls. For any given vault state and inputs, the error
//!   returned by `simulate_deduct` is exactly the error `deduct` would return
//!   at the matching validation step.
//!
//! The simulation stops at the validation stage and does not reach the
//! external call or settlement-credit step — so a vault with no
//! `settlement` configured would simulate as if it were configured.
//! Production callers should still call
//! [`crate::CalloraVault::get_settlement`] before submitting a real `deduct`.

use soroban_sdk::{contractimpl, Address, Env};

use crate::errors::VaultError;
use crate::{CalloraVault, CalloraVaultArgs, CalloraVaultClient};

/// Read-only pre-flight of [`crate::CalloraVault::deduct`].
///
/// Runs [`crate::CalloraVault::validate_deduct`] — the same shared validation
/// function that `deduct` calls — without performing any state mutation.
/// Clients use this to predict whether a real `deduct` call would succeed
/// before signing and submitting a transaction.
///
/// # Parameters
/// - `caller` — address that would call `deduct`. Checked against the
///   authorized-caller role. **Not authenticated** (no `require_auth`).
/// - `amount` — amount to deduct in USDC stroops.
/// - `request_id` — `u64` idempotency key; same type as `deduct`.
///
/// # Returns
/// - `Ok(projected_balance)` — the vault balance after a successful deduct.
/// - `Err(VaultError)` — the same error variant `deduct` would return at the
///   matching validation step.
///
/// # Errors
/// Mirrors `deduct`'s validation errors in the same order:
///
/// 1. [`VaultError::Unauthorized`] — `caller` is neither the owner nor the
///    authorized deduct caller. (Auth is not enforced here, but the role check
///    still runs.)
/// 2. [`VaultError::Paused`] — vault is paused.
/// 3. [`VaultError::AmountNotPositive`] — `amount <= 0`.
/// 4. [`VaultError::BelowMinDeposit`] — `amount < min_deposit`.
/// 5. [`VaultError::ExceedsMaxDeduct`] — `amount > max_deduct`.
/// 6. [`VaultError::InsufficientBalance`] — vault balance < `amount`.
/// 7. [`VaultError::DuplicateRequestId`] — non-zero `request_id` already
///    processed.
///
/// [`VaultError::SettlementNotSet`] and [`VaultError::NotInitialized`] are
/// **not** raised here — the simulation does not reach the external settlement
/// call. Callers that need this assurance should additionally call
/// [`crate::CalloraVault::get_settlement`] before submitting.
#[contractimpl]
impl CalloraVault {
    /// Read-only pre-flight of `deduct`. See the [`crate::views`] module docs.
    pub fn simulate_deduct(
        env: Env,
        caller: Address,
        amount: i128,
        request_id: u64,
    ) -> Result<i128, VaultError> {
        CalloraVault::validate_deduct(&env, &caller, amount)?;

        // Idempotency: `deduct` rejects a replayed non-zero request id with
        // `DuplicateRequestId`; `request_id == 0` means "no idempotency".
        if request_id != 0 {
            CalloraVault::require_not_duplicate(&env, &request_id)?;
        }

        // Projected new balance — same shape as `deduct`'s `Ok(..)` payload.
        let balance: i128 = env
            .storage()
            .instance()
            .get(&crate::DataKey::Balance)
            .unwrap_or(0);
        let new_balance = balance
            .checked_sub(amount)
            .ok_or(VaultError::Overflow)?;
        Ok(new_balance)
    }
}

