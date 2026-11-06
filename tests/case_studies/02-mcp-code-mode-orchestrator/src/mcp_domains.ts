import type { BillingInvoice, EnterpriseCustomer, SupportTicket } from "./types.ts";

export interface McpCatalogSchema {
    toolName: string;
    domain: string;
    signature: string;
    description: string;
}

export const MCP_TOOL_CATALOG: McpCatalogSchema[] = [
    {
        toolName: "crm.listCustomers",
        domain: "crm",
        signature: "listCustomers(filter?: { tier?: string }): EnterpriseCustomer[]",
        description: "Lists enterprise CRM customer records with ARR and region metadata.",
    },
    {
        toolName: "billing.getInvoicesByCustomer",
        domain: "billing",
        signature: "getInvoicesByCustomer(customerId: string): BillingInvoice[]",
        description: "Fetches billing invoices and SLA breach minutes for a customer.",
    },
    {
        toolName: "support.getTicketsByCustomer",
        domain: "support",
        signature: "getTicketsByCustomer(customerId: string): SupportTicket[]",
        description: "Fetches active Sev-1/2/3 support tickets for a customer.",
    },
    {
        toolName: "flags.isCreditAutoApprovalEnabled",
        domain: "flags",
        signature: "isCreditAutoApprovalEnabled(region: string): boolean",
        description: "Checks whether regional SLA credit auto-approval is active.",
    },
];

export function generateMcpDataset(): {
    customers: EnterpriseCustomer[];
    invoices: BillingInvoice[];
    tickets: SupportTicket[];
    regionAutoApprove: Record<string, boolean>;
} {
    const regions = ["us-east", "eu-central", "ap-northeast", "us-west"];
    const tiers: ("enterprise" | "growth" | "starter")[] = ["enterprise", "growth", "starter"];
    const customers: EnterpriseCustomer[] = [];
    const invoices: BillingInvoice[] = [];
    const tickets: SupportTicket[] = [];

    for (let i = 1; i <= 40; i++) {
        const cid = `CUST-${1000 + i}`;
        const tier = tiers[i % 3];
        const region = regions[i % regions.length];
        const arrCents = tier === "enterprise" ? 12000000 + i * 250000 : 2400000 + i * 50000;

        customers.push({
            customerId: cid,
            orgName: `Acme-Global-${i} Corp`,
            tier,
            arrCents,
            region,
            csmEmail: `csm-${i}@slice-enterprise.internal`,
        });

        for (let j = 1; j <= 3; j++) {
            const breach = i % 4 === 0 && j <= 2 ? 45 + i * 2 : j === 1 ? 10 : 0;
            invoices.push({
                invoiceId: `INV-${cid}-${j}`,
                customerId: cid,
                amountCents: Math.floor(arrCents / 12),
                slaBreachMinutes: breach,
                status: breach > 60 ? "disputed" : "paid",
            });
        }

        for (let k = 1; k <= 3; k++) {
            const sev: 1 | 2 | 3 = i % 4 === 0 && k <= 2 ? 1 : k === 2 ? 2 : 3;
            tickets.push({
                ticketId: `TKT-${cid}-${k}`,
                customerId: cid,
                severity: sev,
                unresolvedHours: sev === 1 ? 18 + i : 4,
                summary: `Edge gateway latency alert #${k} for ${cid}`,
            });
        }
    }

    return {
        customers,
        invoices,
        tickets,
        regionAutoApprove: {
            "us-east": true,
            "eu-central": true,
            "ap-northeast": false,
            "us-west": true,
        },
    };
}

