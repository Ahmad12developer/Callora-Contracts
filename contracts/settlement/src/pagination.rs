use crate::{
    CalloraSettlement, DeveloperBalance, StorageKey, INSTANCE_BUMP_AMOUNT,
    INSTANCE_BUMP_THRESHOLD, MAX_DEVELOPER_BALANCES_PAGE_SIZE, PERSISTENT_BUMP_AMOUNT,
    PERSISTENT_BUMP_THRESHOLD,
};
use soroban_sdk::{Address, Env, Vec};

/// Get a paginated page of developer balances using cursor-based pagination
/// over the **paged persistent index**.
///
/// # Pagination Behavior
/// Returns up to `limit` developer balance records starting **after** the supplied `cursor`
/// address (exclusive), or from the beginning of the index when `cursor` is `None`.
///
/// # Cursor Semantics
/// The returned `next_cursor` is the address of the last record returned on a full page.
/// Subsequent calls should pass this `next_cursor` as the `cursor` argument.
/// When the returned page has fewer elements than the requested limit (or is empty), the end
/// of the list has been reached, and `None` is returned as the next cursor.
///
/// # Ordering Guarantees
/// Within each index page addresses are stored in insertion order (append-only).
/// Ordering across pages is therefore also insertion order, which is stable for
/// sequential pagination. Interleaved credits for *new* developers appended after
/// the cursor will appear on later pages; they never shift existing pages.
///
/// # Page-size Configuration
/// The page size is capped at `MAX_DEVELOPER_BALANCES_PAGE_SIZE` (100) to limit gas usage
/// and prevent transaction size limits from being exceeded.
///
/// # Intended Use
/// This function is designed for batch reconciliation, indexing, and reporting dashboards
/// where developer balances must be safely and incrementally sync'd.
///
/// # State Mutation
/// This function is read-only for contract state logic but extends storage TTL for retrieved
/// entries to prevent archival.
pub fn get_page(
    env: &Env,
    cursor: Option<Address>,
    limit: u32,
    usdc_token: &Address,
) -> (Vec<DeveloperBalance>, Option<Address>) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_AMOUNT);

    let effective_limit = if limit == 0 {
        return (Vec::new(env), None);
    } else {
        limit.min(MAX_DEVELOPER_BALANCES_PAGE_SIZE)
    };

    let mut result = Vec::new(env);
    let mut past_cursor = cursor.is_none();
    let mut last_address: Option<Address> = None;

    CalloraSettlement::iter_index(env, |address| {
        if result.len() >= effective_limit {
            return;
        }

        if !past_cursor {
            if let Some(ref c) = cursor {
                if &address == c {
                    past_cursor = true;
                }
            }
            return;
        }

        let key = StorageKey::DeveloperBalance(address.clone(), usdc_token.clone());
        if env.storage().persistent().has(&key) {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_BUMP_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }

        let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0);

        result.push_back(DeveloperBalance {
            address: address.clone(),
            token: usdc_token.clone(),
            balance,
        });
        last_address = Some(address);
    });

    let next_cursor = if result.len() >= effective_limit {
        last_address
    } else {
        None
    };

    (result, next_cursor)
}
