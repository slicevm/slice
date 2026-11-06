export type BranchOutcomeKind =
    | "COMMITTED"
    | "ROLLED_BACK_SLA_REGRESSION"
    | "ROLLED_BACK_EGRESS_DLP"
    | "ROLLED_BACK_SES_POLLUTION"
    | "ROLLED_BACK_CPU_FUEL_DOS"
    | "ROLLED_BACK_FS_OCAP_VIOLATION";

export interface CandidateBranchSpec {
    branchId: string;
    depth: number;
    parentBranchId: string;
    serviceTarget: string;
    hypothesis: string;
    targetVersion: string;
    timeoutWindow: string;
    maxRetries: number;
    poolSize: number;
    sandboxCode: string;
    attackVector: "NONE" | "EGRESS_DLP_B64" | "EGRESS_DLP_HEX" | "SES_PROTOTYPE" | "FUEL_INFINITE_LOOP" | "FS_SHADOW_ESCAPE";
}

export interface BranchEvaluationRecord {
    branchId: string;
    depth: number;
    parentBranchId: string;
    serviceTarget: string;
    outcome: BranchOutcomeKind;
    violationCode: string;
    predictedP99Ms: number;
    postStateDigest: string;
    rollbackVerified: boolean;
}

export interface TotSreAgentReport {
    caseStudy: string;
    incidentId: string;
    totalBranchesEvaluated: number;
    maxSearchDepth: number;
    committedPath: string[];
    rollbacksExecuted: number;
    dlpExfiltrationsBlocked: number;
    sesPollutionsBlocked: number;
    fuelDosPreempted: number;
    fsEscapesBlocked: number;
    slaRegressionsPruned: number;
    initialP99Ms: number;
    finalP99Ms: number;
    finalClusterDigest: string;
    branchTrace: BranchEvaluationRecord[];
}

