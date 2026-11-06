import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import * as crypto from "node:crypto";
import ms from "ms";
import type {
    ChaosFuzzerPaymentsReport,
    ChaosSagaOutcome,
    SagaAccountBalances,
} from "./types.ts";
import {
    computeTotalMoneySupply,
    createInitialLedgerState,
    createSagaEventBus,
    digestLedgerState,
    generateDeterministicChaosSuite,
} from "./saga_ledger.ts";

declare const Slice: any;

interface LedgerCheckpointHandle {
    sliceSnapId: number;
    backupLedgerJson: string;
    backupKvEscrow: string;
    backupKvCommittedCount: string;
}

class ChaosLedgerHarness {
    private readonly ledgerPath: string;
    private readonly fallbackKv: Record<string, string> = Object.create(null);

    constructor() {
        this.ledgerPath = path.join(os.tmpdir(), "slice_cs05_saga_ledger.json");
    }

    public init(stateJson: string): void {
        fs.writeFileSync(this.ledgerPath, stateJson);
        this.kvSet("payments:escrow_inflight", "0");
        this.kvSet("payments:committed_sagas", "0");
    }

    public kvGet(k: string): string {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            return Slice.kv.get(k) || "";
        }
        return this.fallbackKv[k] || "";
    }

    public kvSet(k: string, v: string): void {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            Slice.kv.set(k, v, 0);
        }
        this.fallbackKv[k] = v;
    }

    public readLedgerJson(): string {
        return fs.readFileSync(this.ledgerPath, "utf8");
    }

    public writeLedgerJson(json: string): void {
        fs.writeFileSync(this.ledgerPath, json);
    }

    public checkpoint(): LedgerCheckpointHandle {
        const sliceSnapId =
            typeof Slice !== "undefined" && Slice && typeof Slice.checkpoint === "function"
                ? Number(Slice.checkpoint())
                : 0;
        return {
            sliceSnapId,
            backupLedgerJson: this.readLedgerJson(),
            backupKvEscrow: this.kvGet("payments:escrow_inflight"),
            backupKvCommittedCount: this.kvGet("payments:committed_sagas"),
        };
    }

    public restore(h: LedgerCheckpointHandle): void {
        if (
            h.sliceSnapId > 0 &&
            typeof Slice !== "undefined" &&
            Slice &&
            typeof Slice.restore === "function"
        ) {
            Slice.restore(h.sliceSnapId);
            return;
        }
        this.writeLedgerJson(h.backupLedgerJson);
        this.kvSet("payments:escrow_inflight", h.backupKvEscrow);
        this.kvSet("payments:committed_sagas", h.backupKvCommittedCount);
    }

    public cleanup(): void {
        try {
            if (fs.existsSync(this.ledgerPath)) {
                fs.unlinkSync(this.ledgerPath);
            }
        } catch (_e) {
            // ignore
        }
    }
}

export function runTimeTravelChaosFuzzerStudy(): ChaosFuzzerPaymentsReport {
    const harness = new ChaosLedgerHarness();
    const initialState = createInitialLedgerState();
    const initialTotalSupplyCents = computeTotalMoneySupply(initialState.balances);
    harness.init(JSON.stringify(initialState));

    const suite = generateDeterministicChaosSuite();
    const bus = createSagaEventBus();
    const outcomes: ChaosSagaOutcome[] = [];

    let cleanSettlementsCommitted = 0;
    let duplicateReplaysDeduplicated = 0;
    let chaosFaultsRolledBack = 0;
    let totalSimulatedBackoffMs = 0;

    for (let i = 0; i < suite.length; i++) {
        const instr = suite[i];
        const snap = harness.checkpoint();
        const backoffMs = Number(ms(instr.backoffWindow));
        totalSimulatedBackoffMs += backoffMs;

        const currentState: {
            balances: SagaAccountBalances;
            processedIdempotencyKeys: Record<string, boolean>;
        } = JSON.parse(harness.readLedgerJson());

        // Check idempotency key deduplication first
        if (currentState.processedIdempotencyKeys[instr.idempotencyKey]) {
            duplicateReplaysDeduplicated++;
            const supplyOk = computeTotalMoneySupply(currentState.balances) === initialTotalSupplyCents;
            outcomes.push({
                sagaId: instr.sagaId,
                faultInjected: instr.faultKind,
                amountCents: instr.amountCents,
                committed: false,
                rolledBackViaCheckpoint: false,
                idempotentDeduplicated: true,
                simulatedBackoffMs: backoffMs,
                conservationOfFundsVerified: supplyOk,
                postSagaDigest: digestLedgerState(
                    currentState.balances,
                    Object.keys(currentState.processedIdempotencyKeys).length
                ),
            });
            continue;
        }

        // Phase 1: Reserve funds from payerVault into escrowHold (mutating VFS + KV mid-flight!)
        bus.emitPhase("PHASE_1_RESERVE_ESCROW", instr.sagaId);
        currentState.balances.payerVaultCents -= instr.amountCents;
        currentState.balances.escrowHoldCents += instr.amountCents;
        harness.kvSet("payments:escrow_inflight", String(instr.amountCents));
        harness.writeLedgerJson(JSON.stringify(currentState));

        // Phase 2: Evaluate clearing-house chaos faults
        let faultTriggered = false;
        if (currentState.balances.payerVaultCents < 0) {
            faultTriggered = true;
        } else if (instr.faultKind === "CLEARING_HOUSE_TIMEOUT_ABORT") {
            const abortCtrl = new AbortController();
            abortCtrl.abort("CLEARING_HOUSE_SLA_TIMEOUT");
            if (abortCtrl.signal.aborted) {
                faultTriggered = true;
            }
        } else if (instr.faultKind === "MID_FLIGHT_WAL_CORRUPTION_CRASH") {
            // Simulate a mid-flight partial write corruption where money would otherwise vanish!
            harness.writeLedgerJson(`{"corruptedPartialWrite":true,"saga":"${instr.sagaId}"}`);
            harness.kvSet("payments:escrow_inflight", "999999999");
            faultTriggered = true;
        }

        if (faultTriggered) {
            // Instant < 5us VFS + KV checkpoint rollback restores exact pre-saga double-entry balances!
            harness.restore(snap);
            chaosFaultsRolledBack++;

            const restoredState: {
                balances: SagaAccountBalances;
                processedIdempotencyKeys: Record<string, boolean>;
            } = JSON.parse(harness.readLedgerJson());
            const supplyOk =
                computeTotalMoneySupply(restoredState.balances) === initialTotalSupplyCents &&
                harness.kvGet("payments:escrow_inflight") === "0";

            outcomes.push({
                sagaId: instr.sagaId,
                faultInjected: instr.faultKind,
                amountCents: instr.amountCents,
                committed: false,
                rolledBackViaCheckpoint: true,
                idempotentDeduplicated: false,
                simulatedBackoffMs: backoffMs,
                conservationOfFundsVerified: supplyOk,
                postSagaDigest: digestLedgerState(
                    restoredState.balances,
                    Object.keys(restoredState.processedIdempotencyKeys).length
                ),
            });
            continue;
        }

        // Phase 3: Clearing succeeded -> release escrow into Merchant Settlement + Fee Treasury
        bus.emitPhase("PHASE_3_SETTLE_COMMIT", instr.sagaId);
        const feeCents = Math.floor((instr.amountCents * instr.feeBps) / 10000);
        const netMerchantCents = instr.amountCents - feeCents;

        currentState.balances.escrowHoldCents -= instr.amountCents;
        currentState.balances.merchantSettlementCents += netMerchantCents;
        currentState.balances.feeTreasuryCents += feeCents;
        currentState.processedIdempotencyKeys[instr.idempotencyKey] = true;

        const nextCommitted = Number(harness.kvGet("payments:committed_sagas")) + 1;
        harness.kvSet("payments:escrow_inflight", "0");
        harness.kvSet("payments:committed_sagas", String(nextCommitted));
        harness.writeLedgerJson(JSON.stringify(currentState));
        cleanSettlementsCommitted++;

        const supplyOk = computeTotalMoneySupply(currentState.balances) === initialTotalSupplyCents;
        outcomes.push({
            sagaId: instr.sagaId,
            faultInjected: instr.faultKind,
            amountCents: instr.amountCents,
            committed: true,
            rolledBackViaCheckpoint: false,
            idempotentDeduplicated: false,
            simulatedBackoffMs: backoffMs,
            conservationOfFundsVerified: supplyOk,
            postSagaDigest: digestLedgerState(
                currentState.balances,
                Object.keys(currentState.processedIdempotencyKeys).length
            ),
        });
    }

    const finalState: {
        balances: SagaAccountBalances;
        processedIdempotencyKeys: Record<string, boolean>;
    } = JSON.parse(harness.readLedgerJson());
    const finalTotalSupplyCents = computeTotalMoneySupply(finalState.balances);
    const zeroCentDriftVerified =
        finalTotalSupplyCents === initialTotalSupplyCents &&
        finalState.balances.escrowHoldCents === 0 &&
        outcomes.every((o) => o.conservationOfFundsVerified);

    const ledgerDigest = crypto
        .createHash("sha256")
        .update(
            JSON.stringify(finalState.balances) +
                `|committed=${cleanSettlementsCommitted}|rolledBack=${chaosFaultsRolledBack}|dedup=${duplicateReplaysDeduplicated}|phases=${bus.getPhaseCount()}`
        )
        .digest("hex")
        .slice(0, 16);

    harness.cleanup();

    return {
        caseStudy: "05-time-travel-chaos-fuzzer-payments",
        totalSagaScenarios: suite.length,
        cleanSettlementsCommitted,
        duplicateReplaysDeduplicated,
        chaosFaultsRolledBack,
        totalSimulatedBackoffWindow: ms(totalSimulatedBackoffMs, { long: true }),
        initialTotalSupplyCents,
        finalTotalSupplyCents,
        zeroCentDriftVerified,
        finalBalances: finalState.balances,
        sagaSampleTrace: outcomes.slice(0, 12),
        ledgerDigest,
    };
}

