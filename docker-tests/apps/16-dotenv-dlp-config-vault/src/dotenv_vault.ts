import * as ini from "ini";
import bytes from "bytes";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import type { interfaceParsedDotenvVault } from "./types.ts";

const RAW_DOTENV_FILE = `# Slice Phase F4 Zero-Trust .env Vault
SERVICE_NAME=slice-edge-vault
SERVICE_ENV="production"
PUBLIC_KEY_ID="pk_live_public_tenant_9001"
PUBLISHABLE_KEY_WEB="pk_web_analytics_4002"
export STRIPE_SECRET_KEY="sk_live_f4_vault_stripe_998877"
OPENAI_API_TOKEN=tok_openai_f4_vault_665544 # LLM token
DB_MASTER_PASSWORD='pg_vault_password_f4_332211'
SIGNING_PRIVATE_KEY="prv_hmac_f4_key_112233"
`;

const RAW_CLUSTER_INI = `[cluster]
region = eu-central-1
tier = microvm-isolated

[limits]
max_payload = 64KB
cookie_max_age = 3600
`;

export function isSensitiveEnvKey(key: string): boolean {
    const upper = key.trim().toUpperCase();
    if (upper.indexOf("PUBLIC_KEY") !== -1 || upper.indexOf("PUBLISHABLE_KEY") !== -1) {
        return false;
    }
    return (
        upper.indexOf("SECRET") !== -1 ||
        upper.indexOf("TOKEN") !== -1 ||
        upper.indexOf("KEY") !== -1 ||
        upper.indexOf("PASSWORD") !== -1 ||
        upper.indexOf("PRIVATE") !== -1
    );
}

export function loadAndClassifyDotenvVault(): interfaceParsedDotenvVault {
    const tmpEnvPath = path.join(os.tmpdir(), "slice_app16_vault.env");
    fs.writeFileSync(tmpEnvPath, RAW_DOTENV_FILE);
    const diskContent = fs.readFileSync(tmpEnvPath, "utf8");
    try {
        fs.unlinkSync(tmpEnvPath);
    } catch (_e) {
        // ignore
    }

    const envMap: Record<string, string> = Object.create(null);
    const publicKeys: string[] = [];
    const autoDlpSecrets: Array<{ key: string; value: string; b64: string; b64Url: string }> = [];

    const lines = diskContent.split("\n");
    for (let i = 0; i < lines.length; i++) {
        let line = lines[i].trim();
        if (!line || line.charAt(0) === "#") continue;
        if (line.indexOf("export ") === 0) {
            line = line.slice(7).trim();
        }
        const eqIdx = line.indexOf("=");
        if (eqIdx === -1) continue;
        const key = line.slice(0, eqIdx).trim();
        let valPart = line.slice(eqIdx + 1).trim();
        let val = "";
        if (
            (valPart.charAt(0) === "\"" && valPart.charAt(valPart.length - 1) === "\"") ||
            (valPart.charAt(0) === "'" && valPart.charAt(valPart.length - 1) === "'")
        ) {
            val = valPart.slice(1, -1);
        } else {
            const commentIdx = valPart.indexOf(" #");
            val = commentIdx !== -1 ? valPart.slice(0, commentIdx).trim() : valPart;
        }
        envMap[key] = val;
        if (isSensitiveEnvKey(key) && val.length >= 6) {
            autoDlpSecrets.push({
                key,
                value: val,
                b64: Buffer.from(val, "utf8").toString("base64"),
                b64Url: Buffer.from(val, "utf8").toString("base64url"),
            });
        } else {
            publicKeys.push(key);
        }
    }

    const parsedIni: any = ini.parse(RAW_CLUSTER_INI);
    const maxPayloadBytes = Number(bytes.parse(String(parsedIni.limits.max_payload)));
    const clusterRegion = String(parsedIni.cluster.region);

    return {
        envMap,
        publicKeys,
        autoDlpSecrets,
        maxPayloadBytes,
        clusterRegion,
    };
}

