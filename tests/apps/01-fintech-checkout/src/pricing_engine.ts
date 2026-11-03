import type { CheckoutRequest, CheckoutReceipt, OrderItem } from "./types.ts";
import { validateCheckoutRequest, signReceiptSha256 } from "./audit_crypto.ts";

export class PricingAndRiskEngine {
    private taxRates: Record<string, number>;
    private tierDiscounts: Record<string, number>;

    constructor() {
        this.taxRates = {
            "US-EAST": 0.0825,
            "EU-CENTRAL": 0.20,
            "AP-SOUTH": 0.12
        };
        this.tierDiscounts = {
            "free": 0.0,
            "pro": 0.08,
            "enterprise": 0.18
        };
    }

    public processCheckout(req: CheckoutRequest): CheckoutReceipt {
        if (!validateCheckoutRequest(req)) {
            throw new Error("Invalid checkout request: " + req.requestId);
        }

        const subtotalCents = req.items
            .map((item: OrderItem) => item.unitPriceCents * item.quantity)
            .reduce((acc: number, curr: number) => acc + curr, 0);

        const baseDiscountRate = this.tierDiscounts[req.customer.tier] || 0.0;
        const couponBonus = req.couponCode === "NEXUS2026" ? 0.05 : 0.0;
        const effectiveRate = Math.min(baseDiscountRate + couponBonus, 0.30);
        const discountCents = Math.round(subtotalCents * effectiveRate);

        const taxableCents = subtotalCents - discountCents;
        const taxRate = this.taxRates[req.customer.region] || 0.10;
        const taxCents = Math.round(taxableCents * taxRate);
        const finalTotalCents = taxableCents + taxCents;

        const totalUnits = req.items
            .map((item: OrderItem) => item.quantity)
            .reduce((a: number, b: number) => a + b, 0);

        const hasAiGpu = req.items.some((item: OrderItem) => item.category === "ai-gpu");
        let riskScore = 10;
        if (finalTotalCents > 500000 && req.customer.accountAgeDays < 30) {
            riskScore += 45;
        }
        if (hasAiGpu && req.customer.tier === "free") {
            riskScore += 35;
        }
        if (totalUnits > 50) {
            riskScore += 15;
        }

        const sig = signReceiptSha256(
            req.requestId,
            req.customer.customerId,
            finalTotalCents,
            riskScore
        );

        return {
            requestId: req.requestId,
            customerId: req.customer.customerId,
            subtotalCents: subtotalCents,
            discountCents: discountCents,
            taxCents: taxCents,
            finalTotalCents: finalTotalCents,
            riskScore: riskScore,
            approved: riskScore < 75,
            lineItemCount: totalUnits,
            sha256Signature: sig
        };
    }
}

