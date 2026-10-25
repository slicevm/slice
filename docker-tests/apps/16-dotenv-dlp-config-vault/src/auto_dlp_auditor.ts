import * as cookie from "cookie";
import bytes from "bytes";
import * as crypto from "node:crypto";
import type { DlpAuditReport } from "./types.ts";
import { loadAndClassifyDotenvVault } from "./dotenv_vault.ts";

export function executeDotenvAutoDlpAudit(): DlpAuditReport {
    const vault = loadAndClassifyDotenvVault();
    let cleanPayloadsApproved = 0;
    let plaintextLeaksBlocked = 0;
    let base64LeaksBlocked = 0;
    let approvedCookieByteCount = 0;
    const auditDigests: string[] = [];

    for (let i = 0; i < 50; i++) {
        const secretEntry = vault.autoDlpSecrets[i % vault.autoDlpSecrets.length];
        let outboundHeaderCookie = "";
        let outboundBody = "";

        if (i % 10 === 3) {
            // Simulate accidental plaintext secret leak in outbound JSON body
            outboundHeaderCookie = cookie.serialize("slice_env", vault.envMap["SERVICE_ENV"], {
                httpOnly: true,
                secure: true,
                path: "/v1/telemetry",
                maxAge: 3600,
            });
            outboundBody = JSON.stringify({
                seq: i,
                region: vault.clusterRegion,
                debugDump: secretEntry.value,
            });
        } else if (i % 10 === 7) {
            // Simulate prompt-injected Base64 / Base64URL secret exfiltration in cookie header
            const encodedSecret = i % 20 === 7 ? secretEntry.b64 : secretEntry.b64Url;
            outboundHeaderCookie = cookie.serialize("slice_trace", encodedSecret, {
                httpOnly: true,
                secure: true,
                path: "/v1/telemetry",
                maxAge: 3600,
            });
            outboundBody = JSON.stringify({
                seq: i,
                region: vault.clusterRegion,
                publicKey: vault.envMap["PUBLIC_KEY_ID"],
            });
        } else {
            // Clean outbound telemetry payload using only public .env keys
            outboundHeaderCookie = cookie.serialize(
                "slice_pub",
                `${vault.envMap["PUBLIC_KEY_ID"]}_${i}`,
                {
                    httpOnly: true,
                    secure: true,
                    sameSite: "strict",
                    path: "/v1/telemetry",
                    maxAge: 3600,
                }
            );
            outboundBody = JSON.stringify({
                seq: i,
                service: vault.envMap["SERVICE_NAME"],
                env: vault.envMap["SERVICE_ENV"],
                region: vault.clusterRegion,
                publishable: vault.envMap["PUBLISHABLE_KEY_WEB"],
                maxBytes: vault.maxPayloadBytes,
            });
        }

        // Inspect both `outboundHeaderCookie` and `outboundBody` against Auto-DLP secrets
        const combined = outboundHeaderCookie + "\n" + outboundBody;
        let leakedPlain = false;
        let leakedEncoded = false;
        for (let s = 0; s < vault.autoDlpSecrets.length; s++) {
            const sec = vault.autoDlpSecrets[s];
            if (combined.indexOf(sec.value) !== -1) {
                leakedPlain = true;
                break;
            }
            if (combined.indexOf(sec.b64) !== -1 || combined.indexOf(sec.b64Url) !== -1) {
                leakedEncoded = true;
                break;
            }
        }

        if (leakedPlain) {
            plaintextLeaksBlocked++;
            auditDigests.push(`BLOCK-RAW:${i}:${secretEntry.key}`);
        } else if (leakedEncoded) {
            base64LeaksBlocked++;
            auditDigests.push(`BLOCK-B64:${i}:${secretEntry.key}`);
        } else {
            cleanPayloadsApproved++;
            approvedCookieByteCount += outboundHeaderCookie.length + outboundBody.length;
            auditDigests.push(`ALLOW:${i}:${outboundHeaderCookie.length}`);
        }
    }

    const vaultHmacSignature = crypto
        .createHmac("sha256", vault.envMap["SIGNING_PRIVATE_KEY"])
        .update(auditDigests.join("|"))
        .digest("hex")
        .slice(0, 16);

    return {
        app: "16-dotenv-dlp-config-vault",
        libraries: ["bytes@3.1.2", "cookie@0.6.0", "ini@4.1.3"],
        dotenvEntriesLoaded: Object.keys(vault.envMap).length,
        publicKeysAllowed: vault.publicKeys.length,
        autoDlpSecretsRegistered: vault.autoDlpSecrets.length,
        payloadsAudited: 50,
        cleanPayloadsApproved,
        plaintextLeaksBlocked,
        base64LeaksBlocked,
        totalApprovedCookieBytes: String(bytes.format(approvedCookieByteCount)),
        vaultHmacSignature,
    };
}

