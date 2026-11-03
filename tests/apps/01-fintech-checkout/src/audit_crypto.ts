import * as crypto from "node:crypto";
import type { CheckoutRequest } from "./types.ts";

export function validateCheckoutRequest(req: CheckoutRequest): boolean {
    if (!req.requestId || !req.customer || !req.customer.customerId) {
        return false;
    }
    if (!Array.isArray(req.items) || req.items.length === 0) {
        return false;
    }
    for (let i = 0; i < req.items.length; i++) {
        const item = req.items[i];
        if (item.unitPriceCents < 0 || item.quantity < 0) {
            return false;
        }
    }
    return true;
}

export function signReceiptSha256(
    requestId: string,
    customerId: string,
    finalTotalCents: number,
    riskScore: number
): string {
    const payload = requestId + ":" + customerId + ":" + String(finalTotalCents) + ":" + String(riskScore);
    return crypto.createHash("sha256").update(payload).digest("hex").slice(0, 16);
}

