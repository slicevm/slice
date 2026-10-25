import { FederatedApiMesh } from "./mesh_aggregator.ts";
import type { MeshQueryRequest } from "./types.ts";

async function runMeshBenchmark(): Promise<void> {
    const guardedSecret = "sk_live_MESH_DB_MASTER_TOKEN_2026";
    const b64Secret = Buffer.from(guardedSecret).toString("base64");

    const mesh = new FederatedApiMesh("http://127.0.0.1:18095", [guardedSecret]);
    const queries: MeshQueryRequest[] = [];

    for (let i = 0; i < 25; i++) {
        const isExfilAttempt = i % 5 === 4;
        const outboundAuditNote = isExfilAttempt
            ? "prompt-injection-exfil:" + (i % 2 === 0 ? guardedSecret : b64Secret)
            : "routine-subgraph-sync-ok-" + i;

        queries.push({
            queryId: "q-" + (500 + i),
            tenantId: "tenant-" + (i % 4),
            accountId: "acct-" + (100 + i),
            outboundAuditNote,
        });
    }

    const summary = await mesh.executeBatch(queries);
    console.log(JSON.stringify(summary));
}

runMeshBenchmark();
