//! Batch limits for the Callora Distribute contract.
//!
//! [`Distribute::batch_distribute`](crate::Distribute::batch_distribute) rejects
//! batches larger than [`MAX_BATCH_SIZE`] before validating or transferring
//! any payment. [`Distribute::get_max_batch_size`](crate::Distribute::get_max_batch_size)
//! exposes the same limit to clients.
//!
//! Distributions transfer tokens immediately and do not create per-account
//! state entries or pending payouts. This module does not enforce a lifetime
//! payment count or a per-account storage cap.

/// Maximum number of payment legs allowed in a single batch operation.
pub const MAX_BATCH_SIZE: u32 = 50;

/// Backwards-compatible aliases for callers that used the limits module.
/// The canonical TTL policy is defined once at the crate root.
pub use crate::{
    INSTANCE_BUMP_AMOUNT as BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD as LIFETIME_THRESHOLD,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors::DistributeError, Distribute, DistributeClient};
    use soroban_sdk::testutils::{storage::Instance as _, Address as _, Events as _};
    use soroban_sdk::{token, Address, Env, Vec};

    fn setup() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let usdc = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let contract = env.register(Distribute, ());
        DistributeClient::new(&env, &contract).init(&admin, &usdc);
        token::StellarAssetClient::new(&env, &usdc).mint(&contract, &1_000);
        (env, admin, recipient, usdc, contract)
    }

    fn payments(env: &Env, recipient: &Address, count: u32) -> Vec<(Address, i128)> {
        let mut payments = Vec::new(env);
        for _ in 0..count {
            payments.push_back((recipient.clone(), 1));
        }
        payments
    }

    #[test]
    fn batch_limits_accept_below_and_at_maximum() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let token = token::Client::new(&env, &usdc);
        assert_eq!(client.get_max_batch_size(), MAX_BATCH_SIZE);

        for count in [MAX_BATCH_SIZE - 1, MAX_BATCH_SIZE] {
            let before = token.balance(&recipient);
            client.batch_distribute(&admin, &payments(&env, &recipient, count));
            assert_eq!(token.balance(&recipient), before + i128::from(count));
        }
        assert_eq!(client.balance(), 1_000 - i128::from(2 * MAX_BATCH_SIZE - 1));
    }

    #[test]
    fn batch_limits_reject_empty_and_oversized_without_transfers() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let token = token::Client::new(&env, &usdc);

        for (count, error) in [
            (0, DistributeError::BatchEmpty),
            (MAX_BATCH_SIZE + 1, DistributeError::BatchTooLarge),
        ] {
            assert_eq!(
                client.try_batch_distribute(&admin, &payments(&env, &recipient, count)),
                Err(Ok(soroban_sdk::Error::from_contract_error(error as u32)))
            );
            // Inspect the rejected invocation before balance queries replace
            // the SDK's per-invocation event buffer.
            assert!(env.events().all().is_empty());
            assert_eq!(client.balance(), 1_000);
            assert_eq!(token.balance(&recipient), 0);
        }

        // Rejection must not prevent a subsequent valid batch.
        client.batch_distribute(&admin, &payments(&env, &recipient, MAX_BATCH_SIZE));
        assert_eq!(token.balance(&recipient), i128::from(MAX_BATCH_SIZE));
    }

    #[test]
    fn batch_limits_apply_per_call_without_accumulating_account_state() {
        let (env, admin, recipient, usdc, contract) = setup();
        let client = DistributeClient::new(&env, &contract);
        let batch = payments(&env, &recipient, MAX_BATCH_SIZE);
        let state_before = env.as_contract(&contract, || env.storage().instance().all());

        // Exceed the removed default of 100 payments to the same account.
        // Completed payments are not active state and must not consume a cap.
        for _ in 0..3 {
            client.batch_distribute(&admin, &batch);
        }
        client.distribute(&admin, &recipient, &1);

        let total = i128::from(3 * MAX_BATCH_SIZE) + 1;
        assert_eq!(token::Client::new(&env, &usdc).balance(&recipient), total);
        assert_eq!(client.balance(), 1_000 - total);
        env.as_contract(&contract, || {
            assert_eq!(env.storage().instance().all(), state_before);
        });
    }
}
