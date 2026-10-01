# Refund Contract Storage Documentation

This document outlines the storage keys used by the Refund contract, detailing their Soroban storage tiers and the rationale behind their selection.

## Storage Keys Hierarchy

### `DataKey::Admin`
- **Tier:** **Instance**
- **Rationale:** The administrator address dictates access control for the entire contract. Because it is globally applicable and must survive as long as the contract is active, the `Instance` tier is used. This ensures the admin data shares the same Time-To-Live (TTL) as the contract instance itself, preventing access lockouts.

### `StorageKey::PendingRefund(u64)`
- **Tier:** **Persistent**
- **Rationale:** Stores one refund request by sequential ID. Each request receives its own TTL bump when created, read, or processed.

### `StorageKey::RequesterRefunds(Address)`
- **Tier:** **Persistent**
- **Data Type:** `Vec<u64>`
- **Rationale:** Stores request IDs in creation order so clients can paginate a requester's history without scanning the global counter. The vector is capped at `MAX_REQUESTER_REFUNDS` (100); when full, the oldest index entry is evicted while the underlying request remains addressable by ID. Reads and writes bump the index TTL.

Use `get_refunds_by_requester(requester, start, limit)` for pages. `limit` is capped at 50, and `start` is a zero-based offset into the retained index.

## Storage Tiers Overview

- **Instance Storage**: Stored in the contract instance ledger entry. Automatically extended when the contract instance is invoked. Ideal for small, frequently accessed configuration data and global contract state.
- **Persistent Storage**: Stored as independent ledger entries with their own Time-To-Live (TTL). Ideal for user-specific or data-heavy entries that require explicit TTL management.
- **Temporary Storage**: Short-lived entries intended for transient data (e.g., rate-limit counters, short-lived scratchpads).
