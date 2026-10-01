# Auditor Onboarding Bundle

This document provides a comprehensive guide for security auditors reviewing the Callora Contracts codebase. It covers build instructions, testing procedures, coverage verification, WASM output validation, and security documentation references.

## Quick Start

```bash
# Clone and navigate to the project
git clone <repository-url>
cd Callora-Contracts

# Build all contracts
cargo build

# Run all tests
cargo test

# Generate coverage report (requires cargo-tarpaulin)
//scripts/coverage.sh

# Build WASM binaries for deployment
cargo build --target wasm32-unknown-unknown --release -p callora-vault
cargo build --target wasm32-unknown-unknown --release -p callora-settlement
cargo build --target wasm32-unknown-unknown --release -p callora-revenue-pool

# Verify WASM size constraints
./scripts/check-wasm-size.sh
```

## Project Overview

Callora Contracts is a Soroban (Stellar) smart contract suite for an API marketplace with prepaid vault functionality and revenue settlement. The system consists of three main contracts:

- **`callora-vault`**: Prepaid balance management with deposit/deduct operations
- **`callora-settlement`**: Revenue distribution and payment processing  
- **`callora-revenue-pool`**: Revenue pool management and distribution

## Prerequisites

### Required Tools

1. **Rust** (stable toolchain)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Soroban CLI*** (for deployment testing)
   ```bash
   cargo install soroban-cli
   ```

3. **WASM target** (for release builds)
   ```bash
   rustup target add wasm32-unknown-unknown
   ```

4. **cargo-tarpaulin** (for coverage reports)
   ```bash
   cargo install cargo-tarpaulin
   ```

### System Requirements

- Operating System: Linux, macOS, or Windows
- Rust version: 1.70+ (stable)
- Memory: 4GB+ recommended for WASM builds

## Build Instructions

### Development Build

```bash
# Build all contracts in development mode
cargo build

# Build specific contract
cargo build -p callora-vault
cargo build -p callora-settlement  
cargo build -p callora-revenue-pool
```

### Release Build (WASM)

```bash
# Build all contracts for deployment
cargo build --target wasm32-unknown-unknown --release

# Build specific contract
cargo build --target wasm32-unknown-unknown --release -p callora-vault
cargo build --target wasm32-unknown-unknown --release -p callora-settlement
cargo build --target wasm32-unknown-unknown --release -p callora-revenue-pool
```

### WASM Output Locations

After release build, WASM binaries are located at:
- `target/wasm32-unknown-unknown/release/callora_vault.wasm`
- `target/wasm32-unknown-unknown/release/callora_settlement.wasm`
- `target/wasm32-unknown-unknown/release/callora_revenue_pool.wasm`

### Size Verification

Soroban enforces a 64KB limit on contract size. Verify compliance:

```bash
# Automated size check for vault contract
//scripts/check-wasm-size.sh

# Manual verificationls
ls -la target/wasm32-unknown-unknown/release/*.wasm
```

Expected sizes (as of current version):
- Vault contract: ~17.5KB (well under 64KB limit)
- Settlement contract: ~15-20KB
- Revenue pool contract: ~15-20KB

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for specific contract
cargo test -p callora-vault
cargo test -p callora-settlement
cargo test -p callora-revenue-pool

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_deposit_success
```

### Test Coverage

The project enforces **95% minimum line coverage** on all code.

#### Generate Coverage Report

```bash
# Automated coverage with enforcement
//scripts/coverage.sh

# Manual coverage generation
cargo tarpaulin
```

#### Coverage Output

Reports are generated in the `coverage/` directory:
- `coverage/tarpaulin-report.html` - Interactive HTML report
- `coverage/cobertura.xml` - XML format for CI systems

#### Coverage Requirements

- **Minimum threshold**: 95% line coverage
- **Enforcement**: CI fails if coverage drops below threshold
- **Scope**: All contract code in `contracts/*/src/`
- **Exclusions**: Build scripts, generated code

### Test Categories

#### Unit Tests
- Located in `contracts/*/src/test.rs`
- Cover all public functions and edge cases
- Include panic scenarios with `#[sould_panic]`
- Test arithmetic overflow/underflow conditions

#### Integration Tests
- Cross-contract interaction testing
- End-to-end user flow validation
- Event emission verification

#### Fuzz Tests
- Deterministic property-based testing
- Balance invariant validation
- Input validation stress testing

### Auth Matrix

The following auth matrix covers every mutating entrypoint in the audited contracts. Each privileged function now enforces `require_auth()` on the controlling principal before state mutation.

#### contracts/vault/src/lib.rs
- `init` → owner (`owner.require_auth()`) at line 91
- `set_admin` → current admin (`caller.require_auth()` + admin check) at line 267
- `accept_admin` → pending admin (`pending.require_auth()`) at line 280
- `set_authorized_caller` → owner (`meta.owner.require_auth()`) at line 299
- `set_max_deduct` → owner (`meta.owner.require_auth()`) at line 319
- `set_allowed_depositor` → owner (`caller.require_auth()` + `require_owner`) at line 333
- `clear_allowed_depositors` → owner (`caller.require_auth()` + `require_owner`) at line 359
- `pause` → admin or owner (`caller.require_auth()` + `require_admin_or_owner`) at line 384
- `unpause` → admin or owner (`caller.require_auth()` + `require_admin_or_owner`) at line 393
- `deposit` → owner or authorized depositor (`caller.require_auth()` + `is_authorized_depositor`) at line 411
- `deduct` → owner or authorized caller (`caller.require_auth()` + `require_authorized_deduct_caller`) at line 453
- `batch_deduct` → owner or authorized caller (`caller.require_auth()` + `require_authorized_deduct_caller`) at line 494
- `transfer_ownership` → owner (`meta.owner.require_auth()`) at line 544
- `accept_ownership` → pending owner (`pending.require_auth()`) at line 564
- `withdraw` → owner (`meta.owner.require_auth()`) at line 582
- `withdraw_to` → owner (`meta.owner.require_auth()`) at line 609
- `distribute` → admin (`caller.require_auth()` + admin check) at line 632
- `set_revenue_pool` → admin (`caller.require_auth()` + admin check) at line 655
- `set_settlement` → admin (`caller.require_auth()` + admin check) at line 681
- `set_metadata` → owner (`caller.require_auth()` + `require_owner`) at line 713
- `update_metadata` → owner (`caller.require_auth()` + `require_owner`) at line 739
- `add_address` → owner (`caller.require_auth()` + `require_owner`) at line 816
- `clear_all` → owner (`caller.require_auth()` + `require_owner`) at line 834

#### contracts/settlement/src/lib.rs
- `init` → admin (`admin.require_auth()`) at line 74
- `receive_payment` → vault or admin (`caller.require_auth()` + `require_authorized_caller`) at line 116
- `set_admin` → current admin (`caller.require_auth()` + admin check) at line 301
- `accept_admin` → pending admin (`pending.require_auth()`) at line 337
- `set_vault` → admin (`caller.require_auth()` + admin check) at line 373
- `force_credit_developer` → admin (`caller.require_auth()` + admin check) at line ~453

#### contracts/revenue_pool/src/lib.rs
- `init` → admin (`admin.require_auth()`)
- `set_admin` → current admin (`caller.require_auth()` + admin check)
- `accept_admin` / `claim_admin` → pending admin (`caller.require_auth()` + pending check)
- `cancel_admin_transfer` → current admin (`caller.require_auth()` + admin check)
- `set_pause_guardian` → current admin (`caller.require_auth()` + admin check)
- `clear_pause_guardian` → current admin (`caller.require_auth()` + admin check)
- `pause` → current admin or pause guardian (`caller.require_auth()` + admin/guardian check)
- `unpause` → current admin (`caller.require_auth()` + admin check)
- `receive_payment` → admin (`caller.require_auth()` + admin check)
- `set_max_distribute` → admin (`caller.require_auth()` + admin check)
- `distribute` → admin (`caller.require_auth()` + admin check)
- `batch_distribute` → admin (`caller.require_auth()` + admin check)
- `upgrade` → admin (`caller.require_auth()` + admin check)

### Findings
- Fixed missing admin auth enforcement in `contracts/settlement/src/lib.rs::init`.
- Removed duplicate/uncompilable method definitions in `contracts/vault/src/lib.rs` that would have bypassed or corrupted auth handling.
- Added a negative auth regression test for `settlement::init`.
- Verified no audited view-only functions mutate state.
- Added bounded metadata validation in `callora-vault` so offering metadata
  rejects zero-width, bidi-override, and other confusable Unicode input while
  remaining NFC-normalized by policy.

### Key Test Scenarios

#### Vault Contract Tests
- ✅ Initialization with various parameters
- ✅ Deposit authorization (owner/allowed depositor)
- ✅ Deduction with balance validation
- ✅ Batch operations with partial failures
- ✅ Withdrawal permissions and limits
- ✅ Arithmetic overflow/underflow protection
- ✅ Input validation (zero/negative amounts)
- ✅ Event emission verification

#### Settlement Contract Tests
- ✅ Payment processing and routing
- ✅ Developer balance management
- ✅ Pool distribution logic
- ✅ Authorization controls

#### Revenue Pool Tests
- ✅ Revenue distribution mechanisms
- ✅ Admin controls and permissions
- ✅ Balance tracking and validation

## Security Documentation

### Core Security Documents

1. **[SECURITY.md](../SECURITY.md)** - Comprehensive security checklist and audit recommendations
2. **[INVARIANTS.md](../INVARIANTS.md)** - Mathematical invariants and balance guarantees
3. **[EVENT_SCHEMA.md](../EVENT_SCHEMA.md)** - Event definitions for monitoring and indexing
4. **[ACCESS_CONTROL.md](ACCESS_CONTROL.md)** - Role-based access control documentation

### Security Highlights

#### Arithmetic Safety
- All balance operations use `checked_add`/`checked_sub`
- Overflow checks enabled in both dev and release profiles
- Input validation prevents zero/negative amounts
- Maximum balance: `i128::MAX` (≈ 1.7 × 10³⁸ stroops)

#### Access Control
- Owner-based permissions for privileged operations
- Optional allowed depositor for backend automation
- Authorization required for all state-changing operations
- Immediate access revocation capabilities

#### Input Validation
- Amount validation: `amount > 0` for all operations
- Balance sufficiency checks before deductions
- Parameter validation on initialization
- Boundary condition handling

#### Event Logging
- Comprehensive event emission for all state changes
- Structured event schema for indexing
- Full context logging for audit trails

## Expected PR Artifacts for Security-Sensitive Changes

When reviewing pull requests that affect security-sensitive areas, auditors should expect the following artifacts:

### Code Changes
- [ ] **Diff Review**: Clear, focused changes with minimal scope
- [ ] **Test Updates**: New tests covering changed functionality
- [ ] **Documentation Updates**: Updated security docs if applicable

### Testing Evidence
- [ ] **Test Results**: All tests passing with output logs
- [ ] **Coverage Report**: Maintained or improved coverage (≥95%)
- [ ] **Edge Case Testing**: Specific tests for security boundaries

### Security Analysis
- [ ] **Threat Model Updates**: Analysis of new attack vectors
- [ ] **Invariant Verification**: Proof that mathematical invariants hold
- [ ] **Access Control Review**: Verification of permission changes

### Build Artifacts
- [ ] **WASM Size Check**: Verification that contracts remain under 64KB
- [ ] **Release Build Success**: Clean compilation without warnings
- [ ] **Clippy Clean**: No linting warnings with strict settings

### Documentation
- [ ] **Security Checklist Updates**: Updated SECURITY.md if needed
- [ ] **Event Schema Changes**: Updated EVENT_SCHEMA.md for new events
- [ ] **API Documentation**: Updated function 
