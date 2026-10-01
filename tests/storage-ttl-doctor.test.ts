import * as doctor from "../scripts/storage-ttl-doctor";
import { SorobanRpc, xdr, nativeToScVal } from "@stellar/stellar-sdk";

// `Contract` validates the strkey checksum, so fixtures must be real `C...`
// contract IDs rather than placeholders like "CDVAULT".
const VAULT_ID = "CCOCOGLMUZSDPZGJ3DBTWZWCVFLYCECJKS5JTGC6JHK3TO4BAHZP4RYH";
const SETTLEMENT_ID = "CCVRG3ZQFYZDQ2ZJCXDBO2KU2SJ3ATQYG5UKTKWLOLVCMULFZXCWY2LN";
const POOL_ID = "CCIA6RG5CQCUHYNSLMMCPP4FK4TTMGMXC5EOJZRW4DJWYUOATJ36WAI4";

describe("Storage TTL Doctor Utility Tests", () => {
  let mockExit: jest.SpyInstance;
  let mockLog: jest.SpyInstance;
  let mockSimulateTransaction: jest.SpyInstance;
  let mockGetLedgerEntries: jest.SpyInstance;

  beforeEach(() => {
    mockExit = jest.spyOn(process, "exit").mockImplementation(() => {
      throw new Error("process.exit called");
    });
    mockLog = jest.spyOn(console, "log").mockImplementation(() => {});
    mockSimulateTransaction = jest.spyOn(SorobanRpc.Server.prototype, "simulateTransaction");
    mockGetLedgerEntries = jest.spyOn(SorobanRpc.Server.prototype, "getLedgerEntries");
  });

  afterEach(() => {
    mockExit.mockRestore();
    mockLog.mockRestore();
    mockSimulateTransaction.mockRestore();
    mockGetLedgerEntries.mockRestore();
  });

  // 1. CLI argument parsing
  test("CLI argument parsing works with all flags", () => {
    const args = [
      "--threshold", "1000",
      "--rpc-url", "https://localhost:8000",
      "--vault-id", VAULT_ID,
      "--settlement-id", SETTLEMENT_ID,
      "--revenue-pool-id", POOL_ID,
      "--request-ids", "req1,req2",
      "--developer-addresses", "addr1,addr2"
    ];
    const opts = doctor.parseArgs(args);
    expect(opts.threshold).toBe(1000);
    expect(opts.rpcUrl).toBe("https://localhost:8000");
    expect(opts.vaultId).toBe(VAULT_ID);
    expect(opts.settlementId).toBe(SETTLEMENT_ID);
    expect(opts.revenuePoolId).toBe(POOL_ID);
    expect(opts.requestIds).toEqual(["req1", "req2"]);
    expect(opts.developerAddresses).toEqual(["addr1", "addr2"]);
  });

  test("CLI argument parsing falls back to defaults for missing flags", () => {
    const opts = doctor.parseArgs([]);
    expect(opts.threshold).toBeNull();
    expect(opts.rpcUrl).toBe("https://soroban-testnet.stellar.org");
    expect(opts.vaultId).toBeNull();
    expect(opts.settlementId).toBeNull();
    expect(opts.revenuePoolId).toBeNull();
    expect(opts.requestIds).toEqual([]);
    expect(opts.developerAddresses).toEqual([]);
  });

  // Helper to create mock successful simulation results
  function mockSuccessfulSim(entries: doctor.StorageEntryTtl[]) {
    const retvalVal = nativeToScVal(entries);
    return {
      result: {
        retval: retvalVal
      }
    };
  }

  // Helper to mock a `getLedgerEntries` response with a live TTL.
  function mockLedgerEntry(liveUntilLedgerSeq: number, latestLedger: number = 0) {
    return {
      entries: [{ liveUntilLedgerSeq }],
      latestLedger
    };
  }

  // 2. Successful report generation and grouping
  test("Successful report generation aggregates and groups categories correctly", async () => {
    // Set up mock process.argv
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--vault-id", VAULT_ID,
      "--settlement-id", SETTLEMENT_ID,
      "--revenue-pool-id", POOL_ID
    ];

    // Mock simulateTransaction responses for all three contracts
    // Vault returns Instance & ProcessedRequest
    // Settlement returns Instance & DeveloperBalance
    // Pool returns the TTL policy only (no live `ttl` field)
    mockSimulateTransaction
      .mockResolvedValueOnce(mockSuccessfulSim([
        {
          category: "Instance",
          key_desc: "Instance",
          storage_type: "Instance",
          ttl: 500000,
          threshold: 50000,
          bump_amount: 100000
        },
        {
          category: "ProcessedRequest",
          key_desc: "ProcessedRequest",
          storage_type: "Persistent",
          ttl: 80000,
          threshold: 10000,
          bump_amount: 30000
        }
      ]))
      .mockResolvedValueOnce(mockSuccessfulSim([
        {
          category: "Instance",
          key_desc: "Instance",
          storage_type: "Instance",
          ttl: 600000,
          threshold: 50000,
          bump_amount: 100000
        },
        {
          category: "DeveloperBalance",
          key_desc: "DeveloperBalance",
          storage_type: "Persistent",
          ttl: 45000,
          threshold: 50000, // This is below default threshold!
          bump_amount: 50000
        }
      ]))
      .mockResolvedValueOnce(mockSuccessfulSim([
        {
          category: "Instance",
          key_desc: "Instance",
          storage_type: "Instance",
          threshold: 50000,
          bump_amount: 100000
        }
      ]));

    // The revenue pool's live TTL comes from the ledger, not from the contract.
    mockGetLedgerEntries.mockResolvedValueOnce(mockLedgerEntry(520000));

    // We expect it to exit with 1 because DeveloperBalance (ttl=45000) is below its threshold (50000)
    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(1);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;

    expect(reportJson.errors).toHaveLength(0);
    expect(reportJson.summary.categories_below_threshold).toBe(1);
    expect(reportJson.summary.status).toBe("WARN");

    // Check grouping
    expect(reportJson.categories.Instance.status).toBe("OK");
    expect(reportJson.categories.Instance.remaining_ttl).toBe(500000); // min of 500000, 600000, 520000

    expect(reportJson.categories.ProcessedRequest.status).toBe("OK");
    expect(reportJson.categories.ProcessedRequest.remaining_ttl).toBe(80000);

    expect(reportJson.categories.DeveloperBalance.status).toBe("WARN"); // 45000 < 50000
    expect(reportJson.categories.DeveloperBalance.remaining_ttl).toBe(45000);
  });

  // 2b. Revenue pool: policy from the contract, live TTL from the ledger
  test("Revenue pool reads policy from get_ttl_policy and the live TTL via getLedgerEntries", async () => {
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--revenue-pool-id", POOL_ID
    ];

    mockSimulateTransaction.mockResolvedValueOnce(mockSuccessfulSim([
      {
        category: "Instance",
        key_desc: "Instance",
        storage_type: "Instance",
        threshold: 50000,
        bump_amount: 100000
      }
    ]));
    // remaining TTL = liveUntilLedgerSeq - latestLedger = 41000 - 1000 = 40000
    mockGetLedgerEntries.mockResolvedValueOnce(mockLedgerEntry(41000, 1000));

    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(1);

    expect(mockGetLedgerEntries).toHaveBeenCalledTimes(1);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;
    expect(reportJson.errors).toHaveLength(0);
    expect(reportJson.summary.status).toBe("WARN"); // 40000 < 50000
    expect(reportJson.categories.Instance.remaining_ttl).toBe(40000);
    expect(reportJson.categories.Instance.threshold).toBe(50000);
    expect(reportJson.categories.Instance.bump_amount).toBe(100000);
  });

  // 2c. Revenue pool: no live TTL in the ledger entry is an error, not a guess
  test("Revenue pool without liveUntilLedgerSeq is reported as an error", async () => {
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--revenue-pool-id", POOL_ID
    ];

    mockSimulateTransaction.mockResolvedValueOnce(mockSuccessfulSim([
      {
        category: "Instance",
        key_desc: "Instance",
        storage_type: "Instance",
        threshold: 50000,
        bump_amount: 100000
      }
    ]));
    // Entry present but with no liveUntilLedgerSeq → remaining TTL is unknown.
    mockGetLedgerEntries.mockResolvedValueOnce({ entries: [{}], latestLedger: 1000 });

    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(1);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;
    expect(reportJson.summary.status).toBe("ERROR");
    expect(reportJson.errors.some(e => e.includes("liveUntilLedgerSeq"))).toBe(true);
    expect(reportJson.categories.Instance.status).toBe("EMPTY");
  });

  // 3. Threshold handling (custom CLI threshold)
  test("Custom CLI threshold overrides default entry threshold", async () => {
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--vault-id", VAULT_ID,
      "--threshold", "40000" // Lower than the entry threshold of 50000
    ];

    mockSimulateTransaction.mockResolvedValueOnce(mockSuccessfulSim([
      {
        category: "Instance",
        key_desc: "Instance",
        storage_type: "Instance",
        ttl: 45000, // below default (50000), but above custom (40000)
        threshold: 50000,
        bump_amount: 100000
      }
    ]));

    // Should succeed because 45000 > 40000
    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(0);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;
    expect(reportJson.summary.categories_below_threshold).toBe(0);
    expect(reportJson.summary.status).toBe("OK");
    expect(reportJson.categories.Instance.status).toBe("OK");
  });

  // 4. Empty categories
  test("Handles empty categories gracefully", async () => {
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--vault-id", VAULT_ID
    ];

    // Vault returns instance TTL only, processed request is empty
    mockSimulateTransaction.mockResolvedValueOnce(mockSuccessfulSim([
      {
        category: "Instance",
        key_desc: "Instance",
        storage_type: "Instance",
        ttl: 500000,
        threshold: 50000,
        bump_amount: 100000
      }
    ]));

    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(0);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;
    expect(reportJson.categories.ProcessedRequest.status).toBe("EMPTY");
    expect(reportJson.categories.ProcessedRequest.remaining_ttl).toBeNull();
  });

  // 5. Malformed or missing responses
  test("Gracefully handles simulation errors or missing values", async () => {
    process.argv = [
      "node", "scripts/storage-ttl-doctor.ts",
      "--vault-id", VAULT_ID
    ];

    // Mock simulateTransaction returning a simulation error
    mockSimulateTransaction.mockResolvedValueOnce({
      error: "Contract method not found"
    });

    // Should exit with 1 because of the simulation error
    await expect(doctor.run()).rejects.toThrow("process.exit called");
    expect(mockExit).toHaveBeenCalledWith(1);

    const reportJson = JSON.parse(mockLog.mock.calls[0][0]) as doctor.DoctorReport;
    expect(reportJson.errors).toHaveLength(1);
    expect(reportJson.errors[0]).toContain("Simulation failed for Vault");
    expect(reportJson.summary.status).toBe("ERROR");
  });
});
