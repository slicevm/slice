export interface FieldSelectionNode {
    name: string;
    alias?: string;
    children: FieldSelectionNode[];
}

export interface CostAnalysisResult {
    maxDepth: number;
    complexityCost: number;
    allowed: boolean;
}

export class GraphqlQueryAnalyzer {
    private readonly maxAllowedDepth: number;
    private readonly maxAllowedCost: number;

    constructor(maxAllowedDepth: number, maxAllowedCost: number) {
        this.maxAllowedDepth = maxAllowedDepth;
        this.maxAllowedCost = maxAllowedCost;
    }

    public buildSyntheticAst(queryVariant: number): FieldSelectionNode {
        const includeDeepAudit = queryVariant % 5 === 0;
        const auditNode: FieldSelectionNode = {
            name: "securityAuditTrail",
            children: includeDeepAudit
                ? [
                      {
                          name: "signatureChain",
                          children: [{ name: "rootCertificate", children: [] }],
                      },
                  ]
                : [],
        };

        return {
            name: "viewer",
            children: [
                { name: "id", children: [] },
                {
                    name: "organization",
                    children: [
                        { name: "slug", children: [] },
                        {
                            name: "deployments",
                            children: [
                                { name: "id", children: [] },
                                { name: "region", children: [] },
                                { name: "replicas", children: [] },
                                auditNode,
                            ],
                        },
                    ],
                },
            ],
        };
    }

    public analyzeCost(root: FieldSelectionNode): CostAnalysisResult {
        const walk = (node: FieldSelectionNode, currentDepth: number): { depth: number; cost: number } => {
            let maxD = currentDepth;
            let totalCost = node.children.length > 0 ? 5 * currentDepth : 1;
            for (let i = 0; i < node.children.length; i++) {
                const sub = walk(node.children[i], currentDepth + 1);
                if (sub.depth > maxD) {
                    maxD = sub.depth;
                }
                totalCost += sub.cost;
            }
            return { depth: maxD, cost: totalCost };
        };

        const stats = walk(root, 1);
        const allowed = stats.depth <= this.maxAllowedDepth && stats.cost <= this.maxAllowedCost;
        return {
            maxDepth: stats.depth,
            complexityCost: stats.cost,
            allowed,
        };
    }
}

