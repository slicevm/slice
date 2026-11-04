import crypto from "node:crypto";
import ms from "ms";
import type { CheckoutPayload, OrderLineItem, ValidatedOrderReceipt } from "./types.ts";

const HMAC_SECRET = "slice-hono-v4-edge-signing-key-2026";

const TAX_RATES_BPS: Record<string, number> = {
    USD: 825,
    EUR: 1900,
    GBP: 2000,
    JPY: 1000
};

export function signTenantToken(tenantId: string, role: string): string {
    const payload = `${tenantId}:${role}:v1`;
    const mac = crypto.createHmac("sha256", HMAC_SECRET).update(payload).digest("hex").slice(0, 24);
    return ` slc_${tenantId}_${role}_${mac}`.trim();
}

export function verifyTenantToken(authHeader: string | undefined, expectedTenant: string): boolean {
    if (!authHeader || !authHeader.startsWith("Bearer ")) {
        return false;
    }
    const token = authHeader.slice(7).trim();
    const parts = token.split("_");
    if (parts.length !== 4 || parts[0] !== "slc") {
        return false;
    }
    const [, tenantId, role, providedMac] = parts;
    if (tenantId !== expectedTenant) {
        return false;
    }
    const expectedToken = signTenantToken(tenantId, role);
    return token === expectedToken;
}

export function validateAndComputeOrder(
    tenantId: string,
    orderId: string,
    sessionId: string,
    payload: CheckoutPayload,
    ttlSpec: string
): ValidatedOrderReceipt {
    if (!payload.customerId || !Array.isArray(payload.items) || payload.items.length === 0) {
        throw new Error("InvalidCheckoutPayload: customerId and non-empty items required");
    }
    const currency = (payload.currency || "USD").toUpperCase();
    const taxRateBps = TAX_RATES_BPS[currency] ?? 825;
    const discountBps = Math.max(0, Math.min(5000, payload.discountBps || 0));

    let subtotalCents = 0;
    for (let i = 0; i < payload.items.length; i++) {
        const item: OrderLineItem = payload.items[i];
        if (item.qty <= 0 || item.unitCents < 0) {
            throw new Error(`InvalidLineItem: ${item.sku}`);
        }
        subtotalCents += item.qty * item.unitCents;
    }

    const discountCents = Math.floor((subtotalCents * discountBps) / 10000);
    const taxableCents = subtotalCents - discountCents;
    const taxCents = Math.floor((taxableCents * taxRateBps) / 10000);
    const totalCents = taxableCents + taxCents;
    const ttlMs = Number(ms(ttlSpec as any)) || 900000;

    const sigInput = `${tenantId}|${orderId}|${payload.customerId}|${currency}|${totalCents}|${ttlMs}`;
    const signature = crypto.createHash("sha256").update(sigInput).digest("hex").slice(0, 16);

    return {
        tenantId,
        orderId,
        customerId: payload.customerId,
        currency,
        sessionId,
        subtotalCents,
        discountCents,
        taxCents,
        totalCents,
        ttlMs,
        signature
    };
}

