use crate::{CalloraSettlement, MAX_BATCH_SIZE, SettlementError};
use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SettleInput {
    pub developer: Address,
    pub amount: i128,
    pub to: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum SettleOutcome {
    Success,
    AmountNotPositive,
    ClaimWindowClosed,
    InsufficientBalance,
    DailyWithdrawCapExceeded,
    DeveloperBalanceUnderflow,
    OtherError,
}

impl From<SettlementError> for SettleOutcome {
    fn from(err: SettlementError) -> Self {
        match err {
            SettlementError::AmountNotPositive => SettleOutcome::AmountNotPositive,
            SettlementError::ClaimWindowClosed => SettleOutcome::ClaimWindowClosed,
            SettlementError::InsufficientDeveloperBalance => SettleOutcome::InsufficientBalance,
            SettlementError::DailyWithdrawCapExceeded => SettleOutcome::DailyWithdrawCapExceeded,
            SettlementError::DeveloperBalanceUnderflow => SettleOutcome::DeveloperBalanceUnderflow,
            _ => SettleOutcome::OtherError,
        }
    }
}

/// Errors returned by [`batch_settle`] when the whole batch is
/// rejected before any per-item processing occurs.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum SettlementError {
    BatchEmpty,
    BatchTooLarge,
    CrossTenantBatch,
}

pub fn batch_settle(
    env: &Env,
    settlements: Vec<SettleInput>,
) -> Result<Vec<SettleOutcome>, SettlementError> {
    let mut outcomes = Vec::new(env);

    if settlements.is_empty() {
        return Err((SettlementError::BatchEmpty));
    }

    if settlements.len() > MAX_BATCH_SIZE {
        return Err((SettlementError::BatchTooLarge));
    }

    // Cross-tenant validation before mutation: every item must belong to the
    // same claimant. Per-item authorization is enforced inside
    // `withdraw_developer_balance` (each item requires the claimant's auth), so
    // no extra top-level `require_auth` is needed here.
    let claimant = settlements.get_unchecked(0).developer.clone();

    for input in settlements.iter() {
        if input.developer != claimant {
            return Err((SettlementError::CrossTenantBatch));
        }
    }

    for input in settlements.iter() {
        // We use CalloraSettlement::withdraw_developer_balance internally
        // but we must catch the error to allow partial success.
        let res = CalloraSettlement::withdraw_developer_balance(
            env.clone(),
            input.developer.clone(),
            input.amount,
            input.to.clone(),
        );
        match res {
            Ok(_) => outcomes.push_back(SettleOutcome::Success),
            Err(e) => outcomes.push_back(e.into()),
        }
    }

    Ok(outcomes)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::MAX_BATCH_SIZE;
    use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

    fn make_input(env: &Env, dev: Address) -> SettleInput {
        SettleInput {
            developer: dev,
            amount: 100,
            to: None,
        }
    }

    #[test]
    fn test_batch_settle_empty_returns_error() {
        let env = Env::default();
        let settlements = Vec::new(&env);

        let result = batch_settle(&env, settlements);
        assert!(result.is_err());
        assert_eq!(result.unwrap_error(), SettlementError::BatchEmpty);
    }

    #[test]
    fn test_batch_settle_cap_enforced() {
        let env = Env::default();
        let mut settlements = Vec::new(&env);

        // Push MAX_BATCH_SIZE + 1 items (exceeding the contract-wide cap)
        for _ in 0..(MAX_BATCH_SIZE + 1) {
            settlements.push_back(make_input(&env, Address::generate(&env)));
        }

        let result = batch_settle(&env, settlements);
        assert!(result.is_err());
        assert_eq!(result.unwrap_error(), SettlementError::BatchTooLarge);
    }

    #[test]
    fn test_batch_settle_cross_tenant_error() {
        let env = Env::default();
        let mut settlements = Vec::new(&env);

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        settlements.push_back(make_input(&env, alice));
        settlements.push_back(make_input(&env, bob));

        let result = batch_settle(&env, settlements);
        assert!(result.is_err());
        assert_eq!(result.unwrap_error(), SettlementError::CrossTenantBatch);
    }
}
