import semver from "semver";

export interface ServiceManifest {
    readonly serviceName: string;
    readonly currentVersion: string;
    readonly targetRange: string;
    readonly candidateVersion: string;
}

export interface ReleasePlanSummary {
    readonly totalServices: number;
    readonly validManifests: number;
    readonly rangeSatisfiedCount: number;
    readonly upgradeRequiredCount: number;
    readonly highestVersion: string;
}

export function evaluateReleaseMatrix(manifests: ServiceManifest[]): ReleasePlanSummary {
    let validCount = 0;
    let satisfiedCount = 0;
    let upgradeCount = 0;
    let highest = "0.0.0";

    for (let i = 0; i < manifests.length; i++) {
        const m = manifests[i];
        const cleanCurr = semver.valid(semver.coerce(m.currentVersion));
        const cleanCand = semver.valid(m.candidateVersion);

        if (cleanCurr && cleanCand) {
            validCount += 1;
            if (semver.satisfies(cleanCand, m.targetRange)) {
                satisfiedCount += 1;
            }
            if (semver.gt(cleanCand, cleanCurr)) {
                upgradeCount += 1;
            }
            if (semver.gt(cleanCand, highest)) {
                highest = cleanCand;
            }
        }
    }

    return {
        totalServices: manifests.length,
        validManifests: validCount,
        rangeSatisfiedCount: satisfiedCount,
        upgradeRequiredCount: upgradeCount,
        highestVersion: highest
    };
}

