extern crate std;

use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Symbol, Vec};

use crate::{
    capabilities::{
        ALL_CAPABILITIES, CAP_ADMIN_BROADCAST, CAP_AUTHORIZED_CALLER, CAP_BATCH_DEDUCT,
        CAP_DEPOSITOR_ALLOWLIST, CAP_DEDUCT, CAP_DEPOSIT, CAP_OFFERING_METADATA, CAP_PAUSE,
        CAP_PRICE_REGISTRY, CAP_RATE_LIMIT, CAP_REQUEST_IDEMPOTENCY, CAP_REVENUE_POOL,
        CAP_SETTLEMENT, CAP_SLIPPAGE_GUARD, CAP_TWO_STEP_ADMIN, CAP_TWO_STEP_OWNERSHIP,
        CAP_UPGRADE, CAP_WITHDRAW,
    },
    CalloraVault, CalloraVaultClient,
};

/// Exact expected capability mask: bits 0..5, 8..11, 15, 17.
/// (Bits 6, 7, 12, 13, 14, 16 and 18..63 are reserved/cleared).
const EXPECTED_EXACT_MASK: u64 = 0x0000_0000_0002_8F3F;

fn create_usdc(env: &Env, admin: &Address) -> Address {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    ca.address()
}

fn setup(env: &Env) -> (CalloraVaultClient<'_>, Address) {
    let owner = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);
    let usdc = create_usdc(env, &owner);
    env.mock_all_auths();
    client.init(
        &owner,
        &usdc,
        &Some(0),
        &Some(owner.clone()),
        &Some(1),
        &None,
        &Some(10_000),
        &None,
    );
    (client, owner)
}

// ---------------------------------------------------------------------------
// Basic return value & exact mask assertions
// ---------------------------------------------------------------------------

#[test]
fn capabilities_returns_nonzero() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities(), 0);
}

#[test]
fn capabilities_equals_all_capabilities_constant() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities(), ALL_CAPABILITIES);
}

#[test]
fn capabilities_equals_exact_expected_mask() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities(), EXPECTED_EXACT_MASK);
    assert_eq!(ALL_CAPABILITIES, EXPECTED_EXACT_MASK);
}

// ---------------------------------------------------------------------------
// Each supported individual capability bit is set
// ---------------------------------------------------------------------------

#[test]
fn cap_deposit_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_DEPOSIT, 0);
}

#[test]
fn cap_withdraw_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_WITHDRAW, 0);
}

#[test]
fn cap_deduct_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_DEDUCT, 0);
}

#[test]
fn cap_batch_deduct_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_BATCH_DEDUCT, 0);
}

#[test]
fn cap_pause_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_PAUSE, 0);
}

#[test]
fn cap_authorized_caller_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_AUTHORIZED_CALLER, 0);
}

#[test]
fn cap_request_idempotency_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_REQUEST_IDEMPOTENCY, 0);
}

#[test]
fn cap_two_step_ownership_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_TWO_STEP_OWNERSHIP, 0);
}

#[test]
fn cap_two_step_admin_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_TWO_STEP_ADMIN, 0);
}

#[test]
fn cap_settlement_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_SETTLEMENT, 0);
}

#[test]
fn cap_depositor_allowlist_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_DEPOSITOR_ALLOWLIST, 0);
}

#[test]
fn cap_upgrade_is_set() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_ne!(client.capabilities() & CAP_UPGRADE, 0);
}

// ---------------------------------------------------------------------------
// Unimplemented feature bits are cleared (0) and reserved
// ---------------------------------------------------------------------------

#[test]
fn cap_offering_metadata_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_OFFERING_METADATA, 0);
}

#[test]
fn cap_price_registry_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_PRICE_REGISTRY, 0);
}

#[test]
fn cap_revenue_pool_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_REVENUE_POOL, 0);
}

#[test]
fn cap_rate_limit_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_RATE_LIMIT, 0);
}

#[test]
fn cap_admin_broadcast_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_ADMIN_BROADCAST, 0);
}

#[test]
fn cap_slippage_guard_is_cleared() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities() & CAP_SLIPPAGE_GUARD, 0);
}

// ---------------------------------------------------------------------------
// Reserved bits are zero
// ---------------------------------------------------------------------------

#[test]
fn reserved_bits_are_zero() {
    let env = Env::default();
    let (client, _) = setup(&env);
    let reserved_mask: u64 = !ALL_CAPABILITIES;
    assert_eq!(client.capabilities() & reserved_mask, 0);

    // Specifically verify all bits 18–63 are zero.
    let upper_reserved_mask: u64 = !((1u64 << 18) - 1);
    assert_eq!(client.capabilities() & upper_reserved_mask, 0);
}

// ---------------------------------------------------------------------------
// Stability: bit positions match their constant values exactly
// ---------------------------------------------------------------------------

#[test]
fn bit_positions_are_stable() {
    assert_eq!(CAP_DEPOSIT, 0x0000_0000_0000_0001);
    assert_eq!(CAP_WITHDRAW, 0x0000_0000_0000_0002);
    assert_eq!(CAP_DEDUCT, 0x0000_0000_0000_0004);
    assert_eq!(CAP_BATCH_DEDUCT, 0x0000_0000_0000_0008);
    assert_eq!(CAP_PAUSE, 0x0000_0000_0000_0010);
    assert_eq!(CAP_AUTHORIZED_CALLER, 0x0000_0000_0000_0020);
    assert_eq!(CAP_OFFERING_METADATA, 0x0000_0000_0000_0040);
    assert_eq!(CAP_PRICE_REGISTRY, 0x0000_0000_0000_0080);
    assert_eq!(CAP_REQUEST_IDEMPOTENCY, 0x0000_0000_0000_0100);
    assert_eq!(CAP_TWO_STEP_OWNERSHIP, 0x0000_0000_0000_0200);
    assert_eq!(CAP_TWO_STEP_ADMIN, 0x0000_0000_0000_0400);
    assert_eq!(CAP_SETTLEMENT, 0x0000_0000_0000_0800);
    assert_eq!(CAP_REVENUE_POOL, 0x0000_0000_0000_1000);
    assert_eq!(CAP_RATE_LIMIT, 0x0000_0000_0000_2000);
    assert_eq!(CAP_ADMIN_BROADCAST, 0x0000_0000_0000_4000);
    assert_eq!(CAP_DEPOSITOR_ALLOWLIST, 0x0000_0000_0000_8000);
    assert_eq!(CAP_SLIPPAGE_GUARD, 0x0000_0000_0001_0000);
    assert_eq!(CAP_UPGRADE, 0x0000_0000_0002_0000);
}

// ---------------------------------------------------------------------------
// Edge cases & mask decomposition
// ---------------------------------------------------------------------------

#[test]
fn capabilities_is_idempotent() {
    let env = Env::default();
    let (client, _) = setup(&env);
    assert_eq!(client.capabilities(), client.capabilities());
}

#[test]
fn capabilities_available_before_init() {
    // capabilities() is a pure constant view — it does not require the vault to be
    // initialized.
    let env = Env::default();
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(&env, &vault_addr);
    assert_eq!(client.capabilities(), ALL_CAPABILITIES);
}

#[test]
fn all_reserved_bits_are_cleared_in_all_capabilities() {
    let reserved_bits: &[u64] = &[
        CAP_OFFERING_METADATA,
        CAP_PRICE_REGISTRY,
        CAP_REVENUE_POOL,
        CAP_RATE_LIMIT,
        CAP_ADMIN_BROADCAST,
        CAP_SLIPPAGE_GUARD,
    ];
    for &bit in reserved_bits {
        assert_eq!(
            ALL_CAPABILITIES & bit,
            0,
            "reserved bit {bit:#x} must be cleared in ALL_CAPABILITIES"
        );
    }
}

#[test]
fn all_supported_bits_match_all_capabilities_decomposition() {
    let supported_bits: &[u64] = &[
        CAP_DEPOSIT,
        CAP_WITHDRAW,
        CAP_DEDUCT,
        CAP_BATCH_DEDUCT,
        CAP_PAUSE,
        CAP_AUTHORIZED_CALLER,
        CAP_REQUEST_IDEMPOTENCY,
        CAP_TWO_STEP_OWNERSHIP,
        CAP_TWO_STEP_ADMIN,
        CAP_SETTLEMENT,
        CAP_DEPOSITOR_ALLOWLIST,
        CAP_UPGRADE,
    ];
    let mut expected_mask = 0u64;
    for &bit in supported_bits {
        assert_ne!(
            ALL_CAPABILITIES & bit,
            0,
            "supported bit {bit:#x} must be set in ALL_CAPABILITIES"
        );
        expected_mask |= bit;
    }
    assert_eq!(ALL_CAPABILITIES, expected_mask);
}

#[test]
fn all_capabilities_bits_are_power_of_two_distinct() {
    // Verify no two CAP_* constants share a bit position.
    let individual: &[u64] = &[
        CAP_DEPOSIT,
        CAP_WITHDRAW,
        CAP_DEDUCT,
        CAP_BATCH_DEDUCT,
        CAP_PAUSE,
        CAP_AUTHORIZED_CALLER,
        CAP_OFFERING_METADATA,
        CAP_PRICE_REGISTRY,
        CAP_REQUEST_IDEMPOTENCY,
        CAP_TWO_STEP_OWNERSHIP,
        CAP_TWO_STEP_ADMIN,
        CAP_SETTLEMENT,
        CAP_REVENUE_POOL,
        CAP_RATE_LIMIT,
        CAP_ADMIN_BROADCAST,
        CAP_DEPOSITOR_ALLOWLIST,
        CAP_SLIPPAGE_GUARD,
        CAP_UPGRADE,
    ];
    let mut seen: u64 = 0;
    for &cap in individual {
        assert_eq!(cap & (cap - 1), 0, "CAP constant {cap:#x} is not a power of two");
        assert_eq!(seen & cap, 0, "CAP constant {cap:#x} overlaps with a previously seen bit");
        seen |= cap;
    }
}

// ---------------------------------------------------------------------------
// Mapping: every set capability bit corresponds to a callable client entrypoint
// ---------------------------------------------------------------------------

#[test]
fn every_set_capability_maps_to_callable_vault_entrypoint() {
    let env = Env::default();
    let (client, owner) = setup(&env);
    let caps = client.capabilities();

    // 1. CAP_DEPOSIT (bit 0) -> deposit
    assert_ne!(caps & CAP_DEPOSIT, 0);
    let _ = client.try_deposit(&owner, &100);

    // 2. CAP_WITHDRAW (bit 1) -> withdraw, withdraw_to
    assert_ne!(caps & CAP_WITHDRAW, 0);
    let _ = client.try_withdraw(&0);
    let _ = client.try_withdraw_to(&owner, &0);

    // 3. CAP_DEDUCT (bit 2) -> deduct, simulate_deduct
    assert_ne!(caps & CAP_DEDUCT, 0);
    let _ = client.try_deduct(&owner, &0, &1);

    // 4. CAP_BATCH_DEDUCT (bit 3) -> batch_deduct, simulate_batch_deduct
    assert_ne!(caps & CAP_BATCH_DEDUCT, 0);
    let _ = client.try_batch_deduct(&owner, &Vec::new(&env));

    // 5. CAP_PAUSE (bit 4) -> pause, unpause, is_paused
    assert_ne!(caps & CAP_PAUSE, 0);
    assert!(!client.is_paused());
    client.pause(&owner);
    assert!(client.is_paused());
    client.unpause(&owner);
    assert!(!client.is_paused());

    // 6. CAP_AUTHORIZED_CALLER (bit 5) -> set_authorized_caller
    assert_ne!(caps & CAP_AUTHORIZED_CALLER, 0);
    let caller = Address::generate(&env);
    client.set_authorized_caller(&Some(caller.clone()), &0);

    // 7. CAP_REQUEST_IDEMPOTENCY (bit 8) -> is_request_processed, prune_processed_requests
    assert_ne!(caps & CAP_REQUEST_IDEMPOTENCY, 0);
    let req_id = Symbol::new(&env, "req_1");
    assert!(!client.is_request_processed(&req_id));
    let _ = client.prune_processed_requests(&owner, &Vec::new(&env));

    // 8. CAP_TWO_STEP_OWNERSHIP (bit 9) -> transfer_ownership, accept_ownership, get_owner
    assert_ne!(caps & CAP_TWO_STEP_OWNERSHIP, 0);
    assert_eq!(client.get_owner(), owner);
    let new_owner = Address::generate(&env);
    client.transfer_ownership(&owner, &new_owner);
    client.accept_ownership();
    assert_eq!(client.get_owner(), new_owner);

    // 9. CAP_TWO_STEP_ADMIN (bit 10) -> set_admin, accept_admin, get_admin
    // The vault was initialized with `owner` as admin; after ownership transfer
    // `owner` is no longer the owner but is still the admin.  set_admin
    // requires the *current admin* as caller.
    assert_ne!(caps & CAP_TWO_STEP_ADMIN, 0);
    let new_admin = Address::generate(&env);
    client.set_admin(&owner, &new_admin); // owner == current admin
    client.accept_admin();
    assert_eq!(client.get_admin(), new_admin);

    // 10. CAP_SETTLEMENT (bit 11) -> set_settlement, get_settlement
    assert_ne!(caps & CAP_SETTLEMENT, 0);
    let settlement = Address::generate(&env);
    client.set_settlement(&new_owner, &settlement);
    assert_eq!(client.get_settlement(), settlement);

    // 11. CAP_DEPOSITOR_ALLOWLIST (bit 15) -> add_address, clear_all, get_allowlist, is_authorized_depositor
    // NOTE: add_address / get_allowlist / clear_all store a Vec<Address> under
    // StorageKey::AllowedDepositors, whereas is_authorized_depositor reads a
    // per-address bool under DataKey::Depositor — they use different storage
    // keys and do not interact with each other.  We verify each entrypoint is
    // callable and produces the correct observable effect for its own storage.
    assert_ne!(caps & CAP_DEPOSITOR_ALLOWLIST, 0);
    let depositor = Address::generate(&env);
    // is_authorized_depositor: callable entrypoint — returns false for unknown addr.
    assert!(!client.is_authorized_depositor(&depositor));
    // add_address: entrypoint callable — depositor appears in get_allowlist().
    client.add_address(&new_owner, &depositor);
    let list = client.get_allowlist();
    assert!(list.contains(depositor.clone()), "depositor must be in allowlist after add_address");
    // clear_all: entrypoint callable — allowlist is empty afterward.
    client.clear_all(&new_owner);
    let list_after = client.get_allowlist();
    assert!(list_after.is_empty(), "allowlist must be empty after clear_all");

    // 12. CAP_UPGRADE (bit 17) -> propose_upgrade, execute_upgrade, cancel_upgrade, get_pending_upgrade
    assert_ne!(caps & CAP_UPGRADE, 0);
    assert_eq!(client.get_pending_upgrade(), None);
    let wasm_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.propose_upgrade(&new_admin, &wasm_hash);
    assert!(client.get_pending_upgrade().is_some());
    client.cancel_upgrade(&new_admin);
    assert_eq!(client.get_pending_upgrade(), None);
}
