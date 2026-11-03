import dlv from "dlv";
import { dset } from "dset";
import type { FieldSelectionNode } from "./graphql_ast.ts";

export class DeepDataResolver {
    private readonly backingStore: Record<string, unknown>;

    constructor() {
        this.backingStore = {};
        dset(this.backingStore, "viewer.id", "usr_edge_9001");
        dset(this.backingStore, "viewer.organization.slug", "slice-cloud-enterprise");
        dset(this.backingStore, "viewer.organization.deployments.id", "dep_iad_01");
        dset(this.backingStore, "viewer.organization.deployments.region", "us-east-iad");
        dset(this.backingStore, "viewer.organization.deployments.replicas", 12);
        dset(
            this.backingStore,
            "viewer.organization.deployments.securityAuditTrail.signatureChain.rootCertificate",
            "SHA256:9f86d081884c7d65"
        );
    }

    public executeSelection(root: FieldSelectionNode, queryIndex: number): Record<string, unknown> {
        const responseData: Record<string, unknown> = {};
        // Dynamically update replica count per query using dset
        dset(this.backingStore, "viewer.organization.deployments.replicas", 10 + (queryIndex % 8));

        const project = (node: FieldSelectionNode, pathPrefix: string): void => {
            const fullPath = pathPrefix ? pathPrefix + "." + node.name : node.name;
            if (node.children.length === 0) {
                const resolvedValue = dlv(this.backingStore, fullPath, null);
                dset(responseData, fullPath, resolvedValue);
            } else {
                for (let i = 0; i < node.children.length; i++) {
                    project(node.children[i], fullPath);
                }
            }
        };

        project(root, "");
        return responseData;
    }
}

