import { ZeroTrustKeepAliveMeshClient } from "./keepalive_mesh_client.ts";
import type { GatewayHopTask } from "./types.ts";

async function runZeroTrustKeepAliveBenchmark(): Promise<void> {
    const baseUrl = "http://127.0.0.1:18095";
    const masterSecret = "sk_live_N1_KEEPALIVE_MESH_SECRET_2026";
    const b64Secret = Buffer.from(masterSecret).toString("base64");

    const client = new ZeroTrustKeepAliveMeshClient(
        [masterSecret],
        "hmac_key_slice_n1_mesh_2026",
    );

    const tasks: GatewayHopTask[] = [];
    for (let i = 0; i < 30; i++) {
        const slot = i % 6;
        const tenantId = "tenant-" + (i % 4);
        const accountId = "acct-" + (200 + i);

        if (slot === 0) {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "keepalive_subgraph",
                targetUrl:
                    baseUrl +
                    "/upstream/subgraph/account?tenant=" +
                    encodeURIComponent(tenantId) +
                    "&account=" +
                    encodeURIComponent(accountId),
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote: "keepalive-subgraph-fetch-" + i,
                timeoutWindow: "1500ms",
            });
        } else if (slot === 1) {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "redirect_follow_307",
                targetUrl:
                    baseUrl +
                    "/upstream/redirect-307?tenant=" +
                    encodeURIComponent(tenantId) +
                    "&account=" +
                    encodeURIComponent(accountId),
                simulatedRedirectLocation:
                    baseUrl +
                    "/upstream/subgraph/account?tenant=" +
                    encodeURIComponent(tenantId) +
                    "&account=" +
                    encodeURIComponent(accountId),
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote: "hop-by-hop-307-follow-" + i,
                timeoutWindow: "2s",
            });
        } else if (slot === 2) {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "redirect_manual_302",
                targetUrl:
                    baseUrl +
                    "/upstream/redirect-302?account=" +
                    encodeURIComponent(accountId),
                simulatedRedirectLocation:
                    "https://partner-gateway.example.com/upstream/subgraph/account?account=" +
                    encodeURIComponent(accountId),
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote: "manual-302-inspect-" + i,
                timeoutWindow: "1s",
            });
        } else if (slot === 3) {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "redirect_ssrf_metadata",
                targetUrl: baseUrl + "/upstream/redirect-ssrf",
                simulatedRedirectLocation:
                    i % 12 === 3
                        ? "http://169.254.169.254/latest/meta-data/iam/security-credentials/"
                        : "http://[fd00:ec2::254]/latest/user-data/",
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote: "ssrf-redirect-pivot-attempt-" + i,
                timeoutWindow: "2s",
            });
        } else if (slot === 4) {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "redirect_dns_rebind_private",
                targetUrl: baseUrl + "/upstream/redirect-rebind",
                simulatedRedirectLocation:
                    i % 12 === 4
                        ? "http://100.64.0.15:8080/internal/cgnat-admin"
                        : "http://rebind-internal.evil.example.org/redis",
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote: "dns-rebind-private-ip-attempt-" + i,
                timeoutWindow: "2s",
            });
        } else {
            tasks.push({
                taskId: "hop-" + (1000 + i),
                tenantId,
                accountId,
                mode: "redirect_dlp_exfil",
                targetUrl: baseUrl + "/upstream/subgraph/account?account=" + accountId,
                outboundBearerToken: "jwt-edge-tok-" + i,
                sessionCookieVal: "sid-" + (5000 + i),
                auditNote:
                    "exfil-header:" + (i % 12 === 5 ? masterSecret : b64Secret),
                timeoutWindow: "1500ms",
            });
        }
    }

    const summary = await client.executeBatch(tasks);
    console.log(JSON.stringify(summary));
}

runZeroTrustKeepAliveBenchmark();

