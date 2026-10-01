use soroban_sdk::{contracttype, Address, Symbol};

/// Storage keys for the refund contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageKey {
    Admin,
    RefundConfig,
    PendingRefund(u64),
    RequesterRefunds(Address),
    TotalRefunds,
    RefundCounter,
}

/// Refund configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundConfig {
    pub fee_bps: u32,
    pub min_refund_amount: i128,
}

/// Status of a refund request.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum RefundStatus {
    Pending = 0,
    Approved = 1,
    Rejected = 2,
    Processed = 3,
}

/// Refund request data.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefundRequest {
    pub id: u64,
    pub requester: Address,
    pub token: Address,
    pub amount: i128,
    pub reason: Symbol,
    pub status: RefundStatus,
    pub created_at: u64,
    pub processed_at: Option<u64>,
}
