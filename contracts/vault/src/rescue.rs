//! Admin rescue helpers for recovering tokens accidentally sent to the vault.
//
// The public contract entrypoint lives in `lib.rs` so it is included in the
// generated Soroban client; this module keeps the balance and transfer checks
// focused and testable.

use soroban_sdk::{token, Address, Env};

use crate::VaultError;

/// Transfer `amount` of `token_address` from the current vault contract to `to`.
//
// When `protected_balance` is `Some`, that amount is reserved and cannot be
// rescued. The vault passes the tracked USDC balance here so admin rescue can
// recover only USDC surplus above internal accounting, while still allowing any
// non-USDC token accidentally sent to the contract to be fully rescued.
//
// # Invariants

// - The contract's on-ledger balance of `token_address` must always remain
//   greater than or equal to `protected_balance` after a successful rescue.
// - When `protected_balance` is `None`, the entire on-ledger balance is
//   available and the tracked balance is ignored.
//
// # Errors

// - `AmountNotPositive` if `amount <= 0`.
// - `InsufficientBalance` if the available amount is less than `amount`,
//   including the case where the on-ledger balance is below the protected
//   balance (over-reserved).
pub fn rescue_funds(
    env: &Env,
    token_address: &Address,
    to: &Address,
    amount: i128,
    protected_balance: Option<i128>,
) -> Result<(), VaultError> {
    if amount <= 0 {
        return Err(VaultError::AmountNotPositive);
    }

    let token_client = token::Client::new(env, token_address);
    let vault = env.current_contract_address();
    let on_ledger_balance = token_client.balance(&vault);
    let available = match protected_balance {
        Some(protected) => on_ledger_balance
            .checked_sub(protected)
            .ok_tor(VaultError::InsufficientBalance)?,
        None => on_ledger_balance,
    };

    if available < amount {
        return Err(VaultError::InsufficientBalance);
    }

    token_client.transfer(&vault, to, &amount);
    Ok(())
}
