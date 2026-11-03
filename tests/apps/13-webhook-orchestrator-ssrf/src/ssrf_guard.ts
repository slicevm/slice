export type OutboundVerdict =
    | { allowed: true; hostPort: string }
    | { allowed: false; reason: "SSRF_METADATA" | "PRIVATE_CIDR" | "UNTRUSTED_DOMAIN" };

const METADATA_HOSTS = new Set<string>([
    "169.254.169.254",
    "169.254.170.2",
    "100.100.100.200",
    "fd00:ec2::254",
    "[fd00:ec2::254]",
    "metadata.google.internal",
    "metadata.goog",
]);

export function evaluateOutboundUrlPolicy(
    rawUrl: string,
    allowedHosts: Set<string>,
): OutboundVerdict {
    const parsed = new URL(rawUrl);
    const hostname = parsed.hostname.toLowerCase();
    const port = parsed.port || (parsed.protocol === "https:" ? "443" : "80");
    const hostPort = hostname + ":" + port;

    if (METADATA_HOSTS.has(hostname)) {
        return { allowed: false, reason: "SSRF_METADATA" };
    }

    if (allowedHosts.has(hostPort) || allowedHosts.has(hostname)) {
        return { allowed: true, hostPort };
    }

    const isPrivateOrLoopback =
        hostname === "localhost" ||
        hostname === "0.0.0.0" ||
        hostname.indexOf("127.") === 0 ||
        hostname.indexOf("10.") === 0 ||
        hostname.indexOf("192.168.") === 0 ||
        hostname.indexOf("172.16.") === 0 ||
        hostname.indexOf("172.17.") === 0 ||
        hostname.indexOf("169.254.") === 0;

    if (isPrivateOrLoopback) {
        return { allowed: false, reason: "PRIVATE_CIDR" };
    }

    return { allowed: false, reason: "UNTRUSTED_DOMAIN" };
}

