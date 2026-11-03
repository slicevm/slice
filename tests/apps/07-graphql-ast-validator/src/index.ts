import * as crypto from "node:crypto";
import { GraphqlQueryAnalyzer } from "./graphql_ast.ts";
import { DeepDataResolver } from "./deep_resolver.ts";

const analyzer = new GraphqlQueryAnalyzer(5, 85);
const resolver = new DeepDataResolver();
const hasher = crypto.createHash("sha256");

let allowedQueries = 0;
let blockedDeepQueries = 0;
let totalComplexityScore = 0;
let totalProjectedBytes = 0;

for (let i = 0; i < 60; i++) {
    const ast = analyzer.buildSyntheticAst(i);
    const analysis = analyzer.analyzeCost(ast);
    totalComplexityScore += analysis.complexityCost;

    if (analysis.allowed) {
        allowedQueries += 1;
        const payload = resolver.executeSelection(ast, i);
        const serialized = JSON.stringify(payload);
        totalProjectedBytes += serialized.length;
        hasher.update(serialized);
    } else {
        blockedDeepQueries += 1;
        hasher.update("BLOCKED:" + analysis.maxDepth + ":" + analysis.complexityCost);
    }
}

const summary = {
    app: "07-graphql-ast-validator",
    libraries: ["dlv@1.1.3", "dset@3.1.3"],
    queriesEvaluated: 60,
    allowedQueries,
    blockedDeepQueries,
    totalComplexityScore,
    totalProjectedBytes,
    schemaSha256: hasher.digest("hex").slice(0, 16),
};

console.log(JSON.stringify(summary));

