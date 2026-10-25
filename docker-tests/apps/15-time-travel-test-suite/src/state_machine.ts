import semver from "semver";
import ms from "ms";
import dlv from "dlv";
import { dset } from "dset";
import * as crypto from "node:crypto";
import type { TenantAccountState } from "./types.ts";

export function createBaselineState(tenantId: string): TenantAccountState {
    const obj: any = {
        tenantId,
        planVersion: "2.4.0",
        creditsCents: 25000,
        retentionWindowMs: Number(ms("14d")),
    };
    dset(obj, "quotas.dailyRequests", 1000);
    dset(obj, "quotas.usedRequests", 120);
    return obj as TenantAccountState;
}

export function applyTenantMutation(
    state: TenantAccountState,
    targetVersion: string,
    debitCents: number,
    extraWindow: string,
    addRequests: number
): TenantAccountState {
    const cleanVer = semver.valid(semver.coerce(targetVersion));
    if (!cleanVer || !semver.satisfies(cleanVer, ">=2.0.0 <5.0.0")) {
        throw new Error("Unsupported plan semver: " + targetVersion);
    }
    if (state.creditsCents - debitCents < 0) {
        throw new Error("Insufficient tenant credits");
    }
    const next: any = JSON.parse(JSON.stringify(state));
    next.planVersion = cleanVer;
    next.creditsCents = state.creditsCents - debitCents;
    next.retentionWindowMs = state.retentionWindowMs + Number(ms(extraWindow));

    const prevUsed = Number(dlv(next, "quotas.usedRequests", 0));
    const limit = Number(dlv(next, "quotas.dailyRequests", 1000));
    if (prevUsed + addRequests > limit) {
        throw new Error("Daily request quota exceeded");
    }
    dset(next, "quotas.usedRequests", prevUsed + addRequests);
    return next as TenantAccountState;
}

export function digestState(state: TenantAccountState): string {
    return crypto
        .createHash("sha256")
        .update(
            `${state.tenantId}:${state.planVersion}:${state.creditsCents}:${state.retentionWindowMs}:${state.quotas.usedRequests}/${state.quotas.dailyRequests}`
        )
        .digest("hex")
        .slice(0, 16);
}

