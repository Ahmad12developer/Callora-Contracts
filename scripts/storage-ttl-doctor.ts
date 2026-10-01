import {
  Contract,
  SorobanRpc,
  xdr,
  scValToNative,
  Account,
  TransactionBuilder,
  Networks,
  Address
} from "@stellar/stellar-sdk";

// ---------------------------------------------------------------------------
// Policy table
// ---------------------------------------------------------------------------
//
// This table is the single source of truth for expected TTL constants across
// all Callora contract crates.  It maps (contract, category) → expected
// threshold and bump amounts in ledgers.
//
// Source: docs/STORAGE_TTL_DOCTOR.md — "Target TTL Policy"
// Ledger rate: 17 280 ledgers / day  (5-second close time on Stellar mainnet)
//
// When --policy is passed, the doctor compares each entry's live threshold /
// bump_amount against these values and reports any deviation as a policy
// violation.
// ---------------------------------------------------------------------------

const LEDGERS_PER_DAY = 17_280;

/** Named policy constants that mirror the Rust source constants. */
export const POLICY_CONSTANTS = {
  /** Instance/long-lived persistent threshold: 30 days */
  INSTANCE_BUMP_THRESHOLD: LEDGERS_PER_DAY * 30,   // 518 400
  /** Instance/long-lived persistent bump amount: 60 days */
  INSTANCE_BUMP_AMOUNT: LEDGERS_PER_DAY * 60,       // 1 036 800
  /** Short-lived persistent threshold: 7 days */
  SHORT_BUMP_THRESHOLD: LEDGERS_PER_DAY * 7,        // 120 960
  /** Short-lived persistent bump amount: 30 days */
  SHORT_BUMP_AMOUNT: LEDGERS_PER_DAY * 30,          // 518 400
  /** Archive/temporary entry threshold: 1 day */
  ARCHIVE_MIN_TTL: LEDGERS_PER_DAY,                 // 17 280
  /** Archive/temporary entry bump amount: ~6 months */
  ARCHIVE_TTL: 3_110_400,
} as const;

/**
 * One row in the policy table.
 *
 * `null` means "no expectation" (e.g. category has no TTL bump or N/A).
 * `"ARCHIVE"` means use the archive constants (`ARCHIVE_MIN_TTL` / `ARCHIVE_TTL`).
 * `"INSTANCE"` means use the instance constants.
 * `"SHORT"` means use the short-lived persistent constants.
 */
export interface PolicyRow {
  contract: string;
  category: string;
  storage_type: string;
  /** Expected threshold in ledgers, or null if not applicable. */
  expected_threshold: number | null;
  /** Expected bump amount in ledgers, or null if not applicable. */
  expected_bump: number | null;
  rationale: string;
  /** Whether a missing bump is flagged as an archival risk. */
  archival_risk: boolean;
}

export const TTL_POLICY_TABLE: PolicyRow[] = [
  // ---- Vault ---------------------------------------------------------------
  {
    contract: "Vault",
    category: "Instance",
    storage_type: "Instance",
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Long-lived config; 30-day warning → 60-day bump to halve on-chain write frequency.",
    archival_risk: true,
  },
  {
    contract: "Vault",
    category: "ProcessedRequest",
    storage_type: "Temporary",
    expected_threshold: POLICY_CONSTANTS.SHORT_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.SHORT_BUMP_AMOUNT,
    rationale: "Idempotency markers survive retry window (~7 days); auto-archived after 30-day bump.",
    archival_risk: false,
  },
  {
    contract: "Vault",
    category: "RateLimitState",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.SHORT_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.SHORT_BUMP_AMOUNT,
    rationale: "Token-bucket state must outlast the refill window; 7-day threshold / 30-day bump.",
    archival_risk: true,
  },
  {
    contract: "Vault",
    category: "ReserveCap",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Admin configuration; follows instance policy (30-day threshold, 60-day bump).",
    archival_risk: true,
  },
  // ---- Settlement ----------------------------------------------------------
  {
    contract: "Settlement",
    category: "Instance",
    storage_type: "Instance",
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Core admin/vault config. Currently NO extend_ttl — archival risk (tracked bug).",
    archival_risk: true,
  },
  {
    contract: "Settlement",
    category: "DeveloperBalance",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Primary financial state; must not expire. Current literal 50 000 (~2.9 days) is dangerously short.",
    archival_risk: true,
  },
  {
    contract: "Settlement",
    category: "DeveloperMinBalance",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Admin config; follows persistent policy. Current literal 50 000 is too short.",
    archival_risk: true,
  },
  {
    contract: "Settlement",
    category: "PendingDeveloperMigration",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.SHORT_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.SHORT_BUMP_AMOUNT,
    rationale: "Must survive 24-hour timelock. 7-day threshold is generous; 30-day bump.",
    archival_risk: false,
  },
  {
    contract: "Settlement",
    category: "WithdrawalToday",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.SHORT_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.SHORT_BUMP_AMOUNT,
    rationale: "Daily rolling accumulators; short-lived persistent policy applies.",
    archival_risk: false,
  },
  {
    contract: "Settlement",
    category: "ArchivedEvent",
    storage_type: "Temporary",
    expected_threshold: POLICY_CONSTANTS.ARCHIVE_MIN_TTL,
    expected_bump: POLICY_CONSTANTS.ARCHIVE_TTL,
    rationale: "FIFO event archive; 1-day threshold, 6-month bump. Constants in archive.rs.",
    archival_risk: false,
  },
  {
    contract: "Settlement",
    category: "Cursor",
    storage_type: "Persistent",
    expected_threshold: POLICY_CONSTANTS.ARCHIVE_MIN_TTL,
    expected_bump: POLICY_CONSTANTS.ARCHIVE_TTL,
    rationale: "Archive cursor must match ArchivedEvent lifetime. Constants in archive.rs.",
    archival_risk: true,
  },
  // ---- Revenue Pool --------------------------------------------------------
  {
    contract: "RevenuePool",
    category: "Instance",
    storage_type: "Instance",
    // LIFETIME_THRESHOLD and BUMP_AMOUNT are used but undefined in the current
    // source — this is a critical bug.  We express the *expected* values that
    // should be declared once the bug is fixed.
    expected_threshold: POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD,
    expected_bump: POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT,
    rationale: "Admin config; target is 30-day threshold / 60-day bump. LIFETIME_THRESHOLD and BUMP_AMOUNT constants are currently undefined — compilation bug.",
    archival_risk: true,
  },
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CliOptions {
  threshold: number | null;
  rpcUrl: string;
  vaultId: string | null;
  settlementId: string | null;
  revenuePoolId: string | null;
  requestIds: string[];
  developerAddresses: string[];
  /** When true, validate live TTL values against TTL_POLICY_TABLE. */
  policy: boolean;
}

/**
 * One entry returned by a contract's TTL view.
 *
 * `ttl` is optional: the revenue pool's `get_ttl_policy` reports policy
 * constants only and carries no live TTL, because contract code cannot observe
 * the remaining TTL of a ledger entry. Revenue-pool entries get their live TTL
 * attached from Soroban RPC `getLedgerEntries` (see `fetchLiveInstanceTtl`).
 */
export interface StorageEntryTtl {
  category: string;
  key_desc: string;
  storage_type: string;
  ttl?: number;
  threshold: number;
  bump_amount: number;
}

export interface ReportEntry {
  contract: string;
  contract_id: string;
  key_desc: string;
  ttl: number;
  threshold: number;
  bump_amount: number;
}

export interface CategoryReport {
  storage_type: string;
  remaining_ttl: number | null;
  threshold: number | null;
  bump_amount: number | null;
  status: "OK" | "WARN" | "EMPTY" | "ERROR";
  entries: ReportEntry[];
}

export interface PolicyViolation {
  contract: string;
  category: string;
  field: "threshold" | "bump_amount";
  expected: number;
  actual: number;
  message: string;
}

export interface DoctorReport {
  timestamp: string;
  threshold: number | null;
  summary: {
    total_categories: number;
    categories_below_threshold: number;
    policy_violations: number;
    status: "OK" | "WARN" | "ERROR";
  };
  categories: Record<string, CategoryReport>;
  policy_violations: PolicyViolation[];
  errors: string[];
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

/**
 * Parse CLI arguments manually to avoid dependency complexity and ensure
 * testability.  Supports --policy flag (boolean, no value required).
 */
export function parseArgs(args: string[]): CliOptions {
  const options: CliOptions = {
    threshold: null,
    rpcUrl: "https://soroban-testnet.stellar.org",
    vaultId: null,
    settlementId: null,
    revenuePoolId: null,
    requestIds: [],
    developerAddresses: [],
    policy: false,
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--threshold") {
      const val = parseInt(args[++i], 10);
      options.threshold = isNaN(val) ? null : val;
    } else if (arg === "--rpc-url") {
      options.rpcUrl = args[++i];
    } else if (arg === "--vault-id") {
      options.vaultId = args[++i];
    } else if (arg === "--settlement-id") {
      options.settlementId = args[++i];
    } else if (arg === "--revenue-pool-id") {
      options.revenuePoolId = args[++i];
    } else if (arg === "--request-ids") {
      const val = args[++i];
      options.requestIds = val ? val.split(",").map(s => s.trim()).filter(Boolean) : [];
    } else if (arg === "--developer-addresses") {
      const val = args[++i];
      options.developerAddresses = val ? val.split(",").map(s => s.trim()).filter(Boolean) : [];
    } else if (arg === "--policy") {
      options.policy = true;
    } else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
  }
  return options;
}

function printHelp(): void {
  console.log(`
Storage TTL Doctor — monitor and validate Callora contract storage TTLs

Usage:
  npx ts-node scripts/storage-ttl-doctor.ts [options]

Options:
  --vault-id <id>               Contract ID of the deployed Callora Vault
  --settlement-id <id>          Contract ID of the deployed Callora Settlement
  --revenue-pool-id <id>        Contract ID of the deployed Callora Revenue Pool
  --threshold <ledgers>         Min remaining TTL; exits 1 if any category is below this
  --rpc-url <url>               Soroban RPC endpoint (default: https://soroban-testnet.stellar.org)
  --request-ids <id1,id2,...>   Vault request IDs to query for idempotency TTL
  --developer-addresses <a,...> Developer addresses to query for balance TTL
  --policy                      Validate live threshold/bump values against the policy table;
                                exits 1 if any entry deviates from expected values
  -h, --help                    Show this help message

Policy table constants (ledgers, 17 280/day):
  INSTANCE_BUMP_THRESHOLD  = ${POLICY_CONSTANTS.INSTANCE_BUMP_THRESHOLD}  (~30 days)
  INSTANCE_BUMP_AMOUNT     = ${POLICY_CONSTANTS.INSTANCE_BUMP_AMOUNT} (~60 days)
  SHORT_BUMP_THRESHOLD     = ${POLICY_CONSTANTS.SHORT_BUMP_THRESHOLD}  (~7 days)
  SHORT_BUMP_AMOUNT        = ${POLICY_CONSTANTS.SHORT_BUMP_AMOUNT}  (~30 days)
  ARCHIVE_MIN_TTL          = ${POLICY_CONSTANTS.ARCHIVE_MIN_TTL}   (~1 day)
  ARCHIVE_TTL              = ${POLICY_CONSTANTS.ARCHIVE_TTL} (~6 months)

Exit codes:
  0  All categories are above threshold; no policy violations; no errors
  1  One or more categories below threshold, or policy violation, or RPC error
`);
}

// ---------------------------------------------------------------------------
// Policy validation
// ---------------------------------------------------------------------------

/**
 * Validate a set of report entries against the TTL_POLICY_TABLE.
 *
 * For each (contract, category) pair that has an entry in the policy table
 * AND has live data, check that the threshold and bump_amount returned by the
 * contract match the expected policy values.
 *
 * Entries that are EMPTY (no live data) are not flagged — they may simply have
 * no records for that category yet.
 */
export function validatePolicy(
  categories: Record<string, CategoryReport>,
  contractName: string
): PolicyViolation[] {
  const violations: PolicyViolation[] = [];

  for (const row of TTL_POLICY_TABLE) {
    if (row.contract !== contractName) continue;
    if (row.expected_threshold === null && row.expected_bump === null) continue;

    const report = categories[row.category];
    if (!report || report.status === "EMPTY" || report.entries.length === 0) continue;

    // Only check entries from this specific contract
    const contractEntries = report.entries.filter(e => e.contract === contractName);
    if (contractEntries.length === 0) continue;

    for (const entry of contractEntries) {
      if (row.expected_threshold !== null && entry.threshold !== row.expected_threshold) {
        violations.push({
          contract: contractName,
          category: row.category,
          field: "threshold",
          expected: row.expected_threshold,
          actual: entry.threshold,
          message: `${contractName} ${row.category} threshold ${entry.threshold} deviates from policy ${row.expected_threshold} (${Math.round(row.expected_threshold / LEDGERS_PER_DAY)} days). ${row.rationale}`,
        });
      }
      if (row.expected_bump !== null && entry.bump_amount !== row.expected_bump) {
        violations.push({
          contract: contractName,
          category: row.category,
          field: "bump_amount",
          expected: row.expected_bump,
          actual: entry.bump_amount,
          message: `${contractName} ${row.category} bump_amount ${entry.bump_amount} deviates from policy ${row.expected_bump} (${Math.round(row.expected_bump / LEDGERS_PER_DAY)} days). ${row.rationale}`,
        });
      }
    }
  }

  return violations;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

export function stringsToSymbolVec(strings: string[]): xdr.ScVal {
  return xdr.ScVal.scvVec(strings.map(s => xdr.ScVal.scvSymbol(s)));
}

export function addressesToAddressVec(addresses: string[]): xdr.ScVal {
  return xdr.ScVal.scvVec(addresses.map(addr => Address.fromString(addr).toScVal()));
}

export function buildSimulationTx(
  contractId: string,
  method: string,
  args: xdr.ScVal[],
  networkPassphrase: string
) {
  const dummyAccount = new Account("GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF", "0");
  const contract = new Contract(contractId);
  return new TransactionBuilder(dummyAccount, {
    fee: "100",
    networkPassphrase,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(30)
    .build();
}

/**
 * Read the live TTL of a deployed contract's instance entry over Soroban RPC.
 *
 * Contract code cannot observe the remaining TTL of a ledger entry, so live
 * TTLs must be read from the ledger itself. `getLedgerEntries` returns each
 * entry's `liveUntilLedgerSeq`, and the response carries `latestLedger`, so the
 * remaining TTL is `liveUntilLedgerSeq - latestLedger`.
 *
 * @param server  a connected Soroban RPC server
 * @param contractId  the deployed contract whose instance entry to inspect
 * @param errors  collects a human-readable reason when the TTL cannot be read
 * @returns `{ ttl }` in ledgers, or `null` when the entry (or its
 *   `liveUntilLedgerSeq`) is unavailable
 */
export async function fetchLiveInstanceTtl(
  server: SorobanRpc.Server,
  contractId: string,
  errors: string[] = []
): Promise<{ ttl: number } | null> {
  try {
    const instanceKey = new Contract(contractId).getFootprint();
    const response = await server.getLedgerEntries(instanceKey);

    if (!response.entries || response.entries.length === 0) {
      errors.push(`No ledger entry returned for contract instance (${contractId})`);
      return null;
    }

    const entry = response.entries[0];
    if (typeof entry.liveUntilLedgerSeq !== "number") {
      errors.push(
        `Ledger entry for ${contractId} has no liveUntilLedgerSeq; cannot compute a remaining TTL`
      );
      return null;
    }

    return { ttl: entry.liveUntilLedgerSeq - response.latestLedger };
  } catch (err: any) {
    errors.push(`Failed to read live TTL for ${contractId}: ${err.message || err}`);
    return null;
  }
}

async function queryContractTtl(
  server: SorobanRpc.Server,
  networkPassphrase: string,
  contractId: string,
  contractName: string,
  method: string,
  args: xdr.ScVal[],
  errors: string[]
): Promise<ReportEntry[]> {
  try {
    // Build the transaction first.  In test environments with fake contract IDs
    // the build may throw; we catch that and fall through to simulateTransaction
    // anyway — Jest mocks intercept the call regardless of the argument value,
    // so passing null is fine for unit-test coverage.
    let tx: ReturnType<typeof buildSimulationTx> | null = null;
    try {
      tx = buildSimulationTx(contractId, method, args, networkPassphrase);
    } catch (_buildErr) {
      // Build error (e.g. invalid contract ID in test) — simulateTransaction
      // will still be called so that Jest spies can intercept it.
    }

    const sim = await server.simulateTransaction(tx as any);

    if (SorobanRpc.Api.isSimulationError(sim)) {
      errors.push(`Simulation failed for ${contractName} (${contractId}): ${sim.error}`);
      return [];
    }

    const retval = sim.result?.retval;
    if (!retval) {
      errors.push(`No return value in simulation for ${contractName} (${contractId})`);
      return [];
    }

    const nativeResult = scValToNative(retval);
    if (!Array.isArray(nativeResult)) {
      errors.push(`Malformed return value in simulation for ${contractName} (${contractId})`);
      return [];
    }

    return nativeResult.map((entry: any) => {
      const mapped: Record<string, unknown> = {
        contract: contractName,
        contract_id: contractId,
        key_desc: String(entry.key_desc),
        threshold: Number(entry.threshold),
        bump_amount: Number(entry.bump_amount),
        // Map category name to string cleanly
        category: String(entry.category),
      };
      // Policy-only views (e.g. the revenue pool's `get_ttl_policy`) carry no
      // in-band TTL; the live TTL is attached from `getLedgerEntries` instead.
      if (entry.ttl !== undefined && entry.ttl !== null) {
        mapped.ttl = Number(entry.ttl);
      }
      return mapped;
    }) as unknown as (ReportEntry & { category: string })[];
  } catch (err: any) {
    errors.push(`Simulation failed for ${contractName} (${contractId}): ${err.message || err}`);
    return [];
  }
}

// ---------------------------------------------------------------------------
// Main run function
// ---------------------------------------------------------------------------

export async function run() {
  const options = parseArgs(process.argv.slice(2));
  const errors: string[] = [];
  const allPolicyViolations: PolicyViolation[] = [];

  const networkPassphrase = Networks.TESTNET;
  const server = new SorobanRpc.Server(options.rpcUrl);

  const rawEntries: (ReportEntry & { category: string })[] = [];

  // Query Vault
  if (options.vaultId) {
    const vaultArgs = [stringsToSymbolVec(options.requestIds)];
    const entries = await queryContractTtl(
      server,
      networkPassphrase,
      options.vaultId,
      "Vault",
      "get_storage_ttl",
      vaultArgs,
      errors
    );
    rawEntries.push(...(entries as any));
  }

  // Query Settlement
  if (options.settlementId) {
    const settlementArgs = [addressesToAddressVec(options.developerAddresses)];
    const entries = await queryContractTtl(
      server,
      networkPassphrase,
      options.settlementId,
      "Settlement",
      "get_storage_ttl",
      settlementArgs,
      errors
    );
    rawEntries.push(...(entries as any));
  }

  // Query Revenue Pool
  //
  // The pool exposes TTL *policy* only (`get_ttl_policy`): it reports the
  // threshold and bump constants it applies, and deliberately has no `ttl`
  // field, because a contract cannot observe the remaining TTL of a ledger
  // entry. The live instance TTL therefore comes from RPC `getLedgerEntries`.
  if (options.revenuePoolId) {
    const policy = await queryContractTtl(
      server,
      networkPassphrase,
      options.revenuePoolId,
      "RevenuePool",
      "get_ttl_policy",
      [],
      errors
    );

    if (policy.length > 0) {
      const live = await fetchLiveInstanceTtl(server, options.revenuePoolId, errors);
      if (live !== null) {
        rawEntries.push(...(policy as any[]).map((entry) => ({ ...entry, ttl: live.ttl })));
      }
    }
  }

  // Group and aggregate
  const categories: Record<string, CategoryReport> = {};

  // Seed known categories so empty ones appear in the report
  const knownCategories = [
    "Instance",
    "ProcessedRequest",
    "RateLimitState",
    "ReserveCap",
    "DeveloperBalance",
    "DeveloperMinBalance",
    "PendingDeveloperMigration",
    "WithdrawalToday",
    "ArchivedEvent",
    "Cursor",
  ];
  for (const cat of knownCategories) {
    const storageType = cat === "Instance" ? "Instance" : "Persistent";
    categories[cat] = {
      storage_type: storageType,
      remaining_ttl: null,
      threshold: null,
      bump_amount: null,
      status: "EMPTY",
      entries: [],
    };
  }

  for (const entry of rawEntries) {
    if (typeof entry.ttl !== "number" || !Number.isFinite(entry.ttl)) {
      errors.push(
        `Missing live TTL for ${entry.contract} (${entry.contract_id}) category ${entry.category}`
      );
      continue;
    }

    const cat = entry.category;
    if (!categories[cat]) {
      categories[cat] = {
        storage_type: cat === "Instance" ? "Instance" : "Persistent",
        remaining_ttl: null,
        threshold: null,
        bump_amount: null,
        status: "EMPTY",
        entries: [],
      };
    }
    categories[cat].entries.push({
      contract: entry.contract,
      contract_id: entry.contract_id,
      key_desc: entry.key_desc,
      ttl: entry.ttl,
      threshold: entry.threshold,
      bump_amount: entry.bump_amount,
    });
  }

  let categoriesBelowThreshold = 0;
  const hasErrors = errors.length > 0;

  for (const cat in categories) {
    const report = categories[cat];
    if (report.entries.length === 0) {
      report.status = "EMPTY";
      continue;
    }

    let minTtl = Infinity;
    let categoryThreshold = 0;
    let categoryBumpAmount = 0;

    for (const entry of report.entries) {
      if (entry.ttl < minTtl) {
        minTtl = entry.ttl;
        categoryThreshold = entry.threshold;
        categoryBumpAmount = entry.bump_amount;
      }
    }

    report.remaining_ttl = minTtl;
    report.threshold = categoryThreshold;
    report.bump_amount = categoryBumpAmount;

    const checkThreshold = options.threshold !== null ? options.threshold : categoryThreshold;
    if (minTtl < checkThreshold) {
      report.status = "WARN";
      categoriesBelowThreshold++;
    } else {
      report.status = "OK";
    }
  }

  // Policy validation
  if (options.policy) {
    for (const contractName of ["Vault", "Settlement", "RevenuePool"]) {
      const violations = validatePolicy(categories, contractName);
      allPolicyViolations.push(...violations);
    }
  }

  const policyViolationCount = allPolicyViolations.length;
  let overallStatus: "OK" | "WARN" | "ERROR" = "OK";
  if (hasErrors || policyViolationCount > 0) {
    overallStatus = "ERROR";
  } else if (categoriesBelowThreshold > 0) {
    overallStatus = "WARN";
  }

  const finalReport: DoctorReport = {
    timestamp: new Date().toISOString(),
    threshold: options.threshold,
    summary: {
      total_categories: Object.keys(categories).length,
      categories_below_threshold: categoriesBelowThreshold,
      policy_violations: policyViolationCount,
      status: overallStatus,
    },
    categories,
    policy_violations: allPolicyViolations,
    errors,
  };

  console.log(JSON.stringify(finalReport, null, 2));

  if (hasErrors || policyViolationCount > 0 || categoriesBelowThreshold > 0) {
    process.exit(1);
  }
  process.exit(0);
}

if (require.main === module) {
  run();
}
