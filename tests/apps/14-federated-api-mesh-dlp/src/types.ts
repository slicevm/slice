export interface MeshQueryRequest {
    queryId: string;
    tenantId: string;
    accountId: string;
    outboundAuditNote: string;
}

export interface FederatedMeshSummary {
    app: string;
    libraries: string[];
    queriesExecuted: number;
    subgraphFetchesCompleted: number;
    dlpExfiltrationsBlocked: number;
    cleanAuditForwards: number;
    totalAggregatedBytes: string;
    meshStateHash: string;
}

