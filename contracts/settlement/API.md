# Settlement API Notes

## `simulate_claim(developer, amount, to)`

`simulate_claim` is a read-only view that previews the result of a developer
claim through `withdraw_developer_balance` without side effects. It returns a
`ClaimSimulation` record containing the simulated recipient, configured USDC
token, current developer balance, remaining balance, contract token balance,
daily cap, current same-day withdrawal amount, and same-day withdrawal amount
after the simulated claim.

The view runs the exact shared claim validation used by a real claim, in the
same order, and returns the same typed errors: `DeveloperFrozen` when the
developer's withdrawals are frozen, `AmountNotPositive` for a non-positive
amount, `ClaimWindowClosed` when outside a configured claim window,
`UsdcTokenNotConfigured` when no USDC token is set,
`InsufficientDeveloperBalance` when the tracked balance cannot cover the
amount, `MinBalanceViolation` when the withdrawal would leave the developer
below their configured minimum balance, `DailyWithdrawCapExceeded` when the
daily cap would be exceeded, `DeveloperBalanceUnderflow` on arithmetic
underflow, and `InsufficientContractBalance` when the contract lacks token
liquidity. As with a real claim, simulating a claim whose recipient is the
settlement contract itself panics with `InvalidRecipient`.

The view does not require developer authorization, transfer tokens, mutate
storage, extend TTLs, or emit events — validation itself is read-only, so
calling `simulate_claim` never changes the TTL of the instance, the claim
window, the minimum balance, the developer balance, or any other entry.
