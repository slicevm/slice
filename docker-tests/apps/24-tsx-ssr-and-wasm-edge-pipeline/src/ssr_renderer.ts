import { escapeHtml } from "./wasm_scorer.ts";
import type { ScoredTenantRow } from "./types.ts";

export interface VNode {
    tag: string;
    props: Record<string, string | number | boolean | undefined> | null;
    children: Array<VNode | string | number>;
}

export function h(
    tag: string,
    props: Record<string, string | number | boolean | undefined> | null,
    ...children: Array<VNode | Array<VNode | string | number> | string | number>
): VNode {
    const flatChildren: Array<VNode | string | number> = [];
    for (let i = 0; i < children.length; i++) {
        const c = children[i];
        if (Array.isArray(c)) {
            for (let j = 0; j < c.length; j++) {
                flatChildren.push(c[j]);
            }
        } else {
            flatChildren.push(c);
        }
    }
    return { tag, props, children: flatChildren };
}

export function renderVNodeToHtml(node: VNode | string | number): string {
    if (typeof node === "string") {
        return escapeHtml(node);
    }
    if (typeof node === "number") {
        return String(node);
    }
    let attrs = "";
    if (node.props) {
        const keys = Object.keys(node.props);
        for (let i = 0; i < keys.length; i++) {
            const k = keys[i];
            const v = node.props[k];
            if (v === undefined || v === false) continue;
            const attrName = k === "className" ? "class" : k;
            if (v === true) {
                attrs += ` ${attrName}`;
            } else {
                attrs += ` ${attrName}="${escapeHtml(String(v))}"`;
            }
        }
    }
    let inner = "";
    for (let i = 0; i < node.children.length; i++) {
        inner += renderVNodeToHtml(node.children[i]);
    }
    return `<${node.tag}${attrs}>${inner}</${node.tag}>`;
}

export function renderEdgeDashboardSsrHtml(
    sessionTenant: string,
    filterRegion: string,
    cacheTtlWindow: string,
    rows: ScoredTenantRow[],
    totalWasmScoreSum: number
): string {
    const cardNodes = rows.map((row) =>
        h(
            "article",
            {
                className: `tenant-card sla-${row.slaTierLabel.toLowerCase()}`,
                "data-tenant": row.tenantId,
                "data-region": row.region,
                "data-score": row.wasmHealthScore
            },
            h("h2", { className: "tenant-title" }, row.rawBannerTitle || row.tenantId),
            h("span", { className: "badge" }, row.slaTierLabel),
            h("p", { className: "metrics" }, `Memory: ${row.formattedMemory} | Score: ${row.wasmHealthScore}`)
        )
    );

    const rootTree = h(
        "main",
        {
            id: "slice-wasm-ssr-root",
            "data-session": sessionTenant,
            "data-filter-region": filterRegion,
            "data-ttl": cacheTtlWindow
        },
        h(
            "header",
            { className: "edge-header" },
            h("h1", null, "Slice WebAssembly + SSR Edge Telemetry"),
            h("div", { className: "aggregate-score" }, `Total WASM Score: ${totalWasmScoreSum}`)
        ),
        h("section", { className: "tenant-grid" }, cardNodes)
    );

    return "<!doctype html>" + renderVNodeToHtml(rootTree);
}

