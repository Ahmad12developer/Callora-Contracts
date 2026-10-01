# Revenue Pool Admin Rotation Procedures
 
This document outlines the operational procedures for rotating the admin address of the Callora Revenue Pool contract.
 
## Overview
 
The Revenue Pool implements a **two-step admin transfer** process to maximize security and prevent accidental loss of administrative control. A transfer must be initiated by the current admin and then accepted by the nominated successor.
 
## Roles
 
- **Current Admin**: The address currently holding administrative privileges.
- **Pending Admin**: The address nominated by the current admin to take over.
- **Pause Guardian**: Optional emergency address that may call `pause` without receiving full admin privileges.
 
## Rotation Process
 
### Step 1: Nomination
 
The current admin initiates the transfer by calling `set_admin` with the address of the proposed successor.
 
```rust
pool.set_admin(current_admin_address, proposed_new_admin_address);
```
 
- **Action**: Sets the `PENDING_ADMIN` storage key.
- **Auth**: Requires signature from `current_admin`.
- **Event**: Emits `admin_transfer_started(current_admin, pending_admin)` and nothing else — no `admin_changed` event is published until Step 2 completes.
- **State**: The current admin retains all privileges until Step 2 is completed.
 
### Step 2: Acceptance
 
The nominated successor must explicitly claim the admin role by calling `claim_admin`.
 
```rust
pool.claim_admin(proposed_new_admin_address);
```
 
- **Action**: Updates `ADMIN` to the caller's address and clears `PENDING_ADMIN`.
- **Auth**: Requires signature from the `proposed_new_admin`.
- **Event**: Emits `admin_changed(previous_admin, new_admin)` recording the actual change, then `admin_transfer_completed(new_admin)`.
- **State**: Administrative control is fully transferred to the new address.
 
## Security Considerations
 
- **No Accidental Lockouts**: If an incorrect address is nominated in Step 1, the transfer will never be completed because only the nominated address can claim the role. The current admin can overwrite the nomination at any time by calling `set_admin` again with a different address.
- **Audit Trails**: Both steps of the rotation are explicitly logged via contract events, providing a clear audit trail for operations and indexers.
- **Immediate Effect**: Once `claim_admin` succeeds, the old admin immediately loses all administrative privileges.
 
## Emergency Procedures

The admin can delegate emergency pause authority without granting full admin power:

```rust
pool.set_pause_guardian(current_admin_address, guardian_address);
```

- **Action**: Sets the `pause_guardian` storage key.
- **Auth**: Requires signature from the current admin.
- **Event**: Emits `pause_guardian_set(current_admin)` with the guardian address as data.
- **Scope**: The guardian can call `pause` only. It cannot call `unpause`, distribute funds, rotate admin, change caps, clear or replace the guardian, or upgrade the contract.

To remove the emergency role:

```rust
pool.clear_pause_guardian(current_admin_address);
```

- **Action**: Removes the `pause_guardian` storage key.
- **Auth**: Requires signature from the current admin.
- **Event**: Emits `pause_guardian_cleared(current_admin)` with the previous guardian address as data.

If the current admin keys are lost before a transfer is initiated, the contract administrative functions will be permanently locked. It is recommended to use a multi-signature wallet or a hardware security module (HSM) for the admin role in production environments.
