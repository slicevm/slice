import crypto from "node:crypto";
import bytes from "bytes";
import dlv from "dlv";
import { dset } from "dset";
import { OutboundEgressDlpFirewall } from "./dlp_firewall.ts";
import type { MeshQueryRequest, FederatedMeshSummary } from "./types.ts";

export class FederatedApiMesh {
    private readonly baseUrl: string;
    private readonly dlpFirewall: OutboundEgressDlpFirewall;

    constructor(baseUrl: string, secrets: string[]) {
        this.baseUrl = baseUrl;
        this.dlpFirewall = new OutboundEgressDlpFirewall(secrets);
    }

    public async executeBatch(queries: MeshQueryRequest[]): Promise<FederatedMeshSummary> {
        let subgraphFetchesCompleted = 0;
        let dlpExfiltrationsBlocked = 0;
        let cleanAuditForwards = 0;
        let rawByteCount = 0;
        const graphFingerprints: string[] = [];

        for (let i = 0; i < queries.length; i++) {
            const q = queries[i];
            const subgraphUrl =
                this.baseUrl +
                "/upstream/subgraph/account?tenant=" +
                encodeURIComponent(q.tenantId) +
                "&account=" +
                encodeURIComponent(q.accountId);

            const res = await fetch(subgraphUrl, {
                method: "GET",
                headers: {
                    accept: "application/json",
                    "x-mesh-query-id": q.queryId,
                },
            });

            if (res.status === 200 && res.ok) {
                subgraphFetchesCompleted++;
            }
            const subgraphData = (await res.json()) as {
                accountId: string;
                tier: string;
                balanceCents: number;
                region: string;
            };

            const federatedDoc: Record<string, unknown> = {};
            dset(federatedDoc, "meta.queryId", q.queryId);
            dset(federatedDoc, "meta.tenantId", q.tenantId);
            dset(federatedDoc, "account.profile.id", subgraphData.accountId);
            dset(federatedDoc, "account.profile.tier", subgraphData.tier);
            dset(federatedDoc, "account.billing.balanceCents", subgraphData.balanceCents);
            dset(federatedDoc, "account.billing.region", subgraphData.region);

            const resolvedTier = String(dlv(federatedDoc, "account.profile.tier", "free"));
            const resolvedBalance = Number(dlv(federatedDoc, "account.billing.balanceCents", 0));

            const auditUrl = this.baseUrl + "/upstream/audit";
            const auditHeaders = JSON.stringify({ "x-tenant": q.tenantId });
            const auditBody = JSON.stringify({
                queryId: q.queryId,
                tier: resolvedTier,
                balance: resolvedBalance,
                note: q.outboundAuditNote,
            });

            if (!this.dlpFirewall.inspectOutboundRequest(auditUrl, auditHeaders, auditBody)) {
                dlpExfiltrationsBlocked++;
                graphFingerprints.push(q.queryId + ":DLP_BLOCKED");
            } else {
                cleanAuditForwards++;
                const serialized = JSON.stringify(federatedDoc);
                rawByteCount += serialized.length;
                graphFingerprints.push(
                    q.queryId + ":" + resolvedTier + ":" + String(resolvedBalance),
                );
            }
        }

        const meshStateHash = crypto
            .createHash("sha256")
            .update(graphFingerprints.join("|"))
            .digest("hex")
            .slice(0, 16);

        return {
            app: "14-federated-api-mesh-dlp",
            libraries: ["bytes@3.1.2", "dlv@1.1.3", "dset@3.1.3"],
            queriesExecuted: queries.length,
            subgraphFetchesCompleted,
            dlpExfiltrationsBlocked,
            cleanAuditForwards,
            totalAggregatedBytes: String(bytes.format(rawByteCount)),
            meshStateHash,
        };
    }
}

