import bytes from "bytes";
import { parse, stringify } from "flatted";

export interface SpanNode {
    spanId: string;
    service: string;
    operation: string;
    durationUs: number;
    memoryFootprint: string;
    memoryBytes: number;
    parent?: SpanNode;
    children: SpanNode[];
}

export class TraceGraphBuilder {
    public buildCircularTrace(traceIndex: number): SpanNode {
        const gatewayBudget = bytes.parse("64KB") as number;
        const authBudget = bytes.parse("32KB") as number;
        const dbBudget = bytes.parse("128KB") as number;

        const root: SpanNode = {
            spanId: "span_root_" + traceIndex,
            service: "edge-ingress",
            operation: "HTTP GET /v2/orders",
            durationUs: 420 + (traceIndex % 15) * 18,
            memoryFootprint: bytes.format(gatewayBudget) as string,
            memoryBytes: gatewayBudget,
            children: [],
        };

        const authChild: SpanNode = {
            spanId: "span_auth_" + traceIndex,
            service: "zero-trust-auth",
            operation: "VerifyCapabilityToken",
            durationUs: 95 + (traceIndex % 7) * 9,
            memoryFootprint: bytes.format(authBudget) as string,
            memoryBytes: authBudget,
            parent: root,
            children: [],
        };

        const dbChild: SpanNode = {
            spanId: "span_db_" + traceIndex,
            service: "ledger-cas-store",
            operation: "ReadRadixSnapshot",
            durationUs: 210 + (traceIndex % 11) * 14,
            memoryFootprint: bytes.format(dbBudget + traceIndex * 1024) as string,
            memoryBytes: dbBudget + traceIndex * 1024,
            parent: root,
            children: [],
        };

        // Introduce parent <-> child circular references that standard JSON.stringify cannot handle
        root.children.push(authChild);
        root.children.push(dbChild);
        return root;
    }

    public roundtripSerialize(root: SpanNode): { wirePayload: string; totalDurationUs: number; totalMemoryBytes: number } {
        const wirePayload = stringify(root);
        const restored = parse(wirePayload) as SpanNode;
        let totalDurationUs = restored.durationUs;
        let totalMemoryBytes = restored.memoryBytes;
        for (let i = 0; i < restored.children.length; i++) {
            const child = restored.children[i];
            if (child.parent === restored) {
                totalDurationUs += child.durationUs;
                totalMemoryBytes += child.memoryBytes;
            }
        }
        return { wirePayload, totalDurationUs, totalMemoryBytes };
    }
}

