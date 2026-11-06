export interface EnterpriseCustomer {
    customerId: string;
    orgName: string;
    tier: "enterprise" | "growth" | "starter";
    arrCents: number;
    region: string;
    csmEmail: string;
}

export interface BillingInvoice {
    invoiceId: string;
    customerId: string;
    amountCents: number;
    slaBreachMinutes: number;
    status: "paid" | "overdue" | "disputed";
}

export interface SupportTicket {
    ticketId: string;
    customerId: string;
    severity: 1 | 2 | 3;
    unresolvedHours: number;
    summary: string;
}

export interface ChurnRemediationPlan {
    customerId: string;
    orgName: string;
    region: string;
    riskScore: number;
    proratedCreditCents: number;
    sev1Count: number;
    recommendedAction: string;
}

export interface AdversarialCodeModeProbe {
    probeId: string;
    description: string;
    codeSnippet: string;
    expectedViolation: string;
    trapped: boolean;
    actualViolation: string;
}

export interface McpCodeModeReport {
    caseStudy: string;
    totalRecordsScanned: number;
    legacySequentialToolCalls: number;
    legacyContextBytes: number;
    legacyEstimatedTokens: number;
    codeModeSandboxPasses: number;
    codeModeContextBytes: number;
    codeModeEstimatedTokens: number;
    tokenReductionPercent: number;
    parityVerifiedBetweenModes: boolean;
    highRiskAccountsIdentified: number;
    totalProratedCreditsCents: number;
    securityProbesBlocked: number;
    remediationPlans: ChurnRemediationPlan[];
    securityAudit: AdversarialCodeModeProbe[];
    reportDigest: string;
}

