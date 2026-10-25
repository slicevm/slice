export class OutboundEgressDlpFirewall {
    private readonly patterns: string[] = [];

    constructor(secrets: string[]) {
        for (let i = 0; i < secrets.length; i++) {
            const raw = secrets[i];
            if (!raw) continue;
            const b64Std = Buffer.from(raw).toString("base64");
            const b64Url = Buffer.from(raw).toString("base64url");
            this.patterns.push(raw);
            if (b64Std) this.patterns.push(b64Std);
            if (b64Url) this.patterns.push(b64Url);
        }
    }

    public inspectOutboundRequest(url: string, headersJson: string, body: string): boolean {
        for (let i = 0; i < this.patterns.length; i++) {
            const pat = this.patterns[i];
            if (
                url.indexOf(pat) !== -1 ||
                headersJson.indexOf(pat) !== -1 ||
                body.indexOf(pat) !== -1
            ) {
                return false;
            }
        }
        return true;
    }
}

