export type ChaosFaultKind =
    | "NONE_CLEAN_SETTLEMENT"
    | "CLEARING_HOUSE_TIMEOUT_ABORT"
    | "DUPLICATE_IDEMPOTENCY_REPLAY"
    | "MID_FLIGHT_WAL_CORRUPTION_CRASH"
    | "INSUFFICIENT_LIQUIDITY_OVERDRAFT";

export interface SagaAccountBalances {
    payerVaultCents: number;
    escrowHoldCents: number;
    merchantSettlementCents: number;
    feeTreasuryCents: number;
}

export interface ChaosSagaOutcome {
    sagaId: string;
    faultInjected: ChaosFaultKind;
    amountCents: number;
    committed: boolean;
    rolledBackViaCheckpoint: boolean;
    idempotentDeduplicated: boolean;
    simulatedBackoffMs: number;
    conservationOfFundsVerified: boolean;
    postSagaDigest: string;
}

export interface ChaosFuzzerPaymentsReport {
    caseStudy: string;
    totalSagaScenarios: number;
    cleanSettlementsCommitted: number;
    duplicateReplaysDeduplicated: number;
    chaosFaultsRolledBack: number;
    totalSimulatedBackoffWindow: string;
    initialTotalSupplyCents: number;
    finalTotalSupplyCents: number;
    zeroCentDriftVerified: boolean;
    finalBalances: SagaAccountBalances;
    sagaSampleTrace: ChaosSagaOutcome[];
    ledgerDigest: string;
}

