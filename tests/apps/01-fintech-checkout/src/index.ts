import type { CheckoutRequest, CloudRegion, CustomerTier } from "./types.ts";
import { PricingAndRiskEngine } from "./pricing_engine.ts";

function runFintechCheckoutApp(): string {
    const engine = new PricingAndRiskEngine();
    const regions: CloudRegion[] = ["US-EAST", "EU-CENTRAL", "AP-SOUTH"];
    const tiers: CustomerTier[] = ["free", "pro", "enterprise"];

    let approvedCount = 0;
    let aggregateRevenueCents = 0;
    let lastSha256Sig = "";

    for (let i = 0; i < 50; i++) {
        const req: CheckoutRequest = {
            requestId: "req_" + String(1000 + i),
            customer: {
                customerId: "cust_" + String(i),
                tier: tiers[i % 3],
                region: regions[i % 3],
                accountAgeDays: (i * 11) % 365
            },
            items: [
                {
                    sku: "SLICE-MICROVM-PRO",
                    title: "Edge Isolate Compute Cluster",
                    unitPriceCents: 12500 + (i * 100),
                    quantity: (i % 5) + 1,
                    category: "compute"
                },
                {
                    sku: "SLICE-CAS-STORAGE",
                    title: "Zero-Stat BLAKE3 CAS Volume",
                    unitPriceCents: 4500,
                    quantity: 2,
                    category: "storage"
                },
                {
                    sku: "SLICE-H100-BURST",
                    title: "Serverless GPU Tensor Slice",
                    unitPriceCents: 85000,
                    quantity: i % 2,
                    category: "ai-gpu"
                }
            ],
            couponCode: i % 2 === 0 ? "NEXUS2026" : null
        };

        const receipt = engine.processCheckout(req);
        if (receipt.approved) {
            approvedCount += 1;
            aggregateRevenueCents += receipt.finalTotalCents;
        }
        lastSha256Sig = receipt.sha256Signature;
    }

    return JSON.stringify({
        app: "01-fintech-checkout",
        ordersProcessed: 50,
        approvedCount: approvedCount,
        aggregateRevenueCents: aggregateRevenueCents,
        lastSha256Signature: lastSha256Sig
    });
}

console.log(runFintechCheckoutApp());

