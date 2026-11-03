import * as crypto from "node:crypto";
import type { ServiceManifest } from "./release_planner.ts";
import { evaluateReleaseMatrix } from "./release_planner.ts";
import { parseClusterTopologyAndSla } from "./config_loader.ts";

function runNpmEcosystemSuite(): string {
    const manifests: ServiceManifest[] = [];
    const ranges = ["^2.0.0", ">=1.4.0 <3.0.0", "~3.1.0", "^4.0.0"];
    const candidates = ["2.4.1", "1.9.5", "3.1.8", "4.2.0", "5.0.0"];

    for (let i = 0; i < 20; i++) {
        manifests.push({
            serviceName: "edge-worker-" + String(i),
            currentVersion: "1." + String(i % 10) + ".0",
            targetRange: ranges[i % ranges.length],
            candidateVersion: candidates[i % candidates.length]
        });
    }

    const releaseSummary = evaluateReleaseMatrix(manifests);

    const sampleIni = [
        "[timeouts]",
        "readTimeout = 2500ms",
        "writeTimeout = 15s",
        "drainWindow = 3m",
        "edgeCacheTtl = 2h",
        "heartbeatMs = 120000",
        "",
        "[regions]",
        "primary = iad-va-01",
        "secondary = fra-de-02",
        "tertiary = nrt-jp-03"
    ].join("\n");

    const slaMetrics = parseClusterTopologyAndSla(sampleIni);

    const digestPayload = JSON.stringify({ releaseSummary, slaMetrics });
    const sha256 = crypto.createHash("sha256").update(digestPayload).digest("hex").slice(0, 16);

    return JSON.stringify({
        app: "02-npm-ecosystem-suite",
        libraries: ["semver@7.6.3", "ms@2.1.3", "ini@4.1.3"],
        validManifests: releaseSummary.validManifests,
        rangeSatisfied: releaseSummary.rangeSatisfiedCount,
        upgradesRequired: releaseSummary.upgradeRequiredCount,
        highestSemver: releaseSummary.highestVersion,
        totalSlaTimeoutMs: slaMetrics.totalTimeoutMs,
        edgeCacheTtlMs: slaMetrics.cacheTtlMs,
        heartbeatInterval: slaMetrics.heartbeatLabel,
        sha256Digest: sha256
    });
}

console.log(runNpmEcosystemSuite());

