use soroban_sdk::{contracttype, Address, Symbol};

/// Minimum threshold of remaining ledgers before instance storage TTL is extended (~30 days).
pub const INSTANCE_BUMP_THRESHOLD: u32 = 17_280 * 30;

/// Number of ledgers to extend instance storage TTL by (~60 days).
pub const INSTANCE_BUMP_AMOUNT: u32 = 17_280 * 60;

/// Ledgers per day at the ~5 s target close time.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// Remaining-TTL threshold (~30 days) below which a persistent entry is
/// re-extended on access/write (#1131).
///
/// Kept strictly below [`PERSISTENT_BUMP_AMOUNT`]: with the previous
/// `threshold == amount == 50_000`, every single write paid for a TTL
/// extension. Now an entry is only re-extended once it has aged past
/// ~90 days of its ~120-day lifetime.
pub const PERSISTENT_BUMP_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;

/// Persistent-entry lifetime (~120 days) applied on every extension (#1131).
///
/// The old value, `50_000` ledgers, was ~2.9 days, not the "1 year" the
/// code comments claimed — developer balances, caps, claim windows and
/// replay high-water marks could archive after a long weekend of
/// inactivity. One year (~6.3M ledgers) is above the network's
/// `max_entry_ttl` (~180 days on pubnet), so extending that far would be
/// rejected; ~120 days stays safely under that cap while outliving any
/// realistic idle period. Every settlement persistent key family —
/// balances, HWM, caps, claim windows, minimum balances, pending
/// migrations, price-registry write ledgers — uses this pair.
pub const PERSISTENT_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 120;

/// Persistent storage keys for settlement contract.
///
/// # Migration note
/// Discriminant 5 was the original `DeveloperBalance(Address)` (single-token, now
/// `DeveloperBalanceV1` — kept for migration reads only).  New per-token entries use
/// `DeveloperBalance(Address, Address)` at discriminant 6.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StorageKey {
    Admin,
    HighWaterMark(Address),
    PoolHighWaterMark,
    Vault,
    PendingAdmin,
    PendingVault,
    /// Legacy flat developer index in instance storage — kept so the
    /// index-to-pages migration can read and drain it.  Do **not** write
    /// new entries here; new registrations go via [`StorageKey::IndexPage`].
    DeveloperIndex,
    /// Persistent per-developer membership flag (`bool`).
    ///
    /// Set to `true` the first time a developer is credited.  Used as an O(1)
    /// duplicate guard so `sorted_insert` no longer scans the whole index.
    DeveloperMember(Address),
    /// One page of the developer index stored in **persistent** storage.
    ///
    /// Key: zero-based page number.  Each page holds up to
    /// [`INDEX_PAGE_SIZE`] addresses in ascending order by address bytes.
    IndexPage(u32),
    /// Total number of allocated index pages (stored in instance storage as
    /// a single `u32`).  Replaces the unbounded `DeveloperIndex` vector.
    IndexPageCount,
    /// Legacy single-token balance — kept for V1 → V2 migration reads only.
    /// Do **not** use for new writes; new per-token credits go to
    /// [`StorageKey::DeveloperBalance`].
    DeveloperBalanceV1(Address),
    /// Per-token developer balance `(developer, token)`.
    DeveloperBalance(Address, Address),
    DeveloperMinBalance(Address),
    GlobalPool,
    Usdc,
    DailyWithdrawCap(Address),
    WithdrawalToday(Address),
    ContractVersion,
    /// Pending timelock'd developer balance migration record.
    /// Key: source developer address.
    PendingDeveloperMigration(Address),
    /// Storage-layout version marker (u32).
    /// Absent   → V1 (pre-migration, no version tracking).
    /// Value 2  → V2 (single-token → per-token migration complete).
    StorageVersion,
    /// Claim window configuration per developer.
    DeveloperClaimWindow(Address),
    /// Cumulative total of every amount ever credited via `receive_payment` /
    /// `batch_receive_payment`, regardless of routing (pool or developer).
    TotalReceived,
    /// Whether a specific developer's withdrawals are frozen.
    FrozenDeveloper(Address),
    /// Per-admin last write ledger for price registry rate limiting.
    PriceRegistryLastWrite(Address),
    /// Price entry for a given offering identifier.
    Price(soroban_sdk::String),
    /// Pending timelocked WASM upgrade proposal.
    PendingUpgrade,
    /// Whether a token contract is accepted for settlement payments.
    SupportedToken(Address),
    /// Whether the configured-USDC allowlist backfill has run.
    SupportedTokensMigrated,
    /// Persistent replay marker for an accounting-only vault deduction.
    DeductionRequest(u64),
}

/// Accounting-only deduction recorded; does not imply a token transfer.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeductionRecordedEvent {
    pub amount: i128,
    pub request_id: u64,
}

/// Read-only preview of a developer claim/withdrawal.
///
/// Returned by `simulate_claim` after running the same validation checks as
/// `withdraw_developer_balance`, without requiring auth, transferring tokens,
/// writing storage, or emitting events.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimSimulation {
    pub developer: Address,
    pub amount: i128,
    pub recipient: Address,
    pub token: Address,
    pub current_balance: i128,
    pub remaining_balance: i128,
    pub contract_balance: i128,
    pub daily_withdraw_cap: i128,
    pub withdrawn_today: i128,
    pub withdrawn_today_after: i128,
}

/// Severity levels for admin broadcast messages.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

/// Payload for the `admin_broadcast` event.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminBroadcast {
    pub severity: Severity,
    pub message: soroban_sdk::String,
}

/// Storage TTL policy entry for a given storage key category.
///
/// Retained for ABI compatibility with the off-chain tooling that consumes the
/// settlement contract's TTL views. Note that the `ttl` field is **not** a live
/// measurement: contract code cannot observe the remaining TTL of a ledger
/// entry. Read live TTLs over Soroban RPC `getLedgerEntries` and compare
/// `liveUntilLedgerSeq` against the current ledger sequence — see
/// `docs/STORAGE_TTL_DOCTOR.md`. The revenue pool exposes policy constants only
/// via `get_ttl_policy` (`callora_revenue_pool::TtlPolicy`).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StorageEntryTtl {
    pub category: soroban_sdk::String,
    pub key_desc: soroban_sdk::String,
    pub storage_type: soroban_sdk::String,
    pub ttl: u32,
    pub threshold: u32,
    pub bump_amount: u32,
}

/// Developer balance record in settlement contract.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeveloperBalance {
    pub address: Address,
    pub token: Address,
    pub balance: i128,
}

/// Timestamp range during which a developer may claim accrued balance.
///
/// `start_ts` and `end_ts` are ledger timestamps in seconds. The window is
/// inclusive on both ends: a withdrawal is allowed when
/// `start_ts <= env.ledger().timestamp() <= end_ts`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeveloperClaimWindow {
    pub start_ts: u64,
    pub end_ts: u64,
}

/// Emitted when the admin sets or clears a developer claim window.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeveloperClaimWindowChanged {
    pub developer: Address,
    pub start_ts: u64,
    pub end_ts: u64,
    pub enabled: bool,
}

/// Global pool balance tracking.
///
/// `last_updated` is set to `env.ledger().timestamp()` on every
/// `receive_payment` call that credits the pool (`to_pool = true`).
/// It is also set at `init` time. It is **not** updated when payments
/// are routed to individual developer balances.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalPool {
    pub total_balance: i128,
    /// Ledger timestamp of the last pool credit. Useful for analytics
    /// and staleness checks.
    pub last_updated: u64,
}

/// Tracks a developer's cumulative withdrawal amount for a given epoch day.
///
/// `day` is `timestamp / 86400` (UTC epoch day). When the current call's day
/// differs from the stored day the accumulator is silently reset.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DailyWithdrawState {
    pub day: u64,
    pub amount: i128,
}

/// Payment received event.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PaymentReceivedEvent {
    pub from_vault: Address,
    pub amount: i128,
    pub to_pool: bool,
    pub developer: Option<Address>,
    pub token: Address,
}

/// Balance credited event.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BalanceCreditedEvent {
    pub developer: Address,
    pub amount: i128,
    pub new_balance: i128,
    pub token: Address,
}

/// Emitted when a deposit is made for a developer.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DepositEvent {
    pub developer: Address,
    pub token: Address,
    pub amount: i128,
}

/// Emitted when a new vault address is proposed via `propose_vault()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VaultProposedEvent {
    pub current_vault: Address,
    pub proposed_vault: Address,
}

/// Emitted when the proposed vault is accepted via `accept_vault()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VaultAcceptedEvent {
    pub old_vault: Address,
    pub new_vault: Address,
    pub accepted_by: Address,
}

/// Emitted when a developer withdraws their balance.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeveloperWithdrawEvent {
    pub developer: Address,
    pub amount: i128,
    pub remaining_balance: i128,
    pub to: Address,
    pub token: Address,
}

/// Emitted when the admin sets or changes a developer's daily withdrawal cap.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DailyWithdrawCapChanged {
    pub developer: Address,
    pub new_cap: i128,
}

/// Emitted when an admin force-credits a developer balance (escape hatch).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DeveloperForceCreditedEvent {
    pub developer: Address,
    pub amount: i128,
    pub reason: Symbol,
    pub new_balance: i128,
    pub token: Address,
}

/// Emitted when the admin proposes or executes a timelock'd developer balance
/// migration (address rotation).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminMigrationEvent {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
    pub executed_at: u64,
}

/// Emitted when the admin proposes a timelocked WASM upgrade.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UpgradeProposedEvent {
    pub wasm_hash: soroban_sdk::BytesN<32>,
    pub proposed_at: u64,
    pub execute_after: u64,
}

/// Emitted when a pending WASM upgrade is cancelled by the admin.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct UpgradeCancelledEvent {
    pub wasm_hash: soroban_sdk::BytesN<32>,
    pub cancelled_at: u64,
}
