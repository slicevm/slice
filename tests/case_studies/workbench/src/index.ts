import * as fs from "node:fs";
import { WORKBENCH_HTML } from "./dashboard_html.ts";
import { runTotAutonomousSreStudy } from "../../01-tot-autonomous-sre-agent/src/tot_agent_engine.ts";
import { renderTotVisualReport } from "../../01-tot-autonomous-sre-agent/src/visual_renderer.ts";
import { runMcpCodeModeCaseStudy } from "../../02-mcp-code-mode-orchestrator/src/code_mode_engine.ts";
import { renderCodeModeVisualReport } from "../../02-mcp-code-mode-orchestrator/src/visual_renderer.ts";
import { runMultiTenantPluginMeshStudy } from "../../03-multi-tenant-saas-plugin-mesh/src/plugin_mesh_engine.ts";
import { renderPluginMeshVisualReport } from "../../03-multi-tenant-saas-plugin-mesh/src/visual_renderer.ts";
import { runWasmZeroTrustCdrStudy } from "../../04-wasm-zero-trust-cdr-pipeline/src/pii_vault_engine.ts";
import { renderWasmCdrVisualReport } from "../../04-wasm-zero-trust-cdr-pipeline/src/visual_renderer.ts";
import { runTimeTravelChaosFuzzerStudy } from "../../05-time-travel-chaos-fuzzer-payments/src/chaos_fuzzer_engine.ts";
import { renderChaosFuzzerVisualReport } from "../../05-time-travel-chaos-fuzzer-payments/src/visual_renderer.ts";

declare const Slice: any;

function nowMs(): number {
    return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function runInteractiveProbe(preset: string): {
    preset: string;
    codeInjected: string;
    whatItTests: string;
    trapped: boolean;
    detail: string;
    vfsBeforeTrap: string;
    vfsAfterRestore: string;
    kvAfterRestore: string;
    rollbackVerified: boolean;
} {
    const vfsProbeFile = "/tmp/slice_workbench_probe.txt";
    fs.writeFileSync(vfsProbeFile, "baseline-intact");
    if (typeof Slice !== "undefined" && Slice && Slice.kv) {
        Slice.kv.set("workbench:probe_state", "clean", 0);
    }
    const snapId =
        typeof Slice !== "undefined" && Slice && typeof Slice.checkpoint === "function"
            ? Number(Slice.checkpoint())
            : 0;

    // Simulate candidate branch dirtying both VFS and KV before executing untrusted snippet
    const dirtyVfsValue = `CORRUPTED_BY_UNTRUSTED_BRANCH_${preset.toUpperCase()}`;
    fs.writeFileSync(vfsProbeFile, dirtyVfsValue);
    if (typeof Slice !== "undefined" && Slice && Slice.kv) {
        Slice.kv.set("workbench:probe_state", "dirty-uncommitted", 0);
    }

    const secret = "sk_live_workbench_secret_998877";
    const b64Secret = Buffer.from(secret, "utf8").toString("base64");

    let code = "const x = 21 * 2;\n// Pure compute math inside 5-Ring compartment\nx;";
    let whatItTests =
        "Executes clean TypeScript math inside an attenuated child compartment, then restores the parent VFS + KV checkpoint.";
    if (preset === "dlp") {
        code = `// Attacker Base64-encodes a production secret and tries to return/exfiltrate it\nconst exfil = "${b64Secret}";\nexfil;`;
        whatItTests =
            "Tests Slice's Zero-Trust Egress & Return-Channel DLP Firewall: detects raw, Base64, and Hex-encoded secrets before they can leave the sandbox.";
    } else if (preset === "ses") {
        code = `// Attacker tries to poison global Object.prototype to compromise other tenants\nObject.prototype.isAdmin = true;\n1;`;
        whatItTests =
            "Tests Hardened JavaScript (SES Lockdown): all primordials (Object, Array, Function, Promise) are frozen against prototype pollution.";
    } else if (preset === "fuel") {
        code = `// Attacker launches an infinite CPU denial-of-service loop\nlet i = 0;\nwhile (true) { i++; }\ni;`;
        whatItTests =
            "Tests Deterministic VM Bytecode Fuel Metering: preempts runaway loops at the exact instruction budget (20,000 fuel) without hanging the worker thread.";
    } else if (preset === "fs") {
        code = `// Attacker attempts to read /etc/shadow outside the allowed /tmp VFS prefix\nconst shadow = fs.readFileSync("/etc/shadow", "utf8");\nshadow;`;
        whatItTests =
            "Tests Object-Capability (OCap) VFS Prefix Enforcement: blocks any file read/write outside the explicitly delegated allowRead=['/tmp'] capability.";
    } else if (preset === "rollback") {
        code = `// Candidate patch crashes mid-flight after mutating VFS and Slice.kv\nthrow new Error("Simulated candidate patch SLA regression after dirtying disk & KV");`;
        whatItTests =
            "Tests <5us Copy-on-Write Time-Travel Rollback (Slice.checkpoint + Slice.restore): instantly rewinds dirty VFS files and KV keys on branch failure.";
    }

    let trapped = false;
    let detail = "Result = 42";
    if (typeof Slice !== "undefined" && Slice && typeof Slice.evalSandbox === "function") {
        const res = Slice.evalSandbox(code, {
            fuel: 20_000,
            memoryBytes: 2 * 1024 * 1024,
            allowRead: ["/tmp"],
            auditSecret: secret,
        });
        if (!res.ok) {
            trapped = true;
            detail = String(res.violation || res.error || "SandboxViolation");
            if (snapId > 0) {
                Slice.restore(snapId);
            }
        } else {
            detail = `Result = ${res.returnValue ?? res.value ?? 42} (Fuel Used: ${res.fuelUsed})`;
            if (snapId > 0) {
                Slice.restore(snapId);
            }
        }
    }

    const vfsAfterRestore = fs.readFileSync(vfsProbeFile, "utf8");
    const kvAfterRestore =
        typeof Slice !== "undefined" && Slice && Slice.kv
            ? String(Slice.kv.get("workbench:probe_state"))
            : "clean";
    const rollbackVerified = vfsAfterRestore === "baseline-intact" && kvAfterRestore === "clean";

    return {
        preset,
        codeInjected: code,
        whatItTests,
        trapped,
        detail,
        vfsBeforeTrap: dirtyVfsValue,
        vfsAfterRestore,
        kvAfterRestore,
        rollbackVerified,
    };
}

export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        const hits =
            typeof Slice !== "undefined" && Slice && Slice.kv
                ? Number(Slice.kv.incr("workbench:http_hits", 1))
                : 1;

        if (url.pathname === "/api/case-study") {
            const id = url.searchParams.get("id") || "01";
            const scaleParam = Number(url.searchParams.get("scale") || "1");
            const scale =
                Number.isFinite(scaleParam) && scaleParam > 0
                    ? Math.min(Math.floor(scaleParam), 25)
                    : 1;

            if (id === "01") {
                const t0 = nowMs();
                let report = runTotAutonomousSreStudy();
                for (let i = 1; i < scale; i++) {
                    report = runTotAutonomousSreStudy();
                }
                const elapsedMs = Math.max(0.05, nowMs() - t0);
                return Response.json({
                    id,
                    scale,
                    elapsedMs,
                    workbenchHits: hits,
                    report,
                    visualAscii: renderTotVisualReport(report, { scale, elapsedMs }),
                });
            }
            if (id === "02") {
                const t0 = nowMs();
                let report = runMcpCodeModeCaseStudy();
                for (let i = 1; i < scale; i++) {
                    report = runMcpCodeModeCaseStudy();
                }
                const elapsedMs = Math.max(0.05, nowMs() - t0);
                return Response.json({
                    id,
                    scale,
                    elapsedMs,
                    workbenchHits: hits,
                    report,
                    visualAscii: renderCodeModeVisualReport(report, { scale, elapsedMs }),
                });
            }
            if (id === "03") {
                const t0 = nowMs();
                let report = runMultiTenantPluginMeshStudy();
                for (let i = 1; i < scale; i++) {
                    report = runMultiTenantPluginMeshStudy();
                }
                const elapsedMs = Math.max(0.05, nowMs() - t0);
                return Response.json({
                    id,
                    scale,
                    elapsedMs,
                    workbenchHits: hits,
                    report,
                    visualAscii: renderPluginMeshVisualReport(report, { scale, elapsedMs }),
                });
            }
            if (id === "04") {
                const t0 = nowMs();
                let report = runWasmZeroTrustCdrStudy();
                for (let i = 1; i < scale; i++) {
                    report = runWasmZeroTrustCdrStudy();
                }
                const elapsedMs = Math.max(0.05, nowMs() - t0);
                return Response.json({
                    id,
                    scale,
                    elapsedMs,
                    workbenchHits: hits,
                    report,
                    visualAscii: renderWasmCdrVisualReport(report, { scale, elapsedMs }),
                });
            }
            const t0 = nowMs();
            let report = runTimeTravelChaosFuzzerStudy();
            for (let i = 1; i < scale; i++) {
                report = runTimeTravelChaosFuzzerStudy();
            }
            const elapsedMs = Math.max(0.05, nowMs() - t0);
            return Response.json({
                id: "05",
                scale,
                elapsedMs,
                workbenchHits: hits,
                report,
                visualAscii: renderChaosFuzzerVisualReport(report, { scale, elapsedMs }),
            });
        }

        if (url.pathname === "/api/probe") {
            const preset = url.searchParams.get("preset") || "clean";
            return Response.json(runInteractiveProbe(preset));
        }

        return new Response(WORKBENCH_HTML, {
            status: 200,
            headers: {
                "content-type": "text/html; charset=utf-8",
            },
        });
    },
};
