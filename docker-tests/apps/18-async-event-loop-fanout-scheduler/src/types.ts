export interface ShardSpec {
    shardId: number;
    region: string;
    timeoutBudget: string;
    payloadBudget: string;
    cancelPreAborted: boolean;
}

export interface ShardFanoutOutcome {
    shardId: number;
    region: string;
    status: number;
    recordsProcessed: number;
    abortedBySignal: boolean;
    payloadBytes: number;
    retryAttempts: number;
}

