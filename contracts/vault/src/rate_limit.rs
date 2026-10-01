use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitConfig {
    pub capacity: i128,
    pub refill_rate: i128, // Refill amount per ledger
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitState {
    pub tokens: i128,
    pub last_updated_ledger: u32,
}

// TTL configuration for rate limit state
pub const RATE_LIMIT_BUMP_AMOUNT: u32 = 17_280 * 30; // ~30 days
pub const RATE_LIMIT_BUMP_THRESHOLD: u32 = 17_280 * 7; // ~7 days

/// Set the rate limit config for a specific developer.
pub fn set_config(env: &Env, developer: &Address, config: &RateLimitConfig) {
    env.storage().instance().set(
        &crate::StorageKey::DeveloperConfig(developer.clone()),
        config,
    );
}

/// Get the rate limit config for a specific developer.
pub fn get_config(env: &Env, developer: &Address) -> Option<RateLimitConfig> {
    env.storage()
        .instance()
        .get(&crate::StorageKey::DeveloperConfig(developer.clone()))
}

/// Get the current rate limit state for a developer.
pub fn get_state(env: &Env, developer: &Address) -> Option<RateLimitState> {
    env.storage()
        .persistent()
        .get(&crate::StorageKey::DeveloperState(developer.clone()))
}

/// Internal helper: compute the post-refill snapshot of a developer's token
/// bucket without writing anything to storage.
///
/// Returns `None` when no rate limit is configured for `developer`, in which
/// case the caller should treat the developer as unconstrained (`Ok(())`).
fn refill_state(env: &Env, developer: &Address) -> Option<(RateLimitConfig, RateLimitState)> {
    let config = get_config(env, developer)?;
    let current_ledger = env.ledger().sequence();

    let mut state = get_state(env, developer).unwrap_or(RateLimitState {
        tokens: config.capacity,
        last_updated_ledger: current_ledger,
    });

    if current_ledger > state.last_updated_ledger {
        let elapsed = (current_ledger - state.last_updated_ledger) as i128;
        if let Some(refilled) = elapsed.checked_mul(config.refill_rate) {
            state.tokens = state.tokens.saturating_add(refilled);
            if state.tokens > config.capacity {
                state.tokens = config.capacity;
            }
        }
        state.last_updated_ledger = current_ledger;
    }

    Some((config, state))
}

/// Read-only dry-run of [`consume_tokens`].
///
/// Performs the exact same arithmetic — refill, threshold check, and bucket
/// read — but does NOT persist state. Used by [`crate::CalloraVault::simulate_deduct`]
/// to predict the outcome of a real deduct without mutating the rate-limit
/// bucket.
///
/// # Errors
/// - `VaultError::RateLimited` if the post-refill `tokens` is below `amount`.
/// - Returns `Ok(())` when no rate limit is configured for the developer (the
///   developer's deducts are unconstrained).
pub fn would_consume_tokens(
    env: &Env,
    developer: &Address,
    amount: i128,
) -> Result<(), crate::VaultError> {
    let (_config, state) = match refill_state(env, developer) {
        Some(v) => v,
        None => return Ok(()),
    };
    if state.tokens < amount {
        return Err(crate::VaultError::RateLimited);
    }
    Ok(())
}

/// Consume tokens from the developer's token bucket.
/// Applies the amortized refill based on elapsed ledgers before checking the limit.
pub fn consume_tokens(
    env: &Env,
    developer: &Address,
    amount: i128,
) -> Result<(), crate::VaultError> {
    let (_config, mut state) = match refill_state(env, developer) {
        Some(v) => v,
        None => return Ok(()), // No rate limit configured
    };
    if state.tokens < amount {
        return Err(crate::VaultError::RateLimited);
    }
    state.tokens = state
        .tokens
        .checked_sub(amount)
        .ok_or(crate::VaultError::Overflow)?;

    let state_key = crate::StorageKey::DeveloperState(developer.clone());
    env.storage().persistent().set(&state_key, &state);
    env.storage().persistent().extend_ttl(
        &state_key,
        RATE_LIMIT_BUMP_THRESHOLD,
        RATE_LIMIT_BUMP_AMOUNT,
    );

    Ok(())
}
