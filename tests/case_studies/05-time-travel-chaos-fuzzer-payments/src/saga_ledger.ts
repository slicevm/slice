import dlv from "dlv";
import { dset } from "dset";
import mitt from "mitt";
import ms from "ms";
import * as crypto from "node:crypto";
import type { ChaosFaultKind, SagaAccountBalances } from "./types.ts";

export interface SagaInstruction {
    sagaId: string;
    idempotencyKey: string;
    amountCents: number;
    feeBps: number;
    faultKind: ChaosFaultKind;
    backoffWindow: string;
}

export function createInitialLedgerState(): {
    balances: SagaAccountBalances;
    processedIdempotencyKeys: Record<string, boolean>;
} {
    const obj: any = {};
    dset(obj, "balances.payerVaultCents", 10_000_000); // $100,000.00
    dset(obj, "balances.escrowHoldCents", 0);
    dset(obj, "balances.merchantSettlementCents", 0);
    dset(obj, "balances.feeTreasuryCents", 0);

    return {
        balances: obj.balances as SagaAccountBalances,
        processedIdempotencyKeys: {},
    };
}

export function computeTotalMoneySupply(b: SagaAccountBalances): number {
    return (
        Number(dlv(b as any, "payerVaultCents", 0)) +
        Number(dlv(b as any, "escrowHoldCents", 0)) +
        Number(dlv(b as any, "merchantSettlementCents", 0)) +
        Number(dlv(b as any, "feeTreasuryCents", 0))
    );
}

export function digestLedgerState(b: SagaAccountBalances, idempCount: number): string {
    return crypto
        .createHash("sha256")
        .update(
            `${b.payerVaultCents}:${b.escrowHoldCents}:${b.merchantSettlementCents}:${b.feeTreasuryCents}|keys=${idempCount}`
        )
        .digest("hex")
        .slice(0, 16);
}

export function generateDeterministicChaosSuite(): SagaInstruction[] {
    const instructions: SagaInstruction[] = [];
    const faultCycle: ChaosFaultKind[] = [
        "NONE_CLEAN_SETTLEMENT",
        "CLEARING_HOUSE_TIMEOUT_ABORT",
        "NONE_CLEAN_SETTLEMENT",
        "DUPLICATE_IDEMPOTENCY_REPLAY",
        "MID_FLIGHT_WAL_CORRUPTION_CRASH",
        "NONE_CLEAN_SETTLEMENT",
        "INSUFFICIENT_LIQUIDITY_OVERDRAFT",
        "NONE_CLEAN_SETTLEMENT",
    ];

    for (let i = 1; i <= 48; i++) {
        const faultKind = faultCycle[(i - 1) % faultCycle.length];
        const prevCleanIdx = Math.max(1, i - 1);
        const idempotencyKey =
            faultKind === "DUPLICATE_IDEMPOTENCY_REPLAY"
                ? `IDEMP-KEY-${1000 + prevCleanIdx}`
                : `IDEMP-KEY-${1000 + i}`;

        const amountCents =
            faultKind === "INSUFFICIENT_LIQUIDITY_OVERDRAFT"
                ? 99_000_000 // $990,000.00 > available vault balance
                : 45_000 + i * 1_250;

        instructions.push({
            sagaId: `SAGA-${2000 + i}`,
            idempotencyKey,
            amountCents,
            feeBps: 150, // 1.50% clearing fee
            faultKind,
            backoffWindow: faultKind === "CLEARING_HOUSE_TIMEOUT_ABORT" ? "45m" : "15m",
        });
    }

    return instructions;
}

export function createSagaEventBus(): {
    emitPhase: (phase: string, sagaId: string) => void;
    getPhaseCount: () => number;
} {
    const emitter = mitt();
    let count = 0;
    emitter.on("phase", () => {
        count += 1;
    });
    return {
        emitPhase(phase: string, sagaId: string) {
            emitter.emit("phase", { phase, sagaId, ts: ms("1s") });
        },
        getPhaseCount() {
            return count;
        },
    };
}

