// =============================================================================
// REAL-LIFE PRODUCTION MULTI-MODULE GRAPH: MODULE 1 (Schema & Validation Engine)
// =============================================================================

export interface ValidationResult<T> {
    readonly ok: boolean;
    readonly data: T | null;
    readonly errors: string[];
}

export interface ApiEventPayload {
    readonly eventId: string;
    readonly tenantId: string;
    readonly timestampMs: number;
    readonly metrics: number[];
    readonly tags: Record<string, string>;
}

export class SchemaValidator {
    private requiredTagKeys: string[];

    constructor(requiredKeys: string[]) {
        this.requiredTagKeys = requiredKeys;
    }

    public validateEvent(raw: ApiEventPayload): ValidationResult<ApiEventPayload> {
        const errs: string[] = [];
        if (!raw.eventId || raw.eventId.length < 4) {
            errs.push("invalid_event_id");
        }
        if (!raw.tenantId || !raw.tenantId.startsWith("tenant_")) {
            errs.push("invalid_tenant_id");
        }
        if (raw.metrics.length === 0) {
            errs.push("empty_metrics");
        }
        for (let i = 0; i < this.requiredTagKeys.length; i++) {
            const key = this.requiredTagKeys[i];
            if (!raw.tags[key]) {
                errs.push("missing_tag:" + key);
            }
        }
        return {
            ok: errs.length === 0,
            data: errs.length === 0 ? raw : null,
            errors: errs
        };
    }
}

