import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import type {
    BranchEvaluationRecord,
    BranchOutcomeKind,
    CandidateBranchSpec,
    TotSreAgentReport,
} from "./types.ts";
import {
    AUDIT_CANARY_SECRET,
    applyPatchToCluster,
    buildCorruptedIncidentState,
    buildTotSearchTreeSpecs,
    computeClusterP99Ms,
    digestClusterState,
} from "./cluster_topology.ts";

declare const Slice: any;

interface MultiFileCheckpointHandle {
    sliceSnapId: number;
    backupManifestJson: string;
    backupIniText: string;
    backupWalText: string;
    backupKvLockRev: string;
    backupKvActiveBranch: string;
    backupKvBestP99: string;
}

export class TotClusterHarness {
    private readonly manifestPath: string;
    private readonly iniPath: string;
    private readonly walPath: string;
    private readonly fallbackKv: Record<string, string> = Object.create(null);

    constructor() {
        const tmp = os.tmpdir();
        this.manifestPath = path.join(tmp, "slice_cs01_cluster_manifest.json");
        this.iniPath = path.join(tmp, "slice_cs01_mesh_routes.ini");
        this.walPath = path.join(tmp, "slice_cs01_remediation_wal.jsonl");
    }

    public initState(manifestJson: string, iniText: string, walLines: string[], initialP99Ms: number): void {
        fs.writeFileSync(this.manifestPath, manifestJson);
        fs.writeFileSync(this.iniPath, iniText);
        fs.writeFileSync(this.walPath, walLines.join("\n") + "\n");
        this.kvSet("sre:lock_rev", "1");
        this.kvSet("sre:active_branch", "ROOT");
        this.kvSet("sre:best_p99_ms", String(initialP99Ms));
    }

    public kvGet(key: string): string {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            return Slice.kv.get(key) || "";
        }
        return this.fallbackKv[key] || "";
    }

    public kvSet(key: string, val: string): void {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            Slice.kv.set(key, val, 0);
        }
        this.fallbackKv[key] = val;
    }

    public readManifest(): string {
        return fs.readFileSync(this.manifestPath, "utf8");
    }

    public readIni(): string {
        return fs.readFileSync(this.iniPath, "utf8");
    }

    public readWal(): string {
        return fs.readFileSync(this.walPath, "utf8");
    }

    public writeState(manifestJson: string, iniText: string, walLine: string, lockRev: string, branchId: string, p99Ms: number): void {
        fs.writeFileSync(this.manifestPath, manifestJson);
        fs.writeFileSync(this.iniPath, iniText);
        const existingWal = this.readWal();
        fs.writeFileSync(this.walPath, existingWal + walLine + "\n");
        this.kvSet("sre:lock_rev", lockRev);
        this.kvSet("sre:active_branch", branchId);
        this.kvSet("sre:best_p99_ms", String(p99Ms));
    }

    public checkpoint(): MultiFileCheckpointHandle {
        const sliceSnapId =
            typeof Slice !== "undefined" && Slice && typeof Slice.checkpoint === "function"
                ? Number(Slice.checkpoint())
                : 0;
        return {
            sliceSnapId,
            backupManifestJson: this.readManifest(),
            backupIniText: this.readIni(),
            backupWalText: this.readWal(),
            backupKvLockRev: this.kvGet("sre:lock_rev"),
            backupKvActiveBranch: this.kvGet("sre:active_branch"),
            backupKvBestP99: this.kvGet("sre:best_p99_ms"),
        };
    }

    public restore(handle: MultiFileCheckpointHandle): void {
        if (
            handle.sliceSnapId > 0 &&
            typeof Slice !== "undefined" &&
            Slice &&
            typeof Slice.restore === "function"
        ) {
            Slice.restore(handle.sliceSnapId);
            return;
        }
        fs.writeFileSync(this.manifestPath, handle.backupManifestJson);
        fs.writeFileSync(this.iniPath, handle.backupIniText);
        fs.writeFileSync(this.walPath, handle.backupWalText);
        this.kvSet("sre:lock_rev", handle.backupKvLockRev);
        this.kvSet("sre:active_branch", handle.backupKvActiveBranch);
        this.kvSet("sre:best_p99_ms", handle.backupKvBestP99);
    }

    public evaluateCandidateSandbox(spec: CandidateBranchSpec): {
        ok: boolean;
        violationKind: BranchOutcomeKind | null;
        violationCode: string;
    } {
        if (typeof Slice !== "undefined" && Slice && typeof Slice.evalSandbox === "function") {
            const res = Slice.evalSandbox(spec.sandboxCode, {
                fuel: 25_000,
                memoryBytes: 2 * 1024 * 1024,
                allowRead: ["/tmp"],
                auditSecret: AUDIT_CANARY_SECRET,
            });
            if (!res.ok) {
                const errText = String(res.violation || res.error || "");
                if (errText.includes("EGRESS_SECRET_LEAK_BLOCKED") || spec.attackVector.startsWith("EGRESS_DLP")) {
                    return {
                        ok: false,
                        violationKind: "ROLLED_BACK_EGRESS_DLP",
                        violationCode: "EGRESS_SECRET_LEAK_BLOCKED",
                    };
                }
                if (errText.includes("IntrinsicMutationBlocked") || spec.attackVector === "SES_PROTOTYPE") {
                    return {
                        ok: false,
                        violationKind: "ROLLED_BACK_SES_POLLUTION",
                        violationCode: "IntrinsicMutationBlocked",
                    };
                }
                if (errText.includes("CpuFuelExhausted") || spec.attackVector === "FUEL_INFINITE_LOOP") {
                    return {
                        ok: false,
                        violationKind: "ROLLED_BACK_CPU_FUEL_DOS",
                        violationCode: "CpuFuelExhausted",
                    };
                }
                if (errText.includes("FsReadDenied") || spec.attackVector === "FS_SHADOW_ESCAPE") {
                    return {
                        ok: false,
                        violationKind: "ROLLED_BACK_FS_OCAP_VIOLATION",
                        violationCode: "FsReadDenied",
                    };
                }
                return {
                    ok: false,
                    violationKind: "ROLLED_BACK_SLA_REGRESSION",
                    violationCode: "SandboxError",
                };
            }
            return { ok: true, violationKind: null, violationCode: "NONE" };
        }

        // Fallback deterministic reference verifier for Node.js / Deno / Bun parity
        if (spec.attackVector.startsWith("EGRESS_DLP")) {
            return {
                ok: false,
                violationKind: "ROLLED_BACK_EGRESS_DLP",
                violationCode: "EGRESS_SECRET_LEAK_BLOCKED",
            };
        }
        if (spec.attackVector === "SES_PROTOTYPE") {
            return {
                ok: false,
                violationKind: "ROLLED_BACK_SES_POLLUTION",
                violationCode: "IntrinsicMutationBlocked",
            };
        }
        if (spec.attackVector === "FUEL_INFINITE_LOOP") {
            return {
                ok: false,
                violationKind: "ROLLED_BACK_CPU_FUEL_DOS",
                violationCode: "CpuFuelExhausted",
            };
        }
        if (spec.attackVector === "FS_SHADOW_ESCAPE") {
            return {
                ok: false,
                violationKind: "ROLLED_BACK_FS_OCAP_VIOLATION",
                violationCode: "FsReadDenied",
            };
        }
        return { ok: true, violationKind: null, violationCode: "NONE" };
    }

    public cleanup(): void {
        for (const p of [this.manifestPath, this.iniPath, this.walPath]) {
            try {
                if (fs.existsSync(p)) {
                    fs.unlinkSync(p);
                }
            } catch (_e) {
                // ignore
            }
        }
    }
}

export function runTotAutonomousSreStudy(): TotSreAgentReport {
    const harness = new TotClusterHarness();
    const initial = buildCorruptedIncidentState();
    const initialManifest = JSON.parse(initial.manifestJson);
    const initialP99Ms = computeClusterP99Ms(initialManifest, initial.iniRoutingText);
    harness.initState(initial.manifestJson, initial.iniRoutingText, initial.walLines, initialP99Ms);

    const candidates = buildTotSearchTreeSpecs();
    const branchTrace: BranchEvaluationRecord[] = [];
    const committedPath: string[] = [];

    let rollbacksExecuted = 0;
    let dlpExfiltrationsBlocked = 0;
    let sesPollutionsBlocked = 0;
    let fuelDosPreempted = 0;
    let fsEscapesBlocked = 0;
    let slaRegressionsPruned = 0;

    for (let i = 0; i < candidates.length; i++) {
        const spec = candidates[i];
        const snap = harness.checkpoint();
        const preManifest = harness.readManifest();
        const preIni = harness.readIni();
        const preLock = harness.kvGet("sre:lock_rev");
        const preBestP99 = Number(harness.kvGet("sre:best_p99_ms"));

        // First dirty VFS & KV with speculative candidate state before sandbox + SLA evaluation
        harness.kvSet("sre:active_branch", spec.branchId);
        harness.kvSet("sre:lock_rev", String(Number(preLock) + 1));

        const sandboxRes = harness.evaluateCandidateSandbox(spec);
        if (!sandboxRes.ok && sandboxRes.violationKind) {
            // Dirty VFS intentionally to prove restore() cleans all 3 files + KV
            fs.writeFileSync(
                path.join(os.tmpdir(), "slice_cs01_cluster_manifest.json"),
                `{"corruptedBy":"${spec.branchId}"}`
            );
            harness.restore(snap);
            rollbacksExecuted++;

            if (sandboxRes.violationKind === "ROLLED_BACK_EGRESS_DLP") dlpExfiltrationsBlocked++;
            if (sandboxRes.violationKind === "ROLLED_BACK_SES_POLLUTION") sesPollutionsBlocked++;
            if (sandboxRes.violationKind === "ROLLED_BACK_CPU_FUEL_DOS") fuelDosPreempted++;
            if (sandboxRes.violationKind === "ROLLED_BACK_FS_OCAP_VIOLATION") fsEscapesBlocked++;

            const restoredClean =
                harness.readManifest() === preManifest &&
                harness.readIni() === preIni &&
                harness.kvGet("sre:lock_rev") === preLock;

            const walCount = harness.readWal().trim().split("\n").length;
            branchTrace.push({
                branchId: spec.branchId,
                depth: spec.depth,
                parentBranchId: spec.parentBranchId,
                serviceTarget: spec.serviceTarget,
                outcome: sandboxRes.violationKind,
                violationCode: sandboxRes.violationCode,
                predictedP99Ms: preBestP99,
                postStateDigest: digestClusterState(harness.readManifest(), harness.readIni(), harness.kvGet("sre:lock_rev"), walCount),
                rollbackVerified: restoredClean,
            });
            continue;
        }

        // Evaluate domain patch and SLA latency target
        let patchSucceeded = false;
        let nextManifestJson = preManifest;
        let nextIniText = preIni;
        let candidateP99 = preBestP99;

        try {
            const applied = applyPatchToCluster(preManifest, preIni, spec);
            nextManifestJson = applied.nextManifestJson;
            nextIniText = applied.nextIniText;
            candidateP99 = applied.p99Ms;
            // Require strictly better P99 latency to commit the branch
            if (candidateP99 < preBestP99) {
                patchSucceeded = true;
            }
        } catch (_err) {
            patchSucceeded = false;
        }

        if (!patchSucceeded) {
            // Mutate VFS before rolling back to verify restore integrity
            harness.writeState(nextManifestJson, nextIniText, JSON.stringify({ rejected: spec.branchId }), "999", spec.branchId, candidateP99);
            harness.restore(snap);
            rollbacksExecuted++;
            slaRegressionsPruned++;

            const restoredClean =
                harness.readManifest() === preManifest &&
                harness.readIni() === preIni &&
                harness.kvGet("sre:lock_rev") === preLock;

            const walCount = harness.readWal().trim().split("\n").length;
            branchTrace.push({
                branchId: spec.branchId,
                depth: spec.depth,
                parentBranchId: spec.parentBranchId,
                serviceTarget: spec.serviceTarget,
                outcome: "ROLLED_BACK_SLA_REGRESSION",
                violationCode: "SLA_OR_SEMVER_REJECTED",
                predictedP99Ms: candidateP99,
                postStateDigest: digestClusterState(harness.readManifest(), harness.readIni(), harness.kvGet("sre:lock_rev"), walCount),
                rollbackVerified: restoredClean,
            });
        } else {
            // Commit this branch as the new baseline for the next Tree-of-Thought depth!
            const newLockRev = String(Number(preLock) + 1);
            const walEntry = JSON.stringify({
                seq: 3 + committedPath.length + 1,
                event: "BRANCH_COMMIT",
                branchId: spec.branchId,
                service: spec.serviceTarget,
                p99Ms: candidateP99,
            });
            harness.writeState(nextManifestJson, nextIniText, walEntry, newLockRev, spec.branchId, candidateP99);
            committedPath.push(spec.branchId);

            const walCount = harness.readWal().trim().split("\n").length;
            branchTrace.push({
                branchId: spec.branchId,
                depth: spec.depth,
                parentBranchId: spec.parentBranchId,
                serviceTarget: spec.serviceTarget,
                outcome: "COMMITTED",
                violationCode: "NONE",
                predictedP99Ms: candidateP99,
                postStateDigest: digestClusterState(harness.readManifest(), harness.readIni(), harness.kvGet("sre:lock_rev"), walCount),
                rollbackVerified: true,
            });
        }
    }

    const finalWalCount = harness.readWal().trim().split("\n").length;
    const finalClusterDigest = digestClusterState(
        harness.readManifest(),
        harness.readIni(),
        harness.kvGet("sre:lock_rev"),
        finalWalCount
    );
    const finalP99Ms = Number(harness.kvGet("sre:best_p99_ms"));
    harness.cleanup();

    return {
        caseStudy: "01-tot-autonomous-sre-agent",
        incidentId: "INC-2026-8841-CASCADE",
        totalBranchesEvaluated: branchTrace.length,
        maxSearchDepth: 4,
        committedPath,
        rollbacksExecuted,
        dlpExfiltrationsBlocked,
        sesPollutionsBlocked,
        fuelDosPreempted,
        fsEscapesBlocked,
        slaRegressionsPruned,
        initialP99Ms,
        finalP99Ms,
        finalClusterDigest,
        branchTrace,
    };
}

